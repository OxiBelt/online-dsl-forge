use std::collections::BTreeMap;

use crate::parser::{BinaryOp, SourceSpan, UnaryOp};
use crate::sema::{VerifiedExprKindRef, VerifiedExpression, VerifiedProgram};
use crate::value::{Value, ValueMetrics};

use super::operators::{add_values, compare_values, expect_bool, numeric_arithmetic};
use super::{
  DynamicRegistry, EvalError, EvalLimits, RuntimeCallContext, RuntimeContext,
  RuntimeResourceLimits, validate_value,
};

pub(super) fn evaluate(
  program: &VerifiedProgram,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
  resource_limits: RuntimeResourceLimits,
) -> Result<Value, EvalError> {
  let mut state = EvalState {
    limits,
    resource_limits,
    processed_bytes: 0,
    steps: 0,
    program,
    locals: Vec::new(),
  };
  state.eval(program.root(), context, 0)
}

struct EvalState<'a> {
  limits: EvalLimits,
  resource_limits: RuntimeResourceLimits,
  processed_bytes: usize,
  steps: usize,
  program: &'a VerifiedProgram,
  locals: Vec<BTreeMap<String, Value>>,
}

struct ExpressionFunctionFrame<'a> {
  name: &'a str,
  params: &'a [String],
  args: &'a [VerifiedExpression],
  body: &'a VerifiedExpression,
  span: SourceSpan,
}

