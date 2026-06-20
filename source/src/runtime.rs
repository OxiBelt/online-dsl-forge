use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use online_dsl_forge_parser::{AstExpression, BinaryOp, ExprKind, SourceSpan, UnaryOp};

use crate::compile::{CompiledExpression, RuntimeSchema};
use crate::value::Value;

type FunctionHandler = Arc<dyn Fn(&[Value]) -> Result<Value, EvalError> + Send + Sync>;
type MethodHandler = Arc<dyn Fn(&Value, &[Value]) -> Result<Value, EvalError> + Send + Sync>;
type UnaryHandler = Arc<dyn Fn(Value) -> Result<Value, EvalError> + Send + Sync>;
type BinaryHandler = Arc<dyn Fn(Value, Value) -> Result<Value, EvalError> + Send + Sync>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EvalError {
  pub message: String,
  pub span: SourceSpan,
}

impl EvalError {
  pub fn new(message: impl Into<String>, span: SourceSpan) -> Self {
    Self {
      message: message.into(),
      span,
    }
  }
}

impl fmt::Display for EvalError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      formatter,
      "{} at {}..{}",
      self.message, self.span.start, self.span.end
    )
  }
}

impl Error for EvalError {}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EvalLimits {
  pub max_steps: usize,
  pub max_depth: usize,
  pub max_string_bytes: usize,
  pub max_array_items: usize,
}

impl Default for EvalLimits {
  fn default() -> Self {
    Self {
      max_steps: 10_000,
      max_depth: 128,
      max_string_bytes: 64 * 1024,
      max_array_items: 4096,
    }
  }
}

#[derive(Clone, Default)]
pub struct DynamicRegistry {
  functions: BTreeMap<String, Vec<FunctionEntry>>,
  methods: BTreeMap<String, Vec<MethodEntry>>,
  unary_ops: HashMap<UnaryOp, UnaryHandler>,
  binary_ops: HashMap<BinaryOp, BinaryHandler>,
}

#[derive(Clone)]
struct FunctionEntry {
  arity: usize,
  handler: FunctionHandler,
}

#[derive(Clone)]
struct MethodEntry {
  arity: usize,
  handler: MethodHandler,
}

