use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use regex::Regex;

use crate::parser::SourceSpan;
use crate::sema::{BodyAccess, CapabilityMeta};
use crate::value::Value;

use super::{DynamicRegistry, EvalError};

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
}

impl Default for RuntimePatternSetLimits {
  fn default() -> Self {
    Self {
      max_sets: 256,
      max_patterns_per_set: 1024,
      max_pattern_bytes: 4096,
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
    let mut sets = BTreeMap::new();
    for config in configs {
      if sets.len() >= limits.max_sets {
        return Err(RuntimePatternSetError::new(
          "runtime pattern set limit exceeded",
        ));
      }
      validate_config(&config, limits)?;
      if sets.contains_key(&config.name) {
        return Err(RuntimePatternSetError::new(format!(
          "duplicate runtime pattern set {}",
          config.name
        )));
      }
      let compiled = CompiledRuntimePatternSet::compile(&config)?;
      sets.insert(config.name, compiled);
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
  Contains(Vec<String>),
  Regex(Vec<Regex>),
}

impl CompiledRuntimePatternSet {
  fn compile(config: &RuntimePatternSetConfig) -> Result<Self, RuntimePatternSetError> {
    match config.kind {
      RuntimePatternSetKind::Contains => Ok(Self::Contains(config.patterns.clone())),
      RuntimePatternSetKind::Regex => config
        .patterns
        .iter()
        .map(|pattern| {
          Regex::new(pattern).map_err(|error| {
            RuntimePatternSetError::new(format!(
              "runtime pattern set {} contains invalid regex pattern: {error}",
              config.name
            ))
          })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Self::Regex),
    }
  }

  fn is_match(&self, text: &str) -> bool {
    match self {
      Self::Contains(patterns) => patterns.iter().any(|pattern| text.contains(pattern)),
      Self::Regex(patterns) => patterns.iter().any(|pattern| pattern.is_match(text)),
    }
  }
}

pub fn register_oxirule_pattern_set_methods(
  registry: &mut DynamicRegistry,
  pattern_sets: RuntimePatternSets,
) -> &mut DynamicRegistry {
  let contains_sets = pattern_sets.clone();
  registry.register_method_capability_with_context(
    CapabilityMeta::method("containsAny", 1).with_body_access(BodyAccess::PrefixBytes),
    move |context, receiver, args| {
      evaluate_pattern_set_method(&contains_sets, context.span(), receiver, args)
    },
  );
  registry.register_method_capability_with_context(
    CapabilityMeta::method("matchesAny", 1).with_body_access(BodyAccess::PrefixBytes),
    move |context, receiver, args| {
      evaluate_pattern_set_method(&pattern_sets, context.span(), receiver, args)
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
) -> Result<(), RuntimePatternSetError> {
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
  for pattern in &config.patterns {
    if pattern.len() > limits.max_pattern_bytes {
      return Err(RuntimePatternSetError::new(format!(
        "runtime pattern set {} contains an oversized pattern",
        config.name
      )));
    }
  }
  Ok(())
}

fn evaluate_pattern_set_method(
  pattern_sets: &RuntimePatternSets,
  span: SourceSpan,
  receiver: &Value,
  args: &[Value],
) -> Result<Value, EvalError> {
  let pattern_set = expect_pattern_set_name(args, span)?;
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
