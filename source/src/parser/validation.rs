use super::{AstExpression, DiagnosticReport, ExprKind};

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct AstSyntaxMetrics {
  pub(crate) scalar_bytes: usize,
  pub(crate) function_calls: usize,
}

pub(crate) fn validate_ast_syntax(
  expression: &AstExpression,
) -> Result<AstSyntaxMetrics, DiagnosticReport> {
  let mut metrics = AstSyntaxMetrics::default();
  let mut pending = vec![expression];

  while let Some(expression) = pending.pop() {
    match &expression.kind {
      ExprKind::String { value } => charge_scalar(&mut metrics, value.len()),
      ExprKind::Identifier { name } => {
        validate_name(name, "identifier", expression)?;
        charge_scalar(&mut metrics, name.len());
      }
      ExprKind::Member { receiver, name } => {
        validate_name(name, "member", expression)?;
        charge_scalar(&mut metrics, name.len());
        pending.push(receiver);
      }
      ExprKind::FunctionCall { name, args } => {
        validate_name(name, "function", expression)?;
        charge_scalar(&mut metrics, name.len());
        metrics.function_calls = metrics.function_calls.saturating_add(1);
        pending.extend(args.iter().rev());
      }
      ExprKind::MethodCall {
        receiver,
        name,
        args,
      } => {
        validate_name(name, "method", expression)?;
        charge_scalar(&mut metrics, name.len());
        pending.extend(args.iter().rev());
        pending.push(receiver);
      }
      ExprKind::Array { items } => pending.extend(items.iter().rev()),
      ExprKind::Unary { expr, .. } => pending.push(expr),
      ExprKind::Binary { left, right, .. } => {
        pending.push(right);
        pending.push(left);
      }
      ExprKind::Float { value } if !value.is_finite() => {
        return Err(DiagnosticReport::single(
          "float value must be finite",
          expression.span,
        ));
      }
      ExprKind::Null | ExprKind::Bool { .. } | ExprKind::Int { .. } | ExprKind::Float { .. } => {}
    }
  }

  Ok(metrics)
}

pub(crate) fn is_valid_identifier(identifier: &str) -> bool {
  let mut bytes = identifier.bytes();
  let Some(first) = bytes.next() else {
    return false;
  };
  (first.is_ascii_alphabetic() || first == b'_')
    && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    && !is_reserved_identifier(identifier)
}

pub(crate) fn is_reserved_identifier(identifier: &str) -> bool {
  matches!(
    identifier,
    "if"
      | "else"
      | "for"
      | "while"
      | "do"
      | "switch"
      | "let"
      | "const"
      | "function"
      | "import"
      | "export"
      | "new"
      | "try"
      | "catch"
      | "throw"
      | "await"
      | "return"
      | "true"
      | "false"
      | "null"
  )
}

fn validate_name(
  name: &str,
  kind: &str,
  expression: &AstExpression,
) -> Result<(), DiagnosticReport> {
  if is_valid_identifier(name) {
    Ok(())
  } else {
    Err(DiagnosticReport::single(
      format!("{kind} name must follow identifier syntax and must not be reserved"),
      expression.span,
    ))
  }
}

fn charge_scalar(metrics: &mut AstSyntaxMetrics, bytes: usize) {
  metrics.scalar_bytes = metrics.scalar_bytes.saturating_add(bytes);
}