impl EvalState<'_> {
  fn eval(
    &mut self,
    expression: &VerifiedExpression,
    context: &dyn RuntimeContext,
    depth: usize,
  ) -> Result<Value, EvalError> {
    let span = expression.span();
    self.step(span)?;
    if depth > self.limits.max_depth {
      return Err(EvalError::new("evaluation depth limit exceeded", span));
    }

    match expression.kind() {
      VerifiedExprKindRef::Null => self.admit(Value::Null, span),
      VerifiedExprKindRef::Bool(value) => self.admit(Value::Bool(value), span),
      VerifiedExprKindRef::Int(value) => self.admit(Value::Int(value), span),
      VerifiedExprKindRef::Float(value) => self.admit(Value::Float(value), span),
      VerifiedExprKindRef::String(value) => self.checked_string(value.to_string(), span),
      VerifiedExprKindRef::Array(items) => self.eval_array(items, context, depth, span),
      VerifiedExprKindRef::Identifier(name) => self.eval_identifier(name, context, span),
      VerifiedExprKindRef::Member { receiver, name } => {
        let value = self.eval(receiver, context, depth + 1)?;
        self.eval_member(value, name, span)
      }
      VerifiedExprKindRef::FunctionCall { name, args } => {
        let args = self.eval_args(args, context, depth)?;
        let value = context
          .registry()
          .call_function(self.call_context(span), name, &args, span)?;
        self.admit(value, span)
      }
      VerifiedExprKindRef::ExpressionFunctionCall {
        name,
        params,
        args,
        body,
      } => self.eval_expression_function(
        ExpressionFunctionFrame {
          name,
          params,
          args,
          body,
          span,
        },
        context,
        depth,
      ),
      VerifiedExprKindRef::MethodCall {
        receiver,
        name,
        args,
      } => {
        let receiver = self.eval(receiver, context, depth + 1)?;
        let args = self.eval_args(args, context, depth)?;
        let value =
          context
            .registry()
            .call_method(self.call_context(span), &receiver, name, &args, span)?;
        self.admit(value, span)
      }
      VerifiedExprKindRef::Unary { op, expr } => {
        let value = self.eval(expr, context, depth + 1)?;
        self.eval_unary(op, value, context.registry(), span)
      }
      VerifiedExprKindRef::Binary { left, op, right } => {
        self.eval_binary(left, op, right, context, depth, span)
      }
    }
  }

  fn step(&mut self, span: SourceSpan) -> Result<(), EvalError> {
    self.steps = self
      .steps
      .checked_add(1)
      .ok_or_else(|| EvalError::new("evaluation step counter overflowed", span))?;
    if self.steps > self.limits.max_steps {
      Err(EvalError::new("evaluation step limit exceeded", span))
    } else {
      Ok(())
    }
  }

  fn eval_identifier(
    &mut self,
    name: &str,
    context: &dyn RuntimeContext,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    if let Some(metrics) = self
      .local_value(name)
      .map(|value| validate_value(value, self.resource_limits, span))
      .transpose()?
    {
      self.charge(metrics, span)?;
      return self
        .local_value(name)
        .cloned()
        .ok_or_else(|| EvalError::new("local value disappeared during evaluation", span));
    }
    if let Some(value) = context.get_variable_borrowed(name) {
      return self.clone_admitted(value, span);
    }
    context
      .get_variable(name)
      .map(|value| self.admit(value, span))
      .transpose()?
      .ok_or_else(|| EvalError::new(format!("unknown variable {name}"), span))
  }

  fn eval_array(
    &mut self,
    items: &[VerifiedExpression],
    context: &dyn RuntimeContext,
    depth: usize,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    if items.len() > self.limits.max_array_items {
      return Err(EvalError::new("array item limit exceeded", span));
    }
    let values = items
      .iter()
      .map(|item| self.eval(item, context, depth + 1))
      .collect::<Result<Vec<_>, _>>()?;
    self.admit(Value::Array(values), span)
  }

  fn eval_args(
    &mut self,
    args: &[VerifiedExpression],
    context: &dyn RuntimeContext,
    depth: usize,
  ) -> Result<Vec<Value>, EvalError> {
    args
      .iter()
      .map(|arg| self.eval(arg, context, depth + 1))
      .collect()
  }

  fn eval_expression_function(
    &mut self,
    frame: ExpressionFunctionFrame<'_>,
    context: &dyn RuntimeContext,
    depth: usize,
  ) -> Result<Value, EvalError> {
    if frame.params.len() != frame.args.len() {
      return Err(EvalError::new(
        format!(
          "verified expression function {} expected {} arguments but got {}",
          frame.name,
          frame.params.len(),
          frame.args.len()
        ),
        frame.span,
      ));
    }
    let values = self.eval_args(frame.args, context, depth)?;
    let locals = frame.params.iter().cloned().zip(values).collect();
    self.locals.push(locals);
    let result = self.eval(frame.body, context, depth + 1);
    self.locals.pop();
    result
  }

  fn local_value(&self, name: &str) -> Option<&Value> {
    self.locals.iter().rev().find_map(|locals| locals.get(name))
  }

  fn eval_member(
    &mut self,
    value: Value,
    name: &str,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    match value {
      Value::Object(values) => values
        .get(name)
        .ok_or_else(|| EvalError::new(format!("missing object member {name}"), span))
        .and_then(|value| self.clone_admitted(value, span)),
      other => Err(EvalError::new(
        format!("cannot read member {name} from {}", other.type_name()),
        span,
      )),
    }
  }

  fn eval_unary(
    &mut self,
    op: UnaryOp,
    value: Value,
    registry: &DynamicRegistry,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    let result = if let Some(entry) = registry.unary_ops.get(&op) {
      (entry.handler)(value).map_err(|error| EvalError { span, ..error })?
    } else {
      match (op, value) {
        (UnaryOp::Not, Value::Bool(value)) => Value::Bool(!value),
        (UnaryOp::Neg, Value::Int(value)) => value
          .checked_neg()
          .map(Value::Int)
          .ok_or_else(|| EvalError::new("integer negation overflowed", span))?,
        (UnaryOp::Neg, Value::Float(value)) => Value::Float(-value),
        (op, value) => {
          return Err(EvalError::new(
            format!(
              "operator {} does not accept {}",
              op.as_str(),
              value.type_name()
            ),
            span,
          ));
        }
      }
    };
    self.admit(result, span)
  }

  fn eval_binary(
    &mut self,
    left: &VerifiedExpression,
    op: BinaryOp,
    right: &VerifiedExpression,
    context: &dyn RuntimeContext,
    depth: usize,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    let left_value = self.eval(left, context, depth + 1)?;
    match op {
      BinaryOp::And => {
        let left_bool = expect_bool(left_value, span)?;
        if !left_bool {
          return self.admit(Value::Bool(false), span);
        }
        let right_bool = expect_bool(self.eval(right, context, depth + 1)?, span)?;
        self.admit(Value::Bool(right_bool), span)
      }
      BinaryOp::Or => {
        let left_bool = expect_bool(left_value, span)?;
        if left_bool {
          return self.admit(Value::Bool(true), span);
        }
        let right_bool = expect_bool(self.eval(right, context, depth + 1)?, span)?;
        self.admit(Value::Bool(right_bool), span)
      }
      _ => {
        let right_value = self.eval(right, context, depth + 1)?;
        let result = if let Some(entry) = context.registry().binary_ops.get(&op) {
          (entry.handler)(left_value, right_value).map_err(|error| EvalError { span, ..error })?
        } else {
          self.eval_builtin_binary(left_value, op, right_value, span)?
        };
        self.admit(result, span)
      }
    }
  }

  fn eval_builtin_binary(
    &self,
    left: Value,
    op: BinaryOp,
    right: Value,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    match op {
      BinaryOp::Eq => Ok(Value::Bool(left == right)),
      BinaryOp::Ne => Ok(Value::Bool(left != right)),
      BinaryOp::Add => add_values(left, right, span, self.limits.max_string_bytes),
      BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
        numeric_arithmetic(left, op, right, span)
      }
      BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
        compare_values(left, op, right, span)
      }
      BinaryOp::And | BinaryOp::Or => Err(EvalError::new("internal boolean dispatch error", span)),
    }
  }

  fn checked_string(&mut self, value: String, span: SourceSpan) -> Result<Value, EvalError> {
    if value.len() > self.limits.max_string_bytes {
      Err(EvalError::new("string byte limit exceeded", span))
    } else {
      self.admit(Value::String(value), span)
    }
  }

  fn clone_admitted(&mut self, value: &Value, span: SourceSpan) -> Result<Value, EvalError> {
    let metrics = validate_value(value, self.resource_limits, span)?;
    self.charge(metrics, span)?;
    Ok(value.clone())
  }

  fn admit(&mut self, value: Value, span: SourceSpan) -> Result<Value, EvalError> {
    let metrics = match validate_value(&value, self.resource_limits, span) {
      Ok(metrics) => metrics,
      Err(error) => {
        value.drain_iteratively();
        return Err(error);
      }
    };
    if let Err(error) = self.charge(metrics, span) {
      value.drain_iteratively();
      return Err(error);
    }
    Ok(value)
  }

  fn charge(&mut self, metrics: ValueMetrics, span: SourceSpan) -> Result<(), EvalError> {
    self.processed_bytes = self
      .processed_bytes
      .checked_add(metrics.bytes)
      .ok_or_else(|| EvalError::new("runtime value byte counter overflowed", span))?;
    if self.processed_bytes > self.resource_limits.max_total_value_bytes {
      Err(EvalError::new(
        "runtime cumulative value byte limit exceeded",
        span,
      ))
    } else {
      Ok(())
    }
  }

  fn call_context(&self, span: SourceSpan) -> RuntimeCallContext<'_> {
    RuntimeCallContext::new(self.program.profile(), self.program.regex_cache(), span)
  }
}
