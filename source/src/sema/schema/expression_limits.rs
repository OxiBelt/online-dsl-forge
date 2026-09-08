use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::parser::preflight::preflight_ast;
use crate::parser::validation::{is_valid_identifier, validate_ast_syntax};
use crate::parser::{Diagnostic, SourceSpan};

use super::{ExpressionFunction, ExpressionFunctionDiagnostic, RuntimeSchema};

pub(crate) const MAX_EXPRESSION_FUNCTION_BODY_DEPTH: usize = 128;
pub(crate) const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 1024;
const MAX_EXPRESSION_FUNCTION_DIAGNOSTICS: usize = 1024;
const MAX_TOTAL_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
const MAX_DIAGNOSTIC_NAME_BYTES: usize = 256;
pub(crate) const DIAGNOSTIC_BUDGET_EXCEEDED: &str = "expression function diagnostic limit exceeded";

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ExpressionFunctionLimits {
  pub max_functions: usize,
  pub max_total_parameters: usize,
  pub max_total_name_bytes: usize,
  pub max_total_body_nodes: usize,
  pub max_total_body_scalar_bytes: usize,
  pub max_total_call_edges: usize,
  pub max_body_depth: usize,
  pub max_diagnostics: usize,
  pub max_total_diagnostic_bytes: usize,
}

impl ExpressionFunctionLimits {
  pub(crate) fn is_default(value: &Self) -> bool {
    *value == Self::default()
  }
}

impl Default for ExpressionFunctionLimits {
  fn default() -> Self {
    Self {
      max_functions: 1024,
      max_total_parameters: 8192,
      max_total_name_bytes: 1024 * 1024,
      max_total_body_nodes: 65_536,
      max_total_body_scalar_bytes: 16 * 1024 * 1024,
      max_total_call_edges: 65_536,
      max_body_depth: 128,
      max_diagnostics: 1024,
      max_total_diagnostic_bytes: 1024 * 1024,
    }
  }
}

#[derive(Default)]
struct ExpressionFunctionUsage {
  functions: usize,
  parameters: usize,
  name_bytes: usize,
  body_nodes: usize,
  body_scalar_bytes: usize,
  call_edges: usize,
}

impl RuntimeSchema {
  pub fn expression_function_limits(&self) -> ExpressionFunctionLimits {
    self.expression_function_limits
  }

  pub fn set_expression_function_limits(&mut self, limits: ExpressionFunctionLimits) -> &mut Self {
    self.expression_function_limits = limits;
    self
  }

  pub fn with_expression_function_limits(mut self, limits: ExpressionFunctionLimits) -> Self {
    self.set_expression_function_limits(limits);
    self
  }

  pub(super) fn collect_expression_function_params<T>(
    &self,
    scope: super::ExpressionFunctionScope,
    name: &str,
    params: impl IntoIterator<Item = T>,
    span: SourceSpan,
  ) -> Result<Vec<String>, Diagnostic>
  where
    T: Into<String>,
  {
    let limits = self.expression_function_limits;
    let mut functions = 0usize;
    let mut parameters = 0usize;
    let mut name_bytes = 0usize;
    for function in self
      .expression_functions()
      .filter(|function| function.scope != scope || function.name != name)
    {
      checked_charge(
        &mut functions,
        1,
        limits.max_functions,
        "expression function count limit exceeded",
        span,
      )?;
      checked_charge(
        &mut parameters,
        function.params.len(),
        limits.max_total_parameters,
        "expression function parameter limit exceeded",
        span,
      )?;
      let signature_bytes = function
        .params
        .iter()
        .try_fold(function.name.len(), |total, parameter| {
          total.checked_add(parameter.len())
        })
        .ok_or_else(|| Diagnostic::new("expression function name byte counter overflowed", span))?;
      checked_charge(
        &mut name_bytes,
        signature_bytes,
        limits.max_total_name_bytes,
        "expression function name byte limit exceeded",
        span,
      )?;
    }
    checked_charge(
      &mut functions,
      1,
      limits.max_functions,
      "expression function count limit exceeded",
      span,
    )?;
    checked_charge(
      &mut name_bytes,
      name.len(),
      limits.max_total_name_bytes,
      "expression function name byte limit exceeded",
      span,
    )?;
    if !valid_function_identifier(name) {
      return Err(Diagnostic::new(
        format!(
          "function name {} must be a valid identifier",
          diagnostic_name(name)
        ),
        span,
      ));
    }

    let mut collected = Vec::new();
    for parameter in params {
      checked_charge(
        &mut parameters,
        1,
        limits.max_total_parameters,
        "expression function parameter limit exceeded",
        span,
      )?;
      let parameter = parameter.into();
      checked_charge(
        &mut name_bytes,
        parameter.len(),
        limits.max_total_name_bytes,
        "expression function name byte limit exceeded",
        span,
      )?;
      collected
        .try_reserve(1)
        .map_err(|_| Diagnostic::new("expression function parameter allocation failed", span))?;
      collected.push(parameter);
    }
    validate_signature_parts(name, &collected, span)?;
    Ok(collected)
  }

