mod capability_check;
mod context;
mod defaults;
mod evaluator;
mod json;
mod operators;
mod pattern_sets;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::parser::{BinaryOp, SourceSpan, UnaryOp};
use crate::sema::VerifiedProgram;

use crate::compile::{
  CapabilityKind, CapabilityMeta, CapabilityTicket, CompiledExpression, RuntimeSchema,
};
use crate::value::{
  DEFAULT_MAX_VALUE_BYTES, DEFAULT_MAX_VALUE_ITEMS, DEFAULT_MAX_VALUE_NODES, MAX_VALUE_DEPTH,
  Value, ValueMetrics,
};
use capability_check::verify_runtime_capabilities;
pub use context::RuntimeCallContext;
pub use defaults::default_registry;
use json::convert_json_with_limits;
use operators::{binary_op_from_name, unary_op_from_name};
pub use pattern_sets::{
  RuntimePatternSetConfig, RuntimePatternSetError, RuntimePatternSetKind, RuntimePatternSetLimits,
  RuntimePatternSets, oxirule_pattern_set_registry, register_oxirule_pattern_set_methods,
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

/// Limits for values admitted to and produced by the runtime.
///
/// Each individual graph is checked before it is cloned, compared, or passed
/// to a handler. `max_total_value_bytes` is charged for every graph the
/// evaluator processes or returns, which bounds repeated accesses as well as
/// intermediate results. Limits are inclusive: a value exactly at a limit is
/// accepted. A zero graph limit rejects every root value deterministically.
/// Value depth has a hard ceiling of 128 even when a larger custom value is
/// supplied, keeping clone, comparison, serialization, and teardown paths
/// within a stack-safe envelope.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RuntimeResourceLimits {
  pub max_value_depth: usize,
  pub max_value_nodes: usize,
  pub max_value_items: usize,
  pub max_value_bytes: usize,
  pub max_total_value_bytes: usize,
}

impl Default for RuntimeResourceLimits {
  fn default() -> Self {
    Self {
      max_value_depth: MAX_VALUE_DEPTH,
      max_value_nodes: DEFAULT_MAX_VALUE_NODES,
      max_value_items: DEFAULT_MAX_VALUE_ITEMS,
      max_value_bytes: DEFAULT_MAX_VALUE_BYTES,
      max_total_value_bytes: 64 * 1024 * 1024,
    }
  }
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

  /// Borrow a host value when possible so the evaluator can validate it before
  /// cloning it. Existing contexts only need to implement `get_variable`.
  fn get_variable_borrowed(&self, _name: &str) -> Option<&Value> {
    None
  }

  fn registry(&self) -> &DynamicRegistry;
}

#[derive(Clone)]
pub struct MapRuntime {
  variables: BTreeMap<String, Value>,
  registry: DynamicRegistry,
}

impl Drop for MapRuntime {
  fn drop(&mut self) {
    for value in std::mem::take(&mut self.variables).into_values() {
      value.drain_iteratively();
    }
  }
}

impl MapRuntime {
  pub fn new(variables: BTreeMap<String, Value>, registry: DynamicRegistry) -> Self {
    Self {
      variables,
      registry,
    }
  }

  /// Construct a map runtime after checking the complete host binding object
  /// against `resource_limits`.
  pub fn try_new_with_limits(
    variables: BTreeMap<String, Value>,
    registry: DynamicRegistry,
    resource_limits: RuntimeResourceLimits,
  ) -> Result<Self, EvalError> {
    let bindings = Value::Object(variables);
    if let Err(error) = validate_value(&bindings, resource_limits, None, SourceSpan::default()) {
      bindings.drain_iteratively();
      return Err(error);
    }
    match bindings {
      Value::Object(variables) => Ok(Self::new(variables, registry)),
      other => {
        other.drain_iteratively();
        Err(EvalError::new(
          "runtime binding admission lost its object wrapper",
          SourceSpan::default(),
        ))
      }
    }
  }

  pub fn from_json_bindings(bindings: serde_json::Value) -> Result<Self, EvalError> {
    Self::from_json_bindings_with_limits(bindings, RuntimeResourceLimits::default())
  }

