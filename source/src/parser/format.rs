use super::ast::{AstExpression, BinaryOp, ExprKind};

pub fn format_expression(expression: &AstExpression) -> String {
  format_with_parent(expression, 0, ChildSide::Root)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChildSide {
  Root,
  Left,
  Right,
  Unary,
  Receiver,
}

fn format_with_parent(
  expression: &AstExpression,
  parent_precedence: u8,
  side: ChildSide,
) -> String {
  let own_precedence = precedence(expression);
  let mut output = match &expression.kind {
    ExprKind::Null => "null".to_string(),
    ExprKind::Bool { value } => value.to_string(),
    ExprKind::Int { value } => value.to_string(),
    ExprKind::Float { value } => format_float(*value),
    ExprKind::String { value } => format!("\"{}\"", escape_string(value)),
    ExprKind::Array { items } => {
      let items = items
        .iter()
        .map(format_expression)
        .collect::<Vec<_>>()
        .join(", ");
      format!("[{items}]")
    }
    ExprKind::Identifier { name } => name.clone(),
    ExprKind::Member { receiver, name } => {
      format!(
        "{}.{}",
        format_with_parent(receiver, own_precedence, ChildSide::Receiver),
        name
      )
    }
    ExprKind::FunctionCall { name, args } => format!("{name}({})", format_args(args)),
    ExprKind::MethodCall {
      receiver,
      name,
      args,
    } => format!(
      "{}.{}({})",
      format_with_parent(receiver, own_precedence, ChildSide::Receiver),
      name,
      format_args(args)
    ),
    ExprKind::Unary { op, expr } => {
      format!(
        "{}{}",
        op.as_str(),
        format_with_parent(expr, own_precedence, ChildSide::Unary)
      )
    }
    ExprKind::Binary { left, op, right } => format!(
      "{} {} {}",
      format_with_parent(left, own_precedence, ChildSide::Left),
      op.as_str(),
      format_with_parent(right, own_precedence, ChildSide::Right)
    ),
  };

  if needs_parentheses(own_precedence, parent_precedence, side) {
    output = format!("({output})");
  }
  output
}

fn format_args(args: &[AstExpression]) -> String {
  args
    .iter()
    .map(format_expression)
    .collect::<Vec<_>>()
    .join(", ")
}

fn precedence(expression: &AstExpression) -> u8 {
  match &expression.kind {
    ExprKind::Binary { op, .. } => binary_precedence(*op),
    ExprKind::Unary { .. } => 7,
    ExprKind::Member { .. } | ExprKind::FunctionCall { .. } | ExprKind::MethodCall { .. } => 8,
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Array { .. }
    | ExprKind::Identifier { .. } => 9,
  }
}

fn binary_precedence(op: BinaryOp) -> u8 {
  match op {
    BinaryOp::Or => 1,
    BinaryOp::And => 2,
    BinaryOp::Eq | BinaryOp::Ne => 3,
    BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 4,
    BinaryOp::Add | BinaryOp::Sub => 5,
    BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 6,
  }
}

fn needs_parentheses(own: u8, parent: u8, side: ChildSide) -> bool {
  if matches!(side, ChildSide::Root) {
    return false;
  }
  own < parent || (side == ChildSide::Right && own == parent)
}

fn escape_string(value: &str) -> String {
  let mut escaped = String::new();
  for ch in value.chars() {
    match ch {
      '\\' => escaped.push_str("\\\\"),
      '"' => escaped.push_str("\\\""),
      '\n' => escaped.push_str("\\n"),
      '\r' => escaped.push_str("\\r"),
      '\t' => escaped.push_str("\\t"),
      other => escaped.push(other),
    }
  }
  escaped
}

fn format_float(value: f64) -> String {
  let mut output = value.to_string();
  if value.is_finite() && !output.contains('.') && !output.contains('e') && !output.contains('E') {
    output.push_str(".0");
  }
  output
}

#[cfg(test)]
mod tests {
  use crate::parse_expression;

  use super::format_expression;

  #[test]
  fn preserves_right_nested_binary_shape() {
    let ast = parse_expression("1 - (2 - 3)").expect("expression should parse");
    assert_eq!(format_expression(&ast), "1 - (2 - 3)");
  }

  #[test]
  fn normalizes_strings() {
    let ast = parse_expression("'a\\nb'").expect("expression should parse");
    assert_eq!(format_expression(&ast), "\"a\\nb\"");
  }
}
