mod capability_check;
mod context;
mod defaults;
mod operators;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::parser::{BinaryOp, SourceSpan, UnaryOp};
use crate::sema::{VerifiedExprKindRef, VerifiedExpression, VerifiedProgram};

use crate::compile::{
  CapabilityKind, CapabilityMeta, CapabilityTicket, CompiledExpression, RuntimeSchema,
};
use crate::value::Value;
use capability_check::verify_runtime_capabilities;
pub use context::RuntimeCallContext;
pub use defaults::default_registry;
use operators::{
  add_values, binary_op_from_name, compare_values, expect_bool, numeric_arithmetic,
  unary_op_from_name,
};

type FunctionHandler =
  Arc<dyn for<'a> Fn(RuntimeCallContext<'a>, &[Value]) -> Result<Value, EvalError> + Send + Sync>;
type MethodHandler = Arc<
  dyn for<'a> Fn(RuntimeCallContext<'a>, &Value, &[Value]) -> Result<Value, EvalError>
    + Send
    + Sync,
>;
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
  unary_ops: HashMap<UnaryOp, UnaryEntry>,
  binary_ops: HashMap<BinaryOp, BinaryEntry>,
}

#[derive(Clone)]
struct FunctionEntry {
  arity: usize,
  capability: CapabilityMeta,
  handler: FunctionHandler,
}

#[derive(Clone)]
struct MethodEntry {
  arity: usize,
  capability: CapabilityMeta,
  handler: MethodHandler,
}

#[derive(Clone)]
struct UnaryEntry {
  capability: CapabilityMeta,
  handler: UnaryHandler,
}

