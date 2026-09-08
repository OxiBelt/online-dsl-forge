use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use regex::{Regex, RegexBuilder};

use crate::parser::SourceSpan;
use crate::sema::{BodyAccess, CapabilityMeta};
use crate::value::Value;

use super::{DynamicRegistry, EvalError, RuntimeCallContext};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RuntimePatternSetKind {
  Contains,
  Regex,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RuntimePatternSetConfig {
  pub name: String,
  pub kind: RuntimePatternSetKind,
  pub patterns: Vec<String>,
}

impl RuntimePatternSetConfig {
  pub fn contains(
    name: impl Into<String>,
    patterns: impl IntoIterator<Item = impl Into<String>>,
  ) -> Self {
    Self {
      name: name.into(),
      kind: RuntimePatternSetKind::Contains,
      patterns: patterns.into_iter().map(Into::into).collect(),
    }
  }

  pub fn regex(
    name: impl Into<String>,
    patterns: impl IntoIterator<Item = impl Into<String>>,
  ) -> Self {
    Self {
      name: name.into(),
      kind: RuntimePatternSetKind::Regex,
      patterns: patterns.into_iter().map(Into::into).collect(),
    }
  }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RuntimePatternSetLimits {
  pub max_sets: usize,
  pub max_patterns_per_set: usize,
  pub max_pattern_bytes: usize,
  pub max_total_pattern_bytes: usize,
  pub max_compiled_regex_bytes: usize,
  pub max_total_compiled_regex_bytes: usize,
}

impl Default for RuntimePatternSetLimits {
  fn default() -> Self {
    Self {
      max_sets: 256,
      max_patterns_per_set: 1024,
      max_pattern_bytes: 4096,
      max_total_pattern_bytes: 64 * 1024 * 1024,
      max_compiled_regex_bytes: 256 * 1024,
      max_total_compiled_regex_bytes: 64 * 1024 * 1024,
    }
  }
}

#[derive(Debug, Clone)]
pub struct RuntimePatternSets {
  sets: BTreeMap<String, CompiledRuntimePatternSet>,
}

impl RuntimePatternSets {
  pub fn compile(
    configs: impl IntoIterator<Item = RuntimePatternSetConfig>,
  ) -> Result<Self, RuntimePatternSetError> {
    Self::compile_with_limits(configs, RuntimePatternSetLimits::default())
  }

  pub fn compile_with_limits(
    configs: impl IntoIterator<Item = RuntimePatternSetConfig>,
    limits: RuntimePatternSetLimits,
  ) -> Result<Self, RuntimePatternSetError> {
    let mut admitted = BTreeMap::new();
    let mut total_pattern_bytes = 0usize;
    let mut total_regex_patterns = 0usize;
    for config in configs {
      if admitted.len() >= limits.max_sets {
        return Err(RuntimePatternSetError::new(
          "runtime pattern set limit exceeded",
        ));
      }
      let metrics = validate_config(&config, limits)?;
      if admitted.contains_key(&config.name) {
        return Err(RuntimePatternSetError::new(format!(
          "duplicate runtime pattern set {}",
          config.name
        )));
      }
      total_pattern_bytes = total_pattern_bytes
        .checked_add(metrics.source_bytes)
        .ok_or_else(|| {
          RuntimePatternSetError::new("runtime pattern source byte count overflowed")
        })?;
      if total_pattern_bytes > limits.max_total_pattern_bytes {
        return Err(RuntimePatternSetError::new(
          "runtime pattern sets exceed max_total_pattern_bytes",
        ));
      }
      total_regex_patterns = total_regex_patterns
        .checked_add(metrics.regex_patterns)
        .ok_or_else(|| RuntimePatternSetError::new("runtime regex pattern count overflowed"))?;
      admitted.insert(config.name.clone(), (config, metrics));
    }

    let projected_compiled_regex_bytes = total_regex_patterns
      .checked_mul(limits.max_compiled_regex_bytes)
      .ok_or_else(|| RuntimePatternSetError::new("runtime compiled regex byte count overflowed"))?;
    if projected_compiled_regex_bytes > limits.max_total_compiled_regex_bytes {
      return Err(RuntimePatternSetError::new(
        "runtime pattern sets exceed max_total_compiled_regex_bytes",
      ));
    }

    let mut sets = BTreeMap::new();
    for (name, (config, metrics)) in admitted {
      let compiled = CompiledRuntimePatternSet::compile(
        &config,
        metrics.match_complexity,
        limits.max_compiled_regex_bytes,
      )?;
      sets.insert(name, compiled);
    }
    Ok(Self { sets })
  }

  fn is_match(&self, name: &str, receiver: &Value, span: SourceSpan) -> Result<bool, EvalError> {
    let Some(set) = self.sets.get(name) else {
      return Err(EvalError::new(
        format!("unknown runtime pattern set {name}"),
        span,
      ));
    };
    match receiver {
      Value::String(value) => Ok(set.is_match(value)),
      Value::Array(values) => values.iter().try_fold(false, |matched, value| {
        let Value::String(value) = value else {
          return Err(EvalError::new(
            format!(
              "pattern-set methods require string array items, got {}",
              value.type_name()
            ),
            span,
          ));
        };
        Ok(matched || set.is_match(value))
      }),
      other => Err(EvalError::new(
        format!(
          "pattern-set methods require string or array receiver, got {}",
          other.type_name()
        ),
        span,
      )),
    }
  }

  fn projected_match_work(
    &self,
    name: &str,
    receiver: &Value,
    span: SourceSpan,
  ) -> Result<usize, EvalError> {
    let Some(set) = self.sets.get(name) else {
      return Err(EvalError::new(
        format!("unknown runtime pattern set {name}"),
        span,
      ));
    };
    let mut candidate_bytes = 0usize;
    match receiver {
      Value::String(value) => candidate_bytes = value.len().saturating_add(1),
      Value::Array(values) => {
        for value in values {
          let Value::String(value) = value else {
            return Err(EvalError::new(
              format!(
                "pattern-set methods require string array items, got {}",
                value.type_name()
              ),
              span,
            ));
          };
          candidate_bytes = candidate_bytes
            .checked_add(value.len().saturating_add(1))
            .ok_or_else(|| EvalError::new("pattern-set work counter overflowed", span))?;
        }
      }
      other => {
        return Err(EvalError::new(
          format!(
            "pattern-set methods require string or array receiver, got {}",
            other.type_name()
          ),
          span,
        ));
      }
    }
    candidate_bytes
      .checked_mul(set.match_complexity())
      .ok_or_else(|| EvalError::new("pattern-set work counter overflowed", span))
  }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RuntimePatternSetError {
  message: String,
}

impl RuntimePatternSetError {
  fn new(message: impl Into<String>) -> Self {
    Self {
      message: message.into(),
    }
  }

  pub fn message(&self) -> &str {
    &self.message
  }
}

impl fmt::Display for RuntimePatternSetError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&self.message)
  }
}