  pub(super) fn admit_expression_function(
    &self,
    candidate: &ExpressionFunction,
  ) -> Result<(), ExpressionFunctionDiagnostic> {
    let existing = self
      .expression_functions()
      .filter(|function| function.scope != candidate.scope || function.name != candidate.name);
    validate_functions(
      existing.chain(std::iter::once(candidate)),
      self.expression_function_limits,
    )
    .map_err(|diagnostic| ExpressionFunctionDiagnostic::new(diagnostic.message, diagnostic.span))
  }

  pub(crate) fn validated_expression_function_diagnostics(&self) -> Vec<Diagnostic> {
    let limits = self.expression_function_limits;
    let diagnostic_limit = expression_function_diagnostic_limit(limits);
    let diagnostic_byte_limit = expression_function_diagnostic_byte_limit(limits);
    let mut diagnostics = Vec::new();
    let mut diagnostic_bytes = 0usize;
    let mut truncated = self.expression_function_diagnostics_truncated;
    for diagnostic in &self.expression_function_diagnostics {
      if diagnostics.len() >= diagnostic_limit {
        truncated = true;
        break;
      }
      let message = bounded_diagnostic_message(&diagnostic.message);
      if !try_charge_bytes(&mut diagnostic_bytes, message.len(), diagnostic_byte_limit) {
        truncated = true;
        break;
      }
      diagnostics.push(Diagnostic::new(message, diagnostic.span));
    }
    if let Err(diagnostic) =
      validate_functions(self.expression_functions(), self.expression_function_limits)
    {
      let message = bounded_diagnostic_message(&diagnostic.message);
      if diagnostics.len() < diagnostic_limit
        && try_charge_bytes(&mut diagnostic_bytes, message.len(), diagnostic_byte_limit)
      {
        diagnostics.push(Diagnostic::new(message, diagnostic.span));
      } else {
        truncated = true;
      }
    }
    if truncated && diagnostics.is_empty() {
      diagnostics.push(Diagnostic::new(
        DIAGNOSTIC_BUDGET_EXCEEDED,
        SourceSpan::default(),
      ));
    }
    diagnostics
  }

  pub(super) fn push_expression_function_diagnostic(
    &mut self,
    diagnostic: ExpressionFunctionDiagnostic,
  ) {
    let limits = self.expression_function_limits;
    let diagnostic_limit = expression_function_diagnostic_limit(limits);
    if self.expression_function_diagnostics.len() >= diagnostic_limit {
      self.expression_function_diagnostics_truncated = true;
      return;
    }
    let diagnostic = ExpressionFunctionDiagnostic::new(diagnostic.message, diagnostic.span);
    let retained_bytes = self
      .expression_function_diagnostics
      .iter()
      .try_fold(0usize, |total, diagnostic| {
        total.checked_add(diagnostic.message.len())
      });
    let fits = retained_bytes
      .and_then(|total| total.checked_add(diagnostic.message.len()))
      .is_some_and(|total| total <= expression_function_diagnostic_byte_limit(limits));
    if fits {
      self.expression_function_diagnostics.push(diagnostic);
    } else {
      self.expression_function_diagnostics_truncated = true;
    }
  }
}

