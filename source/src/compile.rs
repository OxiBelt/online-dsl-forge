use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ast::{AstExpression, ExprKind};
use crate::diagnostics::{Diagnostic, DiagnosticReport};

#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeSchema {
  variables: BTreeSet<String>,
  functions: BTreeMap<String, BTreeSet<usize>>,
  methods: BTreeMap<String, BTreeSet<usize>>,
}

impl RuntimeSchema {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn add_variable(&mut self, name: impl Into<String>) -> &mut Self {
    self.variables.insert(name.into());
    self
  }

  pub fn add_function(&mut self, name: impl Into<String>, arity: usize) -> &mut Self {
    self.functions.entry(name.into()).or_default().insert(arity);
    self
  }

  pub fn add_method(&mut self, name: impl Into<String>, arity: usize) -> &mut Self {
    self.methods.entry(name.into()).or_default().insert(arity);
    self
  }

  pub fn has_variable(&self, name: &str) -> bool {
    self.variables.contains(name)
  }

  pub fn function_accepts(&self, name: &str, arity: usize) -> SignatureMatch {
    signature_accepts(&self.functions, name, arity)
  }

  pub fn method_accepts(&self, name: &str, arity: usize) -> SignatureMatch {
    signature_accepts(&self.methods, name, arity)
  }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SignatureMatch {
  Unknown,
  ArityMismatch,
  Matches,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompileOptions {
  pub allow_unknown_variables: bool,
  pub allow_unknown_functions: bool,
  pub allow_unknown_methods: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct CompiledExpression {
  ast: AstExpression,
}

impl CompiledExpression {
  pub fn ast(&self) -> &AstExpression {
    &self.ast
  }

  pub fn into_ast(self) -> AstExpression {
    self.ast
  }
}

pub fn compile_expression(
  expression: &AstExpression,
  schema: &RuntimeSchema,
  options: CompileOptions,
) -> Result<CompiledExpression, DiagnosticReport> {
  let mut diagnostics = Vec::new();
  validate_expression(expression, schema, options, &mut diagnostics);
  if diagnostics.is_empty() {
    Ok(CompiledExpression {
      ast: expression.clone(),
    })
  } else {
    Err(DiagnosticReport::new(diagnostics))
  }
}

fn signature_accepts(
  signatures: &BTreeMap<String, BTreeSet<usize>>,
  name: &str,
  arity: usize,
) -> SignatureMatch {
  match signatures.get(name) {
    Some(accepted) if accepted.contains(&arity) => SignatureMatch::Matches,
    Some(_) => SignatureMatch::ArityMismatch,
    None => SignatureMatch::Unknown,
  }
}

fn validate_expression(
  expression: &AstExpression,
  schema: &RuntimeSchema,
  options: CompileOptions,
  diagnostics: &mut Vec<Diagnostic>,
) {
  match &expression.kind {
    ExprKind::Identifier { name } => {
      if !options.allow_unknown_variables && !schema.has_variable(name) {
        diagnostics.push(Diagnostic::new(
          format!("unknown variable {name}"),
          expression.span,
        ));
      }
    }
    ExprKind::Array { items } => {
      for item in items {
        validate_expression(item, schema, options, diagnostics);
      }
    }
    ExprKind::Member { receiver, .. } => {
      validate_expression(receiver, schema, options, diagnostics);
    }
    ExprKind::FunctionCall { name, args } => {
      validate_call(
        "function",
        name,
        args.len(),
        schema.function_accepts(name, args.len()),
        options.allow_unknown_functions,
        expression,
        diagnostics,
      );
      for arg in args {
        validate_expression(arg, schema, options, diagnostics);
      }
    }
    ExprKind::MethodCall {
      receiver,
      name,
      args,
    } => {
      validate_expression(receiver, schema, options, diagnostics);
      validate_call(
        "method",
        name,
        args.len(),
        schema.method_accepts(name, args.len()),
        options.allow_unknown_methods,
        expression,
        diagnostics,
      );
      for arg in args {
        validate_expression(arg, schema, options, diagnostics);
      }
    }
    ExprKind::Unary { expr, .. } => validate_expression(expr, schema, options, diagnostics),
    ExprKind::Binary { left, right, .. } => {
      validate_expression(left, schema, options, diagnostics);
      validate_expression(right, schema, options, diagnostics);
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. } => {}
  }
}

fn validate_call(
  kind: &'static str,
  name: &str,
  arity: usize,
  result: SignatureMatch,
  allow_unknown: bool,
  expression: &AstExpression,
  diagnostics: &mut Vec<Diagnostic>,
) {
  match result {
    SignatureMatch::Matches => {}
    SignatureMatch::Unknown if allow_unknown => {}
    SignatureMatch::Unknown => diagnostics.push(Diagnostic::new(
      format!("unknown {kind} {name}"),
      expression.span,
    )),
    SignatureMatch::ArityMismatch => diagnostics.push(Diagnostic::new(
      format!("{kind} {name} does not accept {arity} arguments"),
      expression.span,
    )),
  }
}

#[cfg(test)]
mod tests {
  use crate::parse_expression;

  use super::{CompileOptions, RuntimeSchema, compile_expression};

  #[test]
  fn rejects_unknown_variable() {
    let ast = parse_expression("score > 10").expect("expression should parse");
    let error = compile_expression(&ast, &RuntimeSchema::new(), CompileOptions::default())
      .expect_err("unknown variable should fail");
    assert!(error.to_string().contains("unknown variable score"));
  }

  #[test]
  fn validates_function_arity() {
    let ast = parse_expression("len(items, extra)").expect("expression should parse");
    let mut schema = RuntimeSchema::new();
    schema
      .add_variable("items")
      .add_variable("extra")
      .add_function("len", 1);
    let error = compile_expression(&ast, &schema, CompileOptions::default())
      .expect_err("bad arity should fail");
    assert!(error.to_string().contains("does not accept 2 arguments"));
  }
}
