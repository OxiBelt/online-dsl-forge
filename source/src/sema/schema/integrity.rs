use std::collections::BTreeMap;

use serde::Deserialize;

use crate::parser::{Diagnostic, SourceSpan};

use super::{
  BodyPathRule, CapabilityKind, CapabilityMeta, ExpressionFunction, ExpressionFunctionDiagnostic,
  ExpressionFunctionLimits, ExpressionFunctionScope, RuntimeSchema, VariableMeta, diagnostic_name,
};

#[derive(Deserialize)]
pub(super) struct RuntimeSchemaWire {
  variables: BTreeMap<String, VariableMeta>,
  functions: BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  methods: BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  unary_ops: BTreeMap<String, CapabilityMeta>,
  binary_ops: BTreeMap<String, CapabilityMeta>,
  body_paths: Vec<BodyPathRule>,
  expression_functions: BTreeMap<String, ExpressionFunction>,
  #[serde(default)]
  local_expression_functions: BTreeMap<String, ExpressionFunction>,
  #[serde(default)]
  expression_function_diagnostics: Vec<ExpressionFunctionDiagnostic>,
  #[serde(default)]
  expression_function_diagnostics_truncated: bool,
}

impl TryFrom<RuntimeSchemaWire> for RuntimeSchema {
  type Error = String;

  fn try_from(wire: RuntimeSchemaWire) -> Result<Self, Self::Error> {
    let schema = Self {
      variables: wire.variables,
      functions: wire.functions,
      methods: wire.methods,
      unary_ops: wire.unary_ops,
      binary_ops: wire.binary_ops,
      body_paths: wire.body_paths,
      expression_functions: wire.expression_functions,
      local_expression_functions: wire.local_expression_functions,
      expression_function_diagnostics: wire.expression_function_diagnostics,
      expression_function_diagnostics_truncated: wire.expression_function_diagnostics_truncated,
      expression_function_limits: ExpressionFunctionLimits::default(),
    };
    schema
      .validate_integrity()
      .map_err(|diagnostic| diagnostic.message)?;
    Ok(schema)
  }
}

impl RuntimeSchema {
  pub(crate) fn validate_integrity(&self) -> Result<(), Diagnostic> {
    for (name, variable) in &self.variables {
      if variable.name != *name {
        return Err(identity_error("variable", name, None));
      }
    }
    validate_capability_map(&self.functions, CapabilityKind::Function, "function")?;
    validate_capability_map(&self.methods, CapabilityKind::Method, "method")?;
    validate_operator_map(
      &self.unary_ops,
      CapabilityKind::UnaryOp,
      1,
      &["!", "-"],
      "unary operator",
    )?;
    validate_operator_map(
      &self.binary_ops,
      CapabilityKind::BinaryOp,
      2,
      &[
        "||", "&&", "==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%",
      ],
      "binary operator",
    )?;
    validate_expression_functions(
      &self.expression_functions,
      ExpressionFunctionScope::Global,
      "global expression function",
    )?;
    validate_expression_functions(
      &self.local_expression_functions,
      ExpressionFunctionScope::Local,
      "local expression function",
    )?;
    Ok(())
  }
}

fn validate_capability_map(
  capabilities: &BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  expected_kind: CapabilityKind,
  label: &str,
) -> Result<(), Diagnostic> {
  for (name, signatures) in capabilities {
    for (arity, capability) in signatures {
      validate_capability(capability, expected_kind, name, *arity, label)?;
    }
  }
  Ok(())
}

fn validate_operator_map(
  capabilities: &BTreeMap<String, CapabilityMeta>,
  expected_kind: CapabilityKind,
  expected_arity: usize,
  valid_names: &[&str],
  label: &str,
) -> Result<(), Diagnostic> {
  for (name, capability) in capabilities {
    if !valid_names.contains(&name.as_str()) {
      return Err(identity_error(label, name, Some(expected_arity)));
    }
    validate_capability(capability, expected_kind, name, expected_arity, label)?;
  }
  Ok(())
}

fn validate_capability(
  capability: &CapabilityMeta,
  expected_kind: CapabilityKind,
  expected_name: &str,
  expected_arity: usize,
  label: &str,
) -> Result<(), Diagnostic> {
  let identity_matches = capability.kind == expected_kind
    && capability.name == expected_name
    && capability.arity == expected_arity
    && capability.args.len() == expected_arity
    && capability
      .regex_args
      .iter()
      .all(|argument| argument.index < expected_arity);
  if identity_matches {
    Ok(())
  } else {
    Err(identity_error(label, expected_name, Some(expected_arity)))
  }
}

fn validate_expression_functions(
  functions: &BTreeMap<String, ExpressionFunction>,
  expected_scope: ExpressionFunctionScope,
  label: &str,
) -> Result<(), Diagnostic> {
  for (name, function) in functions {
    if function.name != *name || function.scope != expected_scope {
      return Err(identity_error(label, name, Some(function.params.len())));
    }
  }
  Ok(())
}

fn identity_error(label: &str, name: &str, arity: Option<usize>) -> Diagnostic {
  let suffix = arity.map_or_else(String::new, |arity| format!("/{arity}"));
  Diagnostic::new(
    format!(
      "runtime schema {label} identity mismatch for {}{suffix}",
      diagnostic_name(name)
    ),
    SourceSpan::default(),
  )
}