fn validate_functions<'a>(
  functions: impl IntoIterator<Item = &'a ExpressionFunction>,
  limits: ExpressionFunctionLimits,
) -> Result<(), Diagnostic> {
  let mut usage = ExpressionFunctionUsage::default();
  for function in functions {
    checked_charge(
      &mut usage.functions,
      1,
      limits.max_functions,
      "expression function count limit exceeded",
      function.expression.span,
    )?;
    checked_charge(
      &mut usage.parameters,
      function.params.len(),
      limits.max_total_parameters,
      "expression function parameter limit exceeded",
      function.expression.span,
    )?;
    let signature_bytes = function
      .params
      .iter()
      .try_fold(function.name.len(), |total, param| {
        total.checked_add(param.len())
      })
      .ok_or_else(|| {
        Diagnostic::new(
          "expression function name byte counter overflowed",
          function.expression.span,
        )
      })?;
    checked_charge(
      &mut usage.name_bytes,
      signature_bytes,
      limits.max_total_name_bytes,
      "expression function name byte limit exceeded",
      function.expression.span,
    )?;
    validate_signature(function)?;
    let body = preflight_ast(
      &function.expression,
      limits.max_total_body_nodes,
      limits
        .max_body_depth
        .min(MAX_EXPRESSION_FUNCTION_BODY_DEPTH),
    )
    .map_err(first_diagnostic)?;
    let syntax = validate_ast_syntax(&function.expression).map_err(first_diagnostic)?;
    checked_charge(
      &mut usage.body_nodes,
      body.nodes,
      limits.max_total_body_nodes,
      "expression function body node limit exceeded",
      function.expression.span,
    )?;
    checked_charge(
      &mut usage.body_scalar_bytes,
      syntax.scalar_bytes,
      limits.max_total_body_scalar_bytes,
      "expression function body scalar byte limit exceeded",
      function.expression.span,
    )?;
    checked_charge(
      &mut usage.call_edges,
      syntax.function_calls,
      limits.max_total_call_edges,
      "expression function call edge limit exceeded",
      function.expression.span,
    )?;
  }
  Ok(())
}

fn validate_signature(function: &ExpressionFunction) -> Result<(), Diagnostic> {
  validate_signature_parts(&function.name, &function.params, function.expression.span)
}

fn validate_signature_parts(
  name: &str,
  params: &[String],
  span: SourceSpan,
) -> Result<(), Diagnostic> {
  if !valid_function_identifier(name) {
    return Err(Diagnostic::new(
      format!(
        "function name {} must be a valid identifier",
        diagnostic_name(name)
      ),
      span,
    ));
  }
  let mut parameters = HashSet::new();
  for parameter in params {
    if !valid_function_identifier(parameter) {
      return Err(Diagnostic::new(
        format!(
          "function {} parameter {} must be a valid identifier",
          diagnostic_name(name),
          diagnostic_name(parameter)
        ),
        span,
      ));
    }
    if !parameters.insert(parameter) {
      return Err(Diagnostic::new(
        format!(
          "function {} contains duplicate parameter {}",
          diagnostic_name(name),
          diagnostic_name(parameter)
        ),
        span,
      ));
    }
  }
  Ok(())
}

fn valid_function_identifier(identifier: &str) -> bool {
  is_valid_identifier(identifier)
    && !matches!(
      identifier,
      "Context" | "Request" | "DynamicPolicy" | "Response" | "Stream"
    )
}

fn checked_charge(
  current: &mut usize,
  added: usize,
  limit: usize,
  message: &str,
  span: SourceSpan,
) -> Result<(), Diagnostic> {
  let Some(next) = current.checked_add(added) else {
    return Err(Diagnostic::new(
      "expression function aggregate counter overflowed",
      span,
    ));
  };
  if next > limit {
    Err(Diagnostic::new(message, span))
  } else {
    *current = next;
    Ok(())
  }
}

pub(crate) fn diagnostic_name(name: &str) -> String {
  bounded_text(name, MAX_DIAGNOSTIC_NAME_BYTES)
}

pub(crate) fn bounded_diagnostic_message(message: &str) -> String {
  bounded_text(message, MAX_DIAGNOSTIC_MESSAGE_BYTES)
}

pub(crate) fn expression_function_diagnostic_limit(limits: ExpressionFunctionLimits) -> usize {
  limits
    .max_diagnostics
    .min(MAX_EXPRESSION_FUNCTION_DIAGNOSTICS)
}

pub(crate) fn expression_function_diagnostic_byte_limit(limits: ExpressionFunctionLimits) -> usize {
  limits
    .max_total_diagnostic_bytes
    .min(MAX_TOTAL_DIAGNOSTIC_BYTES)
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
  if value.len() <= max_bytes {
    return value.to_string();
  }
  let suffix = "...";
  let mut end = max_bytes.saturating_sub(suffix.len()).min(value.len());
  while !value.is_char_boundary(end) {
    end -= 1;
  }
  let mut bounded = String::with_capacity(end + suffix.len());
  bounded.push_str(&value[..end]);
  bounded.push_str(suffix);
  bounded
}

fn try_charge_bytes(current: &mut usize, added: usize, limit: usize) -> bool {
  let Some(next) = current.checked_add(added) else {
    return false;
  };
  if next > limit {
    false
  } else {
    *current = next;
    true
  }
}

fn first_diagnostic(report: crate::parser::DiagnosticReport) -> Diagnostic {
  report.diagnostics.into_iter().next().unwrap_or_else(|| {
    Diagnostic::new(
      "expression function validation failed",
      SourceSpan::default(),
    )
  })
}
