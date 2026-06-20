use crate::parser::{AstExpression, BinaryOp, Diagnostic, ExprKind, UnaryOp};

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ExpressionDialect {
  #[default]
  Generic,
  OxiRuleV1,
}

impl ExpressionDialect {
  pub(crate) fn validate(self, expression: &AstExpression, diagnostics: &mut Vec<Diagnostic>) {
    match self {
      Self::Generic => {}
      Self::OxiRuleV1 => validate_oxirule_v1(expression, diagnostics),
    }
  }
}

fn validate_oxirule_v1(expression: &AstExpression, diagnostics: &mut Vec<Diagnostic>) {
  match &expression.kind {
    ExprKind::Float { .. } => diagnostics.push(Diagnostic::new(
      "OxiRule V1 does not support float literals",
      expression.span,
    )),
    ExprKind::Array { items } => {
      diagnostics.push(Diagnostic::new(
        "OxiRule V1 does not support array literals",
        expression.span,
      ));
      for item in items {
        validate_oxirule_v1(item, diagnostics);
      }
    }
    ExprKind::Unary {
      op: UnaryOp::Neg,
      expr,
    } => {
      diagnostics.push(Diagnostic::new(
        "OxiRule V1 does not support unary numeric negation",
        expression.span,
      ));
      validate_oxirule_v1(expr, diagnostics);
    }
    ExprKind::Unary { expr, .. } => validate_oxirule_v1(expr, diagnostics),
    ExprKind::Binary { left, op, right } => {
      if matches!(
        op,
        BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
      ) {
        diagnostics.push(Diagnostic::new(
          format!("OxiRule V1 does not support operator {}", op.as_str()),
          expression.span,
        ));
      }
      validate_oxirule_v1(left, diagnostics);
      validate_oxirule_v1(right, diagnostics);
    }
    ExprKind::Member { receiver, .. } => validate_oxirule_v1(receiver, diagnostics),
    ExprKind::FunctionCall { args, .. } => {
      for arg in args {
        validate_oxirule_v1(arg, diagnostics);
      }
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      validate_oxirule_v1(receiver, diagnostics);
      for arg in args {
        validate_oxirule_v1(arg, diagnostics);
      }
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => {}
  }
}