impl Error for RuntimePatternSetError {}

#[derive(Debug, Clone)]
enum CompiledRuntimePatternSet {
  Contains {
    patterns: Vec<String>,
    match_complexity: usize,
  },
  Regex {
    patterns: Vec<Regex>,
    match_complexity: usize,
  },
}

impl CompiledRuntimePatternSet {
  fn compile(
    config: &RuntimePatternSetConfig,
    match_complexity: usize,
    max_compiled_regex_bytes: usize,
  ) -> Result<Self, RuntimePatternSetError> {
    match config.kind {
      RuntimePatternSetKind::Contains => Ok(Self::Contains {
        patterns: config.patterns.clone(),
        match_complexity,
      }),
      RuntimePatternSetKind::Regex => {
        let patterns = config
          .patterns
          .iter()
          .map(|pattern| {
            let mut builder = RegexBuilder::new(pattern);
            builder.size_limit(max_compiled_regex_bytes);
            builder.build().map_err(|error| {
              RuntimePatternSetError::new(format!(
                "runtime pattern set {} contains invalid regex pattern: {error}",
                config.name
              ))
            })
          })
          .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::Regex {
          patterns,
          match_complexity,
        })
      }
    }
  }

  fn is_match(&self, text: &str) -> bool {
    match self {
      Self::Contains { patterns, .. } => patterns.iter().any(|pattern| text.contains(pattern)),
      Self::Regex { patterns, .. } => patterns.iter().any(|pattern| pattern.is_match(text)),
    }
  }

  fn match_complexity(&self) -> usize {
    match self {
      Self::Contains {
        match_complexity, ..
      }
      | Self::Regex {
        match_complexity, ..
      } => *match_complexity,
    }
  }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct PatternSetMetrics {
  source_bytes: usize,
  match_complexity: usize,
  regex_patterns: usize,
}