#[derive(Clone)]
struct BinaryEntry {
  capability: CapabilityMeta,
  handler: BinaryHandler,
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
    self.register_function_capability(CapabilityMeta::function(name, arity), handler)
  }

  pub fn register_function_with_context(
    &mut self,
    name: impl Into<String>,
    arity: usize,
    handler: impl for<'a> Fn(RuntimeCallContext<'a>, &[Value]) -> Result<Value, EvalError>
    + Send
    + Sync
    + 'static,
  ) -> &mut Self {
    self.register_function_capability_with_context(CapabilityMeta::function(name, arity), handler)
  }

  pub fn register_function_capability(
    &mut self,
    capability: CapabilityMeta,
    handler: impl Fn(&[Value]) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.register_function_capability_with_context(capability, move |_, args| handler(args))
  }

  pub fn register_function_capability_with_context(
    &mut self,
    capability: CapabilityMeta,
    handler: impl for<'a> Fn(RuntimeCallContext<'a>, &[Value]) -> Result<Value, EvalError>
    + Send
    + Sync
    + 'static,
  ) -> &mut Self {
    self
      .functions
      .entry(capability.name.clone())
      .or_default()
      .push(FunctionEntry {
        arity: capability.arity,
        capability,
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
    self.register_method_capability(CapabilityMeta::method(name, arity), handler)
  }

  pub fn register_method_with_context(
    &mut self,
    name: impl Into<String>,
    arity: usize,
    handler: impl for<'a> Fn(RuntimeCallContext<'a>, &Value, &[Value]) -> Result<Value, EvalError>
    + Send
    + Sync
    + 'static,
  ) -> &mut Self {
    self.register_method_capability_with_context(CapabilityMeta::method(name, arity), handler)
  }

  pub fn register_method_capability(
    &mut self,
    capability: CapabilityMeta,
    handler: impl Fn(&Value, &[Value]) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.register_method_capability_with_context(capability, move |_, receiver, args| {
      handler(receiver, args)
    })
  }

  pub fn register_method_capability_with_context(
    &mut self,
    capability: CapabilityMeta,
    handler: impl for<'a> Fn(RuntimeCallContext<'a>, &Value, &[Value]) -> Result<Value, EvalError>
    + Send
    + Sync
    + 'static,
  ) -> &mut Self {
    self
      .methods
      .entry(capability.name.clone())
      .or_default()
      .push(MethodEntry {
        arity: capability.arity,
        capability,
        handler: Arc::new(handler),
      });
    self
  }

  pub fn register_unary_operator(
    &mut self,
    op: UnaryOp,
    handler: impl Fn(Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.register_unary_operator_capability(CapabilityMeta::unary_operator(op), handler)
  }

  pub fn register_unary_operator_capability(
    &mut self,
    capability: CapabilityMeta,
    handler: impl Fn(Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    if let Some(op) = unary_op_from_name(&capability.name) {
      self.unary_ops.insert(
        op,
        UnaryEntry {
          capability,
          handler: Arc::new(handler),
        },
      );
    }
    self
  }

  pub fn register_binary_operator(
    &mut self,
    op: BinaryOp,
    handler: impl Fn(Value, Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    self.register_binary_operator_capability(CapabilityMeta::binary_operator(op), handler)
  }

  pub fn register_binary_operator_capability(
    &mut self,
    capability: CapabilityMeta,
    handler: impl Fn(Value, Value) -> Result<Value, EvalError> + Send + Sync + 'static,
  ) -> &mut Self {
    if let Some(op) = binary_op_from_name(&capability.name) {
      self.binary_ops.insert(
        op,
        BinaryEntry {
          capability,
          handler: Arc::new(handler),
        },
      );
    }
    self
  }

  pub fn schema(&self) -> RuntimeSchema {
    let mut schema = RuntimeSchema::new();
    for entries in self.functions.values() {
      for entry in entries {
        schema.add_function_capability(entry.capability.clone());
      }
    }
    for entries in self.methods.values() {
      for entry in entries {
        schema.add_method_capability(entry.capability.clone());
      }
    }
    for entry in self.unary_ops.values() {
      schema.add_unary_operator_capability(entry.capability.clone());
    }
    for entry in self.binary_ops.values() {
      schema.add_binary_operator_capability(entry.capability.clone());
    }
    schema
  }

  fn capability_for_ticket(&self, ticket: &CapabilityTicket) -> Option<CapabilityMeta> {
    match ticket.kind {
      CapabilityKind::Function => self
        .functions
        .get(&ticket.name)
        .and_then(|entries| entries.iter().find(|entry| entry.arity == ticket.arity))
        .map(|entry| entry.capability.clone()),
      CapabilityKind::Method => self
        .methods
        .get(&ticket.name)
        .and_then(|entries| entries.iter().find(|entry| entry.arity == ticket.arity))
        .map(|entry| entry.capability.clone()),
      CapabilityKind::UnaryOp => {
        let op = unary_op_from_name(&ticket.name)?;
        self
          .unary_ops
          .get(&op)
          .map(|entry| entry.capability.clone())
          .or_else(|| Some(CapabilityMeta::unary_operator(op)))
      }
      CapabilityKind::BinaryOp => {
        let op = binary_op_from_name(&ticket.name)?;
        self
          .binary_ops
          .get(&op)
          .map(|entry| entry.capability.clone())
          .or_else(|| Some(CapabilityMeta::binary_operator(op)))
      }
    }
  }

  fn call_function(
    &self,
    context: RuntimeCallContext<'_>,
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
    (entry.handler)(context, args).map_err(|error| EvalError { span, ..error })
  }

  fn call_method(
    &self,
    context: RuntimeCallContext<'_>,
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
    (entry.handler)(context, receiver, args).map_err(|error| EvalError { span, ..error })
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
  evaluate_verified(expression.verified_program(), context, limits)
}

pub fn evaluate_verified(
  program: &VerifiedProgram,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
) -> Result<Value, EvalError> {
  verify_runtime_capabilities(program, context.registry())?;
  let mut state = EvalState {
    limits,
    steps: 0,
    program,
  };
  state.eval(program.root(), context, 0)
}

struct EvalState<'a> {
  limits: EvalLimits,
  steps: usize,
  program: &'a VerifiedProgram,
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
      VerifiedExprKindRef::Null => Ok(Value::Null),
      VerifiedExprKindRef::Bool(value) => Ok(Value::Bool(value)),
      VerifiedExprKindRef::Int(value) => Ok(Value::Int(value)),
      VerifiedExprKindRef::Float(value) => Ok(Value::Float(value)),
      VerifiedExprKindRef::String(value) => self.checked_string(value.to_string(), span),
      VerifiedExprKindRef::Array(items) => self.eval_array(items, context, depth, span),
      VerifiedExprKindRef::Identifier(name) => context
        .get_variable(name)
        .ok_or_else(|| EvalError::new(format!("unknown variable {name}"), span)),
      VerifiedExprKindRef::Member { receiver, name } => {
        let value = self.eval(receiver, context, depth + 1)?;
        self.eval_member(value, name, span)
      }
      VerifiedExprKindRef::FunctionCall { name, args } => {
        let args = self.eval_args(args, context, depth)?;
        context
          .registry()
          .call_function(self.call_context(span), name, &args, span)
      }
      VerifiedExprKindRef::MethodCall {
        receiver,
        name,
        args,
      } => {
        let receiver = self.eval(receiver, context, depth + 1)?;
        let args = self.eval_args(args, context, depth)?;
        context
          .registry()
          .call_method(self.call_context(span), &receiver, name, &args, span)
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
    items
      .iter()
      .map(|item| self.eval(item, context, depth + 1))
      .collect::<Result<Vec<_>, _>>()
      .map(Value::Array)
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
    if let Some(entry) = registry.unary_ops.get(&op) {
      return (entry.handler)(value).map_err(|error| EvalError { span, ..error });
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
        if let Some(entry) = context.registry().binary_ops.get(&op) {
          return (entry.handler)(left_value, right_value)
            .map_err(|error| EvalError { span, ..error });
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

  fn call_context(&self, span: SourceSpan) -> RuntimeCallContext<'_> {
    RuntimeCallContext::new(self.program.profile(), self.program.regex_cache(), span)
  }
}
