use crate::parser::{BinaryOp, SourceSpan, UnaryOp};
use crate::value::Value;

use super::EvalError;

pub(super) fn expect_bool(value: Value, span: SourceSpan) -> Result<bool, EvalError> {
  match value {
    Value::Bool(value) => Ok(value),
    other => Err(EvalError::new(
      format!("expected bool, got {}", other.type_name()),
      span,
    )),
  }
}

pub(super) fn add_values(
  left: Value,
  right: Value,
  span: SourceSpan,
  max_string_bytes: usize,
) -> Result<Value, EvalError> {
  match (left, right) {
    (Value::Int(left), Value::Int(right)) => left
      .checked_add(right)
      .map(Value::Int)
      .ok_or_else(|| EvalError::new("integer addition overflowed", span)),
    (left, right) if left.is_number() && right.is_number() => {
      Ok(Value::Float(number_as_f64(&left) + number_as_f64(&right)))
    }
    (Value::String(mut left), Value::String(right)) => {
      left.push_str(&right);
      if left.len() > max_string_bytes {
        Err(EvalError::new("string byte limit exceeded", span))
      } else {
        Ok(Value::String(left))
      }
    }
    (left, right) => Err(type_error("+", &left, &right, span)),
  }
}

pub(super) fn numeric_arithmetic(
  left: Value,
  op: BinaryOp,
  right: Value,
  span: SourceSpan,
) -> Result<Value, EvalError> {
  match (left, right) {
    (Value::Int(left), Value::Int(right)) => int_arithmetic(left, op, right, span),
    (left, right) if left.is_number() && right.is_number() => {
      let left = number_as_f64(&left);
      let right = number_as_f64(&right);
      if matches!(op, BinaryOp::Div | BinaryOp::Rem) && right == 0.0 {
        return Err(EvalError::new("division by zero", span));
      }
      Ok(Value::Float(match op {
        BinaryOp::Sub => left - right,
        BinaryOp::Mul => left * right,
        BinaryOp::Div => left / right,
        BinaryOp::Rem => left % right,
        _ => return Err(EvalError::new("internal arithmetic dispatch error", span)),
      }))
    }
    (left, right) => Err(type_error(op.as_str(), &left, &right, span)),
  }
}

pub(super) fn compare_values(
  left: Value,
  op: BinaryOp,
  right: Value,
  span: SourceSpan,
) -> Result<Value, EvalError> {
  let result = match (&left, &right) {
    (left, right) if left.is_number() && right.is_number() => {
      compare_f64(number_as_f64(left), op, number_as_f64(right))
    }
    (Value::String(left), Value::String(right)) => compare_order(left, op, right),
    _ => return Err(type_error(op.as_str(), &left, &right, span)),
  };
  Ok(Value::Bool(result))
}

pub(super) fn type_error(op: &str, left: &Value, right: &Value, span: SourceSpan) -> EvalError {
  EvalError::new(
    format!(
      "operator {op} does not accept {} and {}",
      left.type_name(),
      right.type_name()
    ),
    span,
  )
}

pub(super) fn unary_op_from_name(name: &str) -> Option<UnaryOp> {
  match name {
    "!" => Some(UnaryOp::Not),
    "-" => Some(UnaryOp::Neg),
    _ => None,
  }
}

pub(super) fn binary_op_from_name(name: &str) -> Option<BinaryOp> {
  match name {
    "||" => Some(BinaryOp::Or),
    "&&" => Some(BinaryOp::And),
    "==" => Some(BinaryOp::Eq),
    "!=" => Some(BinaryOp::Ne),
    "<" => Some(BinaryOp::Lt),
    "<=" => Some(BinaryOp::Le),
    ">" => Some(BinaryOp::Gt),
    ">=" => Some(BinaryOp::Ge),
    "+" => Some(BinaryOp::Add),
    "-" => Some(BinaryOp::Sub),
    "*" => Some(BinaryOp::Mul),
    "/" => Some(BinaryOp::Div),
    "%" => Some(BinaryOp::Rem),
    _ => None,
  }
}

fn int_arithmetic(
  left: i64,
  op: BinaryOp,
  right: i64,
  span: SourceSpan,
) -> Result<Value, EvalError> {
  if matches!(op, BinaryOp::Div | BinaryOp::Rem) && right == 0 {
    return Err(EvalError::new("division by zero", span));
  }
  let value = match op {
    BinaryOp::Sub => left.checked_sub(right),
    BinaryOp::Mul => left.checked_mul(right),
    BinaryOp::Div => left.checked_div(right),
    BinaryOp::Rem => left.checked_rem(right),
    _ => None,
  };
  value
    .map(Value::Int)
    .ok_or_else(|| EvalError::new("integer arithmetic overflowed", span))
}

fn compare_f64(left: f64, op: BinaryOp, right: f64) -> bool {
  match op {
    BinaryOp::Lt => left < right,
    BinaryOp::Le => left <= right,
    BinaryOp::Gt => left > right,
    BinaryOp::Ge => left >= right,
    _ => false,
  }
}

fn compare_order<T: Ord>(left: &T, op: BinaryOp, right: &T) -> bool {
  match op {
    BinaryOp::Lt => left < right,
    BinaryOp::Le => left <= right,
    BinaryOp::Gt => left > right,
    BinaryOp::Ge => left >= right,
    _ => false,
  }
}

fn number_as_f64(value: &Value) -> f64 {
  match value {
    Value::Int(value) => *value as f64,
    Value::Float(value) => *value,
    _ => 0.0,
  }
}