pub fn register_oxirule_pattern_set_methods(
  registry: &mut DynamicRegistry,
  pattern_sets: RuntimePatternSets,
) -> &mut DynamicRegistry {
  let contains_sets = pattern_sets.clone();
  registry.register_method_capability_with_context(
    CapabilityMeta::method("containsAny", 1).with_body_access(BodyAccess::PrefixBytes),
    move |context, receiver, args| {
      evaluate_pattern_set_method(&contains_sets, context, receiver, args)
    },
  );
  registry.register_method_capability_with_context(
    CapabilityMeta::method("matchesAny", 1).with_body_access(BodyAccess::PrefixBytes),
    move |context, receiver, args| {
      evaluate_pattern_set_method(&pattern_sets, context, receiver, args)
    },
  );
  registry
}

pub fn oxirule_pattern_set_registry(pattern_sets: RuntimePatternSets) -> DynamicRegistry {
  let mut registry = DynamicRegistry::new();
  register_oxirule_pattern_set_methods(&mut registry, pattern_sets);
  registry
}

fn validate_config(
  config: &RuntimePatternSetConfig,
  limits: RuntimePatternSetLimits,
) -> Result<PatternSetMetrics, RuntimePatternSetError> {
  if config.name.trim().is_empty() {
    return Err(RuntimePatternSetError::new(
      "runtime pattern set name must not be empty",
    ));
  }
  if config.patterns.len() > limits.max_patterns_per_set {
    return Err(RuntimePatternSetError::new(format!(
      "runtime pattern set {} exceeds max_patterns_per_set",
      config.name
    )));
  }
  let mut source_bytes = 0usize;
  let mut match_complexity = 0usize;
  for pattern in &config.patterns {
    if pattern.len() > limits.max_pattern_bytes {
      return Err(RuntimePatternSetError::new(format!(
        "runtime pattern set {} contains an oversized pattern",
        config.name
      )));
    }
    source_bytes = source_bytes
      .checked_add(pattern.len())
      .ok_or_else(|| RuntimePatternSetError::new("runtime pattern source byte count overflowed"))?;
    match_complexity = match_complexity
      .checked_add(pattern.len())
      .and_then(|total| total.checked_add(1))
      .ok_or_else(|| RuntimePatternSetError::new("runtime pattern complexity overflowed"))?;
  }
  Ok(PatternSetMetrics {
    source_bytes,
    match_complexity,
    regex_patterns: if config.kind == RuntimePatternSetKind::Regex {
      config.patterns.len()
    } else {
      0
    },
  })
}

fn evaluate_pattern_set_method(
  pattern_sets: &RuntimePatternSets,
  context: RuntimeCallContext<'_>,
  receiver: &Value,
  args: &[Value],
) -> Result<Value, EvalError> {
  let span = context.span();
  let pattern_set = expect_pattern_set_name(args, span)?;
  let work = pattern_sets.projected_match_work(pattern_set, receiver, span)?;
  context.charge_work(work)?;
  pattern_sets
    .is_match(pattern_set, receiver, span)
    .map(Value::Bool)
}

fn expect_pattern_set_name(args: &[Value], span: SourceSpan) -> Result<&str, EvalError> {
  match &args[0] {
    Value::String(value) => Ok(value),
    other => Err(EvalError::new(
      format!(
        "pattern-set methods require string pattern-set name, got {}",
        other.type_name()
      ),
      span,
    )),
  }
}