  pub fn from_json_bindings_with_limits(
    bindings: serde_json::Value,
    resource_limits: RuntimeResourceLimits,
  ) -> Result<Self, EvalError> {
    let value = convert_json_with_limits(bindings, resource_limits)?;
    let variables = match value {
      Value::Object(variables) => variables,
      other => {
        other.drain_iteratively();
        return Err(EvalError::new(
          "bindings must be a JSON object",
          SourceSpan::default(),
        ));
      }
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

  fn get_variable_borrowed(&self, name: &str) -> Option<&Value> {
    self.variables.get(name)
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
  evaluate_with_resource_limits(
    expression,
    context,
    limits,
    RuntimeResourceLimits::default(),
  )
}

pub fn evaluate_with_resource_limits(
  expression: &CompiledExpression,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
  resource_limits: RuntimeResourceLimits,
) -> Result<Value, EvalError> {
  evaluate_verified_with_resource_limits(
    expression.verified_program(),
    context,
    limits,
    resource_limits,
  )
}

pub fn evaluate_verified(
  program: &VerifiedProgram,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
) -> Result<Value, EvalError> {
  evaluate_verified_with_resource_limits(program, context, limits, RuntimeResourceLimits::default())
}

pub fn evaluate_verified_with_resource_limits(
  program: &VerifiedProgram,
  context: &dyn RuntimeContext,
  limits: EvalLimits,
  resource_limits: RuntimeResourceLimits,
) -> Result<Value, EvalError> {
  verify_runtime_capabilities(program, context.registry())?;
  evaluator::evaluate(program, context, limits, resource_limits)
}

pub(super) fn validate_value(
  value: &Value,
  limits: RuntimeResourceLimits,
  max_string_bytes: Option<usize>,
  span: SourceSpan,
) -> Result<ValueMetrics, EvalError> {
  let mut metrics = ValueMetrics::default();
  let mut pending = vec![(value, 1_usize)];

  while let Some((value, depth)) = pending.pop() {
    metrics.nodes = checked_value_metric(metrics.nodes, 1, "node", span)?;
    if metrics.nodes > limits.max_value_nodes {
      return Err(EvalError::new("value graph node limit exceeded", span));
    }
    metrics.bytes = checked_value_metric(metrics.bytes, 1, "byte", span)?;
    if metrics.bytes > limits.max_value_bytes {
      return Err(EvalError::new("value graph byte limit exceeded", span));
    }
    metrics.depth = metrics.depth.max(depth);
    if metrics.depth > limits.max_value_depth.min(MAX_VALUE_DEPTH) {
      return Err(EvalError::new("value graph depth limit exceeded", span));
    }

    match value {
      Value::String(value) => {
        if max_string_bytes.is_some_and(|limit| value.len() > limit) {
          return Err(EvalError::new("string byte limit exceeded", span));
        }
        metrics.bytes = checked_value_metric(metrics.bytes, value.len(), "byte", span)?;
        if metrics.bytes > limits.max_value_bytes {
          return Err(EvalError::new("value graph byte limit exceeded", span));
        }
      }
      Value::Array(values) => {
        let Some(child_depth) = reserve_value_children(
          &mut metrics,
          pending.len(),
          values.len(),
          depth,
          limits,
          span,
        )?
        else {
          continue;
        };
        pending
          .try_reserve(values.len())
          .map_err(|_| EvalError::new("value graph traversal allocation failed", span))?;
        pending.extend(values.iter().rev().map(|value| (value, child_depth)));
      }
      Value::Object(values) => {
        let Some(child_depth) = reserve_value_children(
          &mut metrics,
          pending.len(),
          values.len(),
          depth,
          limits,
          span,
        )?
        else {
          continue;
        };
        for key in values.keys() {
          metrics.bytes = checked_value_metric(metrics.bytes, key.len(), "byte", span)?;
          if metrics.bytes > limits.max_value_bytes {
            return Err(EvalError::new("value graph byte limit exceeded", span));
          }
        }
        pending
          .try_reserve(values.len())
          .map_err(|_| EvalError::new("value graph traversal allocation failed", span))?;
        pending.extend(values.iter().rev().map(|(_, value)| (value, child_depth)));
      }
      Value::Float(value) if !value.is_finite() => {
        return Err(EvalError::new(
          "runtime value contains a non-finite float",
          span,
        ));
      }
      Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => {}
    }
  }

  Ok(metrics)
}

fn reserve_value_children(
  metrics: &mut ValueMetrics,
  pending: usize,
  children: usize,
  depth: usize,
  limits: RuntimeResourceLimits,
  span: SourceSpan,
) -> Result<Option<usize>, EvalError> {
  metrics.items = checked_value_metric(metrics.items, children, "item", span)?;
  if metrics.items > limits.max_value_items {
    return Err(EvalError::new("value graph item limit exceeded", span));
  }
  if children == 0 {
    return Ok(None);
  }
  let child_depth = depth
    .checked_add(1)
    .ok_or_else(|| EvalError::new("value graph depth counter overflowed", span))?;
  if child_depth > limits.max_value_depth.min(MAX_VALUE_DEPTH) {
    return Err(EvalError::new("value graph depth limit exceeded", span));
  }
  let scheduled = metrics
    .nodes
    .checked_add(pending)
    .and_then(|total| total.checked_add(children))
    .ok_or_else(|| EvalError::new("value graph node counter overflowed", span))?;
  if scheduled > limits.max_value_nodes {
    return Err(EvalError::new("value graph node limit exceeded", span));
  }
  let minimum_bytes = metrics
    .bytes
    .checked_add(pending)
    .and_then(|total| total.checked_add(children))
    .ok_or_else(|| EvalError::new("value graph byte counter overflowed", span))?;
  if minimum_bytes > limits.max_value_bytes {
    return Err(EvalError::new("value graph byte limit exceeded", span));
  }
  Ok(Some(child_depth))
}

fn checked_value_metric(
  current: usize,
  added: usize,
  counter: &str,
  span: SourceSpan,
) -> Result<usize, EvalError> {
  current
    .checked_add(added)
    .ok_or_else(|| EvalError::new(format!("value graph {counter} counter overflowed"), span))
}
