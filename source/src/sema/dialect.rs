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
  let mut stack = vec![expression];
  while let Some(expression) = stack.pop() {
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
        for item in items.iter().rev() {
          stack.push(item);
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
        stack.push(expr);
      }
      ExprKind::Unary { expr, .. } => stack.push(expr),
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
        stack.push(right);
        stack.push(left);
      }
      ExprKind::Member { receiver, .. } => stack.push(receiver),
      ExprKind::FunctionCall { args, .. } => {
        for arg in args.iter().rev() {
          stack.push(arg);
        }
      }
      ExprKind::MethodCall { receiver, args, .. } => {
        for arg in args.iter().rev() {
          stack.push(arg);
        }
        stack.push(receiver);
      }
      ExprKind::Null
      | ExprKind::Bool { .. }
      | ExprKind::Int { .. }
      | ExprKind::String { .. }
      | ExprKind::Identifier { .. } => {}
    }
  }
}