impl DynamicRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn register_function(
    &mut self,
    name: impl Into<String>,
    arity: usize,
    handler: impl Fn(&[Value]) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self
      .functions
      .entry(name.into())
      .or_default()
      .push(FunctionEntry {
        arity,
        handler: Arc::new(handler),
      });
    self
  }

  pub fn register_method(
    &mut self,
    name: impl Into<String>,
    arity: usize,
    handler: impl Fn(&Value, &[Value]) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self
      .methods
      .entry(name.into())
      .or_default()
      .push(MethodEntry {
        arity,
        handler: Arc::new(handler),
      });
    self
  }

  pub fn register_unary_operator(
    &mut self,
    op: UnaryOp,
    handler: impl Fn(Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.unary_ops.insert(op, Arc::new(handler));
    self
  }

  pub fn register_binary_operator(
    &mut self,
    op: BinaryOp,
    handler: impl Fn(Value, Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.binary_ops.insert(op, Arc::new(handler));
    self
  }

  pub fn schema(&self) -> RuntimeSchema {
    let mut schema = RuntimeSchema::new();
    for (name, entries) in &self.functions {
      for entry in entries {
        schema.add_function(name.clone(), entry.arity);
      }
    }
    for (name, entries) in &self.methods {
      for entry in entries {
        schema.add_method(name.clone(), entry.arity);
      }
    }
    schema
  }

  fn call_function(
    &self,
    name: &str,
    args: &[Value],
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    let Some(entries) = self.functions.get(name) else {
      return Err(EvalError::new(format!("unknown function {name}"), span));
    };
    let Some(entry) = entries.iter().find(|entry| entry.arity == args.len()) else {
      return Err(EvalError::new(
        format!("function {name} does not accept {} arguments", args.len()),
        span,
      ));
    };
    (entry.handler)(args).map_err(|error| EvalError { span, ..error })
  }

  fn call_method(
    &self,
    receiver: &Value,
    name: &str,
    args: &[Value],
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    let Some(entries) = self.methods.get(name) else {
      return Err(EvalError::new(format!("unknown method {name}"), span));
    };
    let Some(entry) = entries.iter().find(|entry| entry.arity == args.len()) else {
      return Err(EvalError::new(
        format!("method {name} does not accept {} arguments", args.len()),
        span,
      ));
    };
    (entry.handler)(receiver, args).map_err(|error| EvalError { span, ..error })
  }
}

pub trait RuntimeContext {
  fn get_variable(&self, name: &str) -> Option<Value>;
  fn registry(&self) -> &DynamicRegistry;
}

#[derive(Clone)]
pub struct MapRuntime {
  variables: BTreeMap<String, Value>,
  registry: DynamicRegistry,
}

impl MapRuntime {
  pub fn new(variables: BTreeMap<String, Value>, registry: DynamicRegistry) -> Self {
    Self {
      variables,
      registry,
    }
  }

  pub fn from_json_bindings(bindings: serde_json::Value) -> Result<Self, EvalError> {
    let Value::Object(variables) = Value::try_from(bindings)
      .map_err(|error| EvalError::new(error.to_string(), SourceSpan::default()))?
    else {
      return Err(EvalError::new(
        "bindings must be a JSON object",
        SourceSpan::default(),
      ));
    };
    Ok(Self::new(variables, default_registry()))
  }

  pub fn schema(&self) -> RuntimeSchema {
    let mut schema = self.registry.schema();
    for name in self.variables.keys() {
      schema.add_variable(name.clone());
    }
    schema
  }
}

impl RuntimeContext for MapRuntime {
  fn get_variable(&self, name: &str) -> Option<Value> {
    self.variables.get(name).cloned()
  }

  fn registry(&self) -> &DynamicRegistry {
    &self.registry
  }
}

pub fn evaluate(
  expression: &CompiledExpression,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
) -> Result<Value, EvalError> {
  let mut state = EvalState { limits, steps: 0 };
  state.eval(expression.ast(), context, 0)
}

struct EvalState {
  limits: EvalLimits,
  steps: usize,
}

impl EvalState {
  fn eval(
    &mut self,
    expression: &AstExpression,
    context: &dyn RuntimeContext,
    depth: usize,
  ) -> Result<Value, EvalError> {
    self.step(expression.span)?;
    if depth > self.limits.max_depth {
      return Err(EvalError::new(
        "evaluation depth limit exceeded",
        expression.span,
      ));
    }

    match &expression.kind {
      ExprKind::Null => Ok(Value::Null),
      ExprKind::Bool { value } => Ok(Value::Bool(*value)),
      ExprKind::Int { value } => Ok(Value::Int(*value)),
      ExprKind::Float { value } => Ok(Value::Float(*value)),
      ExprKind::String { value } => self.checked_string(value.clone(), expression.span),
      ExprKind::Array { items } => self.eval_array(items, context, depth, expression.span),
      ExprKind::Identifier { name } => context
        .get_variable(name)
        .ok_or_else(|| EvalError::new(format!("unknown variable {name}"), expression.span)),
      ExprKind::Member { receiver, name } => {
        let value = self.eval(receiver, context, depth + 1)?;
        self.eval_member(value, name, expression.span)
      }
      ExprKind::FunctionCall { name, args } => {
        let args = self.eval_args(args, context, depth)?;
        context
          .registry()
          .call_function(name, &args, expression.span)
      }
      ExprKind::MethodCall {
        receiver,
        name,
        args,
      } => {
        let receiver = self.eval(receiver, context, depth + 1)?;
        let args = self.eval_args(args, context, depth)?;
        context
          .registry()
          .call_method(&receiver, name, &args, expression.span)
      }
      ExprKind::Unary { op, expr } => {
        let value = self.eval(expr, context, depth + 1)?;
        self.eval_unary(*op, value, context.registry(), expression.span)
      }
      ExprKind::Binary { left, op, right } => {
        self.eval_binary(left, *op, right, context, depth, expression.span)
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

  fn eval_array(
    &mut self,
    items: &[AstExpression],
    context: &dyn RuntimeContext,
    depth: usize,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    if items.len() > self.limits.max_array_items {
      return Err(EvalError::new("array item limit exceeded", span));
    }
    items
      .iter()
      .map(|item| self.eval(item, context, depth + 1))
      .collect::<Result<Vec<_>, _>>()
      .map(Value::Array)
  }

  fn eval_args(
    &mut self,
    args: &[AstExpression],
    context: &dyn RuntimeContext,
    depth: usize,
  ) -> Result<Vec<Value>, EvalError> {
    args
      .iter()
      .map(|arg| self.eval(arg, context, depth + 1))
      .collect()
  }

  fn eval_member(&self, value: Value, name: &str, span: SourceSpan) -> Result<Value, EvalError> {
    match value {
      Value::Object(values) => values
        .get(name)
        .cloned()
        .ok_or_else(|| EvalError::new(format!("missing object member {name}"), span)),
      other => Err(EvalError::new(
        format!("cannot read member {name} from {}", other.type_name()),
        span,
      )),
    }
  }

  fn eval_unary(
    &self,
    op: UnaryOp,
    value: Value,
    registry: &DynamicRegistry,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    if let Some(handler) = registry.unary_ops.get(&op) {
      return handler(value).map_err(|error| EvalError { span, ..error });
    }
    match (op, value) {
      (UnaryOp::Not, Value::Bool(value)) => Ok(Value::Bool(!value)),
      (UnaryOp::Neg, Value::Int(value)) => value
        .checked_neg()
        .map(Value::Int)
        .ok_or_else(|| EvalError::new("integer negation overflowed", span)),
      (UnaryOp::Neg, Value::Float(value)) => Ok(Value::Float(-value)),
      (op, value) => Err(EvalError::new(
        format!(
          "operator {} does not accept {}",
          op.as_str(),
          value.type_name()
        ),
        span,
      )),
    }
  }

  fn eval_binary(
    &mut self,
    left: &AstExpression,
    op: BinaryOp,
    right: &AstExpression,
    context: &dyn RuntimeContext,
    depth: usize,
    span: SourceSpan,
  ) -> Result<Value, EvalError> {
    let left_value = self.eval(left, context, depth + 1)?;
    match op {
      BinaryOp::And => {
        let left_bool = expect_bool(left_value, span)?;
        if !left_bool {
          return Ok(Value::Bool(false));
        }
        let right_bool = expect_bool(self.eval(right, context, depth + 1)?, span)?;
        Ok(Value::Bool(right_bool))
      }
      BinaryOp::Or => {
        let left_bool = expect_bool(left_value, span)?;
        if left_bool {
          return Ok(Value::Bool(true));
        }
        let right_bool = expect_bool(self.eval(right, context, depth + 1)?, span)?;
        Ok(Value::Bool(right_bool))
      }
      _ => {
        let right_value = self.eval(right, context, depth + 1)?;
        if let Some(handler) = context.registry().binary_ops.get(&op) {
          return handler(left_value, right_value).map_err(|error| EvalError { span, ..error });
        }
        self.eval_builtin_binary(left_value, op, right_value, span)
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

  fn checked_string(&self, value: String, span: SourceSpan) -> Result<Value, EvalError> {
    if value.len() > self.limits.max_string_bytes {
      Err(EvalError::new("string byte limit exceeded", span))
    } else {
      Ok(Value::String(value))
    }
  }
}

fn expect_bool(value: Value, span: SourceSpan) -> Result<bool, EvalError> {
  match value {
    Value::Bool(value) => Ok(value),
    other => Err(EvalError::new(
      format!("expected bool, got {}", other.type_name()),
      span,
    )),
  }
}

fn add_values(
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

fn numeric_arithmetic(
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

fn compare_values(
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

fn type_error(op: &str, left: &Value, right: &Value, span: SourceSpan) -> EvalError {
  EvalError::new(
    format!(
      "operator {op} does not accept {} and {}",
      left.type_name(),
      right.type_name()
    ),
    span,
  )
}

pub fn default_registry() -> DynamicRegistry {
  let mut registry = DynamicRegistry::new();
  registry.register_function("len", 1, |args| value_len(&args[0]));
  registry.register_method("len", 0, |receiver, _| value_len(receiver));
  registry.register_method("contains", 1, contains_value);
  registry.register_method("contains_key", 1, contains_key);
  registry.register_method("starts_with", 1, string_method("starts_with"));
  registry.register_method("ends_with", 1, string_method("ends_with"));
  registry.register_method("lower_ascii", 0, |receiver, _| match receiver {
    Value::String(value) => Ok(Value::String(value.to_ascii_lowercase())),
    other => Err(EvalError::new(
      format!("lower_ascii requires string, got {}", other.type_name()),
      SourceSpan::default(),
    )),
  });
  registry.register_method("upper_ascii", 0, |receiver, _| match receiver {
    Value::String(value) => Ok(Value::String(value.to_ascii_uppercase())),
    other => Err(EvalError::new(
      format!("upper_ascii requires string, got {}", other.type_name()),
      SourceSpan::default(),
    )),
  });
  registry
}

fn value_len(value: &Value) -> Result<Value, EvalError> {
  let len = match value {
    Value::String(value) => value.len(),
    Value::Array(value) => value.len(),
    Value::Object(value) => value.len(),
    other => {
      return Err(EvalError::new(
        format!(
          "len requires string, array, or object, got {}",
          other.type_name()
        ),
        SourceSpan::default(),
      ));
    }
  };
  i64::try_from(len)
    .map(Value::Int)
    .map_err(|_| EvalError::new("length does not fit in i64", SourceSpan::default()))
}

fn contains_value(receiver: &Value, args: &[Value]) -> Result<Value, EvalError> {
  match (receiver, &args[0]) {
    (Value::String(receiver), Value::String(needle)) => Ok(Value::Bool(receiver.contains(needle))),
    (Value::Array(items), needle) => Ok(Value::Bool(items.iter().any(|item| item == needle))),
    (other, _) => Err(EvalError::new(
      format!(
        "contains requires string or array, got {}",
        other.type_name()
      ),
      SourceSpan::default(),
    )),
  }
}

fn contains_key(receiver: &Value, args: &[Value]) -> Result<Value, EvalError> {
  match (receiver, &args[0]) {
    (Value::Object(values), Value::String(key)) => Ok(Value::Bool(values.contains_key(key))),
    (Value::Object(_), other) => Err(EvalError::new(
      format!(
        "contains_key requires string key, got {}",
        other.type_name()
      ),
      SourceSpan::default(),
    )),
    (other, _) => Err(EvalError::new(
      format!("contains_key requires object, got {}", other.type_name()),
      SourceSpan::default(),
    )),
  }
}

fn string_method(
  method: &'static str,
) -> impl Fn(&Value, &[Value]) -> Result<Value, EvalError> + Send + Sync + 'static {
  move |receiver, args| match (receiver, &args[0]) {
    (Value::String(receiver), Value::String(arg)) => {
      let value = match method {
        "starts_with" => receiver.starts_with(arg),
        "ends_with" => receiver.ends_with(arg),
        _ => false,
      };
      Ok(Value::Bool(value))
    }
    (Value::String(_), other) => Err(EvalError::new(
      format!(
        "{method} requires string argument, got {}",
        other.type_name()
      ),
      SourceSpan::default(),
    )),
    (other, _) => Err(EvalError::new(
      format!(
        "{method} requires string receiver, got {}",
        other.type_name()
      ),
      SourceSpan::default(),
    )),
  }
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeMap;

  use crate::{CompileOptions, RuntimeSchema, compile_expression, parse_expression};

  use super::{EvalLimits, MapRuntime, Value, default_registry, evaluate};

  #[test]
  fn evaluates_default_runtime_expression() {
    let ast = parse_expression("score + 1 >= 10 && name.starts_with('pi')")
      .expect("expression should parse");
    let mut variables = BTreeMap::new();
    variables.insert("score".to_string(), Value::Int(9));
    variables.insert("name".to_string(), Value::String("piquark".to_string()));
    let runtime = MapRuntime::new(variables, default_registry());
    let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
      .expect("expression should compile");
    let value = evaluate(&compiled, &runtime, EvalLimits::default()).expect("eval should pass");
    assert_eq!(value, Value::Bool(true));
  }

  #[test]
  fn short_circuits_boolean_and() {
    let ast = parse_expression("false && missing").expect("expression should parse");
    let compiled = compile_expression(
      &ast,
      &RuntimeSchema::new(),
      CompileOptions {
        allow_unknown_variables: true,
        allow_unknown_functions: false,
        allow_unknown_methods: false,
      },
    )
    .expect("expression should compile");
    let runtime = MapRuntime::new(BTreeMap::new(), default_registry());
    let value = evaluate(&compiled, &runtime, EvalLimits::default()).expect("eval should pass");
    assert_eq!(value, Value::Bool(false));
  }
}
