use super::ast::{AstExpression, BinaryOp, ExprKind};
use super::diagnostics::DiagnosticReport;
use super::limits::AstFormatLimits;
use super::preflight::preflight_ast;
use super::validation::validate_ast_syntax;

pub fn format_expression(expression: &AstExpression) -> String {
  format_expression_with_limits(expression, AstFormatLimits::default()).unwrap_or_default()
}

pub fn format_expression_with_limits(
  expression: &AstExpression,
  limits: AstFormatLimits,
) -> Result<String, DiagnosticReport> {
  preflight_ast(expression, limits.max_nodes, limits.max_depth).map_err(|report| {
    let diagnostic = &report.diagnostics[0];
    if diagnostic.message == "semantic call depth limit exceeded" {
      DiagnosticReport::single("AST format depth limit exceeded", diagnostic.span)
    } else {
      report
    }
  })?;
  validate_ast_syntax(expression)?;

  let mut formatter = Formatter {
    output: String::new(),
    max_output_bytes: limits.max_output_bytes,
    root_span: expression.span,
  };
  let mut stack = vec![Task::Expression(expression, 0, ChildSide::Root)];
  while let Some(task) = stack.pop() {
    match task {
      Task::Expression(expression, parent, side) => {
        schedule_expression(&mut stack, expression, parent, side);
      }
      Task::Text(text) => formatter.push_str(text)?,
      Task::Owned(text) => formatter.push_str(&text)?,
      Task::Escaped(value) => formatter.push_escaped(value)?,
    }
  }
  Ok(formatter.output)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ChildSide {
  Root,
  Left,
  Right,
  Unary,
  Receiver,
}

enum Task<'a> {
  Expression(&'a AstExpression, u8, ChildSide),
  Text(&'a str),
  Owned(String),
  Escaped(&'a str),
}

fn schedule_expression<'a>(
  stack: &mut Vec<Task<'a>>,
  expression: &'a AstExpression,
  parent_precedence: u8,
  side: ChildSide,
) {
  let own_precedence = precedence(expression);
  let parenthesized = needs_parentheses(own_precedence, parent_precedence, side);
  if parenthesized {
    stack.push(Task::Text(")"));
  }
  match &expression.kind {
    ExprKind::Null => stack.push(Task::Text("null")),
    ExprKind::Bool { value } => stack.push(Task::Text(if *value { "true" } else { "false" })),
    ExprKind::Int { value } => stack.push(Task::Owned(value.to_string())),
    ExprKind::Float { value } => stack.push(Task::Owned(format_float(*value))),
    ExprKind::String { value } => {
      stack.push(Task::Text("\""));
      stack.push(Task::Escaped(value));
      stack.push(Task::Text("\""));
    }
    ExprKind::Array { items } => {
      stack.push(Task::Text("]"));
      schedule_sequence(stack, items);
      stack.push(Task::Text("["));
    }
    ExprKind::Identifier { name } => stack.push(Task::Text(name)),
    ExprKind::Member { receiver, name } => {
      stack.push(Task::Text(name));
      stack.push(Task::Text("."));
      stack.push(Task::Expression(
        receiver,
        own_precedence,
        ChildSide::Receiver,
      ));
    }
    ExprKind::FunctionCall { name, args } => {
      stack.push(Task::Text(")"));
      schedule_sequence(stack, args);
      stack.push(Task::Text("("));
      stack.push(Task::Text(name));
    }
    ExprKind::MethodCall {
      receiver,
      name,
      args,
    } => {
      stack.push(Task::Text(")"));
      schedule_sequence(stack, args);
      stack.push(Task::Text("("));
      stack.push(Task::Text(name));
      stack.push(Task::Text("."));
      stack.push(Task::Expression(
        receiver,
        own_precedence,
        ChildSide::Receiver,
      ));
    }
    ExprKind::Unary { op, expr } => {
      stack.push(Task::Expression(expr, own_precedence, ChildSide::Unary));
      stack.push(Task::Text(op.as_str()));
    }
    ExprKind::Binary { left, op, right } => {
      stack.push(Task::Expression(right, own_precedence, ChildSide::Right));
      stack.push(Task::Text(" "));
      stack.push(Task::Text(op.as_str()));
      stack.push(Task::Text(" "));
      stack.push(Task::Expression(left, own_precedence, ChildSide::Left));
    }
  }
  if parenthesized {
    stack.push(Task::Text("("));
  }
}

fn schedule_sequence<'a>(stack: &mut Vec<Task<'a>>, expressions: &'a [AstExpression]) {
  for (index, expression) in expressions.iter().enumerate().rev() {
    stack.push(Task::Expression(expression, 0, ChildSide::Root));
    if index > 0 {
      stack.push(Task::Text(", "));
    }
  }
}

struct Formatter {
  output: String,
  max_output_bytes: usize,
  root_span: super::SourceSpan,
}

impl Formatter {
  fn push_str(&mut self, value: &str) -> Result<(), DiagnosticReport> {
    let Some(length) = self.output.len().checked_add(value.len()) else {
      return Err(self.output_error());
    };
    if length > self.max_output_bytes {
      return Err(self.output_error());
    }
    self.output.push_str(value);
    Ok(())
  }

  fn push_escaped(&mut self, value: &str) -> Result<(), DiagnosticReport> {
    for ch in value.chars() {
      match ch {
        '\\' => self.push_str("\\\\")?,
        '"' => self.push_str("\\\"")?,
        '\n' => self.push_str("\\n")?,
        '\r' => self.push_str("\\r")?,
        '\t' => self.push_str("\\t")?,
        other => {
          let mut bytes = [0_u8; 4];
          self.push_str(other.encode_utf8(&mut bytes))?;
        }
      }
    }
    Ok(())
  }

  fn output_error(&self) -> DiagnosticReport {
    DiagnosticReport::single("AST format output byte limit exceeded", self.root_span)
  }
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
  !matches!(side, ChildSide::Root) && (own < parent || (side == ChildSide::Right && own == parent))
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
