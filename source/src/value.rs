use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
  Null,
  Bool(bool),
  Int(i64),
  Float(
    #[serde(
      deserialize_with = "crate::serde_support::deserialize_f64",
      serialize_with = "crate::serde_support::serialize_f64"
    )]
    f64,
  ),
  String(String),
  Array(Vec<Value>),
  Object(BTreeMap<String, Value>),
}

/// Iteratively collected properties of a [`Value`] graph.
///
/// This intentionally counts logical payload bytes rather than allocator
/// capacity. Every node contributes one byte for its logical tag; strings and
/// object keys contribute their UTF-8 byte lengths as payload.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct ValueMetrics {
  pub depth: usize,
  pub nodes: usize,
  pub items: usize,
  pub bytes: usize,
}

pub(crate) const MAX_VALUE_DEPTH: usize = 128;
pub(crate) const DEFAULT_MAX_VALUE_NODES: usize = 262_144;
pub(crate) const DEFAULT_MAX_VALUE_ITEMS: usize = 262_144;
pub(crate) const DEFAULT_MAX_VALUE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct ValueGraphLimits {
  pub max_depth: usize,
  pub max_nodes: usize,
  pub max_items: usize,
  pub max_bytes: usize,
}

impl Default for ValueGraphLimits {
  fn default() -> Self {
    Self {
      max_depth: MAX_VALUE_DEPTH,
      max_nodes: DEFAULT_MAX_VALUE_NODES,
      max_items: DEFAULT_MAX_VALUE_ITEMS,
      max_bytes: DEFAULT_MAX_VALUE_BYTES,
    }
  }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ValueConversionError {
  message: String,
}

impl ValueConversionError {
  fn new(message: impl Into<String>) -> Self {
    Self {
      message: message.into(),
    }
  }
}

impl fmt::Display for ValueConversionError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&self.message)
  }
}

impl std::error::Error for ValueConversionError {}

impl Value {
  pub fn type_name(&self) -> &'static str {
    match self {
      Self::Null => "null",
      Self::Bool(_) => "bool",
      Self::Int(_) => "int",
      Self::Float(_) => "float",
      Self::String(_) => "string",
      Self::Array(_) => "array",
      Self::Object(_) => "object",
    }
  }

  pub fn as_bool(&self) -> Option<bool> {
    match self {
      Self::Bool(value) => Some(*value),
      _ => None,
    }
  }

  pub fn is_number(&self) -> bool {
    matches!(self, Self::Int(_) | Self::Float(_))
  }

  /// Convert this value into JSON with the default value-graph limits.
  ///
  /// The iterative conversion rejects non-finite floats and over-limit graphs,
  /// then drains rejected owned inputs iteratively.
  ///
  /// `serde_json::Value::from` remains available for compatibility, but it
  /// yields JSON `null` when this fallible conversion rejects an input.
  pub fn try_into_json(self) -> Result<serde_json::Value, ValueConversionError> {
    ValueJsonConverter::convert(self, ValueGraphLimits::default())
  }

  /// Consume a graph without recursively dropping nested containers.
  ///
  /// Runtime rejection paths use this for untrusted owned values whose depth
  /// may have exceeded the admission limit.
  pub(crate) fn drain_iteratively(self) {
    let mut pending = vec![ValueDrainWork::Value(self)];
    while let Some(work) = pending.pop() {
      match work {
        ValueDrainWork::Value(Self::Array(values)) => {
          pending.push(ValueDrainWork::Array(values.into_iter()));
        }
        ValueDrainWork::Value(Self::Object(values)) => {
          pending.push(ValueDrainWork::Object(values.into_values()));
        }
        ValueDrainWork::Array(mut values) => {
          if let Some(value) = values.next() {
            pending.push(ValueDrainWork::Array(values));
            pending.push(ValueDrainWork::Value(value));
          }
        }
        ValueDrainWork::Object(mut values) => {
          if let Some(value) = values.next() {
            pending.push(ValueDrainWork::Object(values));
            pending.push(ValueDrainWork::Value(value));
          }
        }
        ValueDrainWork::Value(
          Self::Null | Self::Bool(_) | Self::Int(_) | Self::Float(_) | Self::String(_),
        ) => {}
      }
    }
  }
}

enum ValueDrainWork {
  Value(Value),
  Array(std::vec::IntoIter<Value>),
  Object(std::collections::btree_map::IntoValues<String, Value>),
}
impl TryFrom<serde_json::Value> for Value {
  type Error = ValueConversionError;

  fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
    convert_json_with_limits(value, ValueGraphLimits::default())
  }
}

enum JsonConversionWork {
  Value(serde_json::Value, usize),
  FinishArray(usize),
  FinishObject(usize, Vec<String>),
}

struct ValueGraphMeter {
  limits: ValueGraphLimits,
  metrics: ValueMetrics,
}

impl ValueGraphMeter {
  fn new(mut limits: ValueGraphLimits) -> Self {
    limits.max_depth = limits.max_depth.min(MAX_VALUE_DEPTH);
    Self {
      limits,
      metrics: ValueMetrics::default(),
    }
  }
  fn add_node(&mut self, depth: usize) -> Result<(), ValueConversionError> {
    self.metrics.nodes = checked_add(self.metrics.nodes, 1, "node")?;
    self.metrics.bytes = checked_add(self.metrics.bytes, 1, "byte")?;
    self.metrics.depth = self.metrics.depth.max(depth);
    self.validate()
  }
  fn add_bytes(&mut self, bytes: usize) -> Result<(), ValueConversionError> {
    self.metrics.bytes = checked_add(self.metrics.bytes, bytes, "byte")?;
    self.validate()
  }
  fn reserve_children(
    &mut self,
    children: usize,
    depth: usize,
    pending_values: usize,
  ) -> Result<usize, ValueConversionError> {
    self.metrics.items = checked_add(self.metrics.items, children, "item")?;
    self.validate()?;
    let child_depth = depth
      .checked_add(1)
      .ok_or_else(|| ValueConversionError::new("value graph depth counter overflowed"))?;
    if child_depth > self.limits.max_depth {
      return Err(ValueConversionError::new(
        "value graph depth limit exceeded",
      ));
    }
    let scheduled = self
      .metrics
      .nodes
      .checked_add(pending_values)
      .and_then(|total| total.checked_add(children))
      .ok_or_else(|| ValueConversionError::new("value graph node counter overflowed"))?;
    if scheduled > self.limits.max_nodes {
      return Err(ValueConversionError::new("value graph node limit exceeded"));
    }
    let minimum_bytes = self
      .metrics
      .bytes
      .checked_add(pending_values)
      .and_then(|total| total.checked_add(children))
      .ok_or_else(|| ValueConversionError::new("value graph byte counter overflowed"))?;
    if minimum_bytes > self.limits.max_bytes {
      return Err(ValueConversionError::new("value graph byte limit exceeded"));
    }
    Ok(child_depth)
  }
  fn validate(&self) -> Result<(), ValueConversionError> {
    if self.metrics.depth > self.limits.max_depth {
      return Err(ValueConversionError::new(
        "value graph depth limit exceeded",
      ));
    }
    if self.metrics.nodes > self.limits.max_nodes {
      return Err(ValueConversionError::new("value graph node limit exceeded"));
    }
    if self.metrics.items > self.limits.max_items {
      return Err(ValueConversionError::new("value graph item limit exceeded"));
    }
    if self.metrics.bytes > self.limits.max_bytes {
      return Err(ValueConversionError::new("value graph byte limit exceeded"));
    }
    Ok(())
  }
}

struct JsonValueConverter {
  meter: ValueGraphMeter,
  pending: Vec<JsonConversionWork>,
  pending_values: usize,
  values: Vec<Value>,
  current: Option<serde_json::Value>,
}

impl JsonValueConverter {
  fn convert(
    value: serde_json::Value,
    limits: ValueGraphLimits,
  ) -> Result<Value, ValueConversionError> {
    let mut converter = Self {
      meter: ValueGraphMeter::new(limits),
      pending: vec![JsonConversionWork::Value(value, 1)],
      pending_values: 1,
      values: Vec::new(),
      current: None,
    };
    while let Some(work) = converter.pending.pop() {
      match work {
        JsonConversionWork::Value(value, depth) => {
          converter.pending_values = converter.pending_values.checked_sub(1).ok_or_else(|| {
            ValueConversionError::new("JSON conversion pending-value counter underflowed")
          })?;
          converter.convert_value(value, depth)?;
        }
        JsonConversionWork::FinishArray(start) => {
          let values = converter.values.split_off(start);
          converter.values.push(Value::Array(values));
        }
        JsonConversionWork::FinishObject(start, keys) => {
          let values = converter.values.split_off(start);
          converter
            .values
            .push(Value::Object(keys.into_iter().zip(values).collect()));
        }
      }
    }
    match converter.values.pop() {
      Some(value) if converter.values.is_empty() => Ok(value),
      Some(value) => {
        value.drain_iteratively();
        converter.reject(ValueConversionError::new(
          "JSON conversion produced an invalid value stack",
        ))
      }
      None => converter.reject(ValueConversionError::new(
        "JSON conversion produced no value",
      )),
    }
  }

  fn convert_value(
    &mut self,
    value: serde_json::Value,
    depth: usize,
  ) -> Result<(), ValueConversionError> {
    self.current = Some(value);
    if let Err(error) = self.meter.add_node(depth) {
      return self.reject(error);
    }
    let Some(value) = self.current.take() else {
      return self.reject(ValueConversionError::new(
        "JSON conversion state is missing a value",
      ));
    };
    match value {
      serde_json::Value::Null => self.values.push(Value::Null),
      serde_json::Value::Bool(value) => self.values.push(Value::Bool(value)),
      serde_json::Value::Number(number) => {
        if let Some(value) = number.as_i64() {
          self.values.push(Value::Int(value));
        } else if !number.is_f64() {
          return self.reject(ValueConversionError::new(
            "JSON integer is outside the supported i64 range",
          ));
        } else if let Some(value) = number.as_f64() {
          self.values.push(Value::Float(value));
        } else {
          return self.reject(ValueConversionError::new("unsupported JSON number"));
        }
      }
      serde_json::Value::String(value) => {
        if let Err(error) = self.meter.add_bytes(value.len()) {
          return self.reject(error);
        }
        self.values.push(Value::String(value));
      }
      serde_json::Value::Array(values) => self.schedule_array(values, depth)?,
      serde_json::Value::Object(values) => self.schedule_object(values, depth)?,
    }
    Ok(())
  }

  fn schedule_array(
    &mut self,
    values: Vec<serde_json::Value>,
    depth: usize,
  ) -> Result<(), ValueConversionError> {
    if values.is_empty() {
      self.values.push(Value::Array(Vec::new()));
      return Ok(());
    }
    let child_depth = match self
      .meter
      .reserve_children(values.len(), depth, self.pending_values)
    {
      Ok(depth) => depth,
      Err(error) => {
        self.current = Some(serde_json::Value::Array(values));
        return self.reject(error);
      }
    };
    if let Err(error) = self.reserve_conversion_work(values.len()) {
      self.current = Some(serde_json::Value::Array(values));
      return self.reject(error);
    }
    let start = self.values.len();
    self.pending.push(JsonConversionWork::FinishArray(start));
    self.pending_values += values.len();
    self.pending.extend(
      values
        .into_iter()
        .rev()
        .map(|value| JsonConversionWork::Value(value, child_depth)),
    );
    Ok(())
  }

  fn schedule_object(
    &mut self,
    values: serde_json::Map<String, serde_json::Value>,
    depth: usize,
  ) -> Result<(), ValueConversionError> {
    if values.is_empty() {
      self.values.push(Value::Object(BTreeMap::new()));
      return Ok(());
    }
    let child_depth = match self
      .meter
      .reserve_children(values.len(), depth, self.pending_values)
    {
      Ok(depth) => depth,
      Err(error) => {
        self.current = Some(serde_json::Value::Object(values));
        return self.reject(error);
      }
    };
    let key_bytes = values
      .keys()
      .try_fold(0_usize, |total, key| total.checked_add(key.len()));
    let Some(key_bytes) = key_bytes else {
      self.current = Some(serde_json::Value::Object(values));
      return self.reject(ValueConversionError::new(
        "value graph byte counter overflowed",
      ));
    };
    if let Err(error) = self.meter.add_bytes(key_bytes) {
      self.current = Some(serde_json::Value::Object(values));
      return self.reject(error);
    }
    if let Err(error) = self.reserve_conversion_work(values.len()) {
      self.current = Some(serde_json::Value::Object(values));
      return self.reject(error);
    }
    let mut entries = Vec::new();
    let mut keys = Vec::new();
    if entries.try_reserve_exact(values.len()).is_err()
      || keys.try_reserve_exact(values.len()).is_err()
    {
      self.current = Some(serde_json::Value::Object(values));
      return self.reject(ValueConversionError::new(
        "JSON conversion allocation failed",
      ));
    }
    entries.extend(values);
    for (key, _) in &mut entries {
      keys.push(std::mem::take(key));
    }
    let start = self.values.len();
    self
      .pending
      .push(JsonConversionWork::FinishObject(start, keys));
    self.pending_values += entries.len();
    self.pending.extend(
      entries
        .into_iter()
        .rev()
        .map(|(_, value)| JsonConversionWork::Value(value, child_depth)),
    );
    Ok(())
  }

  fn reserve_conversion_work(&mut self, children: usize) -> Result<(), ValueConversionError> {
    self
      .pending
      .try_reserve(children.saturating_add(1))
      .map_err(|_| ValueConversionError::new("JSON conversion allocation failed"))?;
    self
      .values
      .try_reserve(children)
      .map_err(|_| ValueConversionError::new("JSON conversion allocation failed"))
  }

  fn reject<T>(&mut self, error: ValueConversionError) -> Result<T, ValueConversionError> {
    if let Some(value) = self.current.take() {
      drain_json_iteratively(value);
    }
    for work in std::mem::take(&mut self.pending) {
      if let JsonConversionWork::Value(value, _) = work {
        drain_json_iteratively(value);
      }
    }
    for value in std::mem::take(&mut self.values) {
      value.drain_iteratively();
    }
    Err(error)
  }
}

pub(crate) fn convert_json_with_limits(
  value: serde_json::Value,
  limits: ValueGraphLimits,
) -> Result<Value, ValueConversionError> {
  JsonValueConverter::convert(value, limits)
}

fn checked_add(current: usize, added: usize, counter: &str) -> Result<usize, ValueConversionError> {
  current
    .checked_add(added)
    .ok_or_else(|| ValueConversionError::new(format!("value graph {counter} counter overflowed")))
}

fn drain_json_iteratively(value: serde_json::Value) {
  let mut pending = vec![JsonDrainWork::Value(value)];
  while let Some(work) = pending.pop() {
    match work {
      JsonDrainWork::Value(serde_json::Value::Array(values)) => {
        pending.push(JsonDrainWork::Array(values.into_iter()));
      }
      JsonDrainWork::Value(serde_json::Value::Object(values)) => {
        pending.push(JsonDrainWork::Object(values.into_iter()));
      }
      JsonDrainWork::Array(mut values) => {
        if let Some(value) = values.next() {
          pending.push(JsonDrainWork::Array(values));
          pending.push(JsonDrainWork::Value(value));
        }
      }
      JsonDrainWork::Object(mut values) => {
        if let Some((_, value)) = values.next() {
          pending.push(JsonDrainWork::Object(values));
          pending.push(JsonDrainWork::Value(value));
        }
      }
      JsonDrainWork::Value(
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_),
      ) => {}
    }
  }
}
enum JsonDrainWork {
  Value(serde_json::Value),
  Array(std::vec::IntoIter<serde_json::Value>),
  Object(serde_json::map::IntoIter),
}
enum ValueJsonConversionWork {
  Value(Value, usize),
  FinishArray(usize),
  FinishObject(usize, Vec<String>),
}
struct ValueJsonConverter {
  meter: ValueGraphMeter,
  pending: Vec<ValueJsonConversionWork>,
  pending_values: usize,
  json_values: Vec<serde_json::Value>,
  current: Option<Value>,
}
impl ValueJsonConverter {
  fn convert(
    value: Value,
    limits: ValueGraphLimits,
  ) -> Result<serde_json::Value, ValueConversionError> {
    let mut converter = Self {
      meter: ValueGraphMeter::new(limits),
      pending: vec![ValueJsonConversionWork::Value(value, 1)],
      pending_values: 1,
      json_values: Vec::new(),
      current: None,
    };
    while let Some(work) = converter.pending.pop() {
      match work {
        ValueJsonConversionWork::Value(value, depth) => {
          let Some(pending_values) = converter.pending_values.checked_sub(1) else {
            return converter.reject(ValueConversionError::new(
              "value-to-JSON pending-value counter underflowed",
            ));
          };
          converter.pending_values = pending_values;
          converter.convert_value(value, depth)?;
        }
        ValueJsonConversionWork::FinishArray(start) => {
          let values = converter.json_values.split_off(start);
          converter.json_values.push(serde_json::Value::Array(values));
        }
        ValueJsonConversionWork::FinishObject(start, keys) => {
          let values = converter.json_values.split_off(start);
          converter.json_values.push(serde_json::Value::Object(
            keys.into_iter().zip(values).collect(),
          ));
        }
      }
    }
    match converter.json_values.pop() {
      Some(value) if converter.json_values.is_empty() => Ok(value),
      Some(value) => {
        drain_json_iteratively(value);
        converter.reject(ValueConversionError::new(
          "value-to-JSON conversion produced an invalid value stack",
        ))
      }
      None => converter.reject(ValueConversionError::new(
        "value-to-JSON conversion produced no value",
      )),
    }
  }
  fn convert_value(&mut self, value: Value, depth: usize) -> Result<(), ValueConversionError> {
    self.current = Some(value);
    if let Err(error) = self.meter.add_node(depth) {
      return self.reject(error);
    }
    let Some(value) = self.current.take() else {
      return self.reject(ValueConversionError::new(
        "value-to-JSON conversion state is missing a value",
      ));
    };
    match value {
      Value::Null => self.json_values.push(serde_json::Value::Null),
      Value::Bool(value) => self.json_values.push(serde_json::Value::Bool(value)),
      Value::Int(value) => self
        .json_values
        .push(serde_json::Value::Number(value.into())),
      Value::Float(value) => {
        let Some(value) = serde_json::Number::from_f64(value) else {
          return self.reject(ValueConversionError::new(
            "non-finite float cannot be represented in JSON",
          ));
        };
        self.json_values.push(serde_json::Value::Number(value));
      }
      Value::String(value) => {
        if let Err(error) = self.meter.add_bytes(value.len()) {
          return self.reject(error);
        }
        self.json_values.push(serde_json::Value::String(value));
      }
      Value::Array(values) => self.schedule_array(values, depth)?,
      Value::Object(values) => self.schedule_object(values, depth)?,
    }
    Ok(())
  }
  fn schedule_array(
    &mut self,
    values: Vec<Value>,
    depth: usize,
  ) -> Result<(), ValueConversionError> {
    if values.is_empty() {
      self.json_values.push(serde_json::Value::Array(Vec::new()));
      return Ok(());
    }
    let child_depth = match self
      .meter
      .reserve_children(values.len(), depth, self.pending_values)
    {
      Ok(depth) => depth,
      Err(error) => {
        self.current = Some(Value::Array(values));
        return self.reject(error);
      }
    };
    if let Err(error) = self.reserve_conversion_work(values.len()) {
      self.current = Some(Value::Array(values));
      return self.reject(error);
    }
    let start = self.json_values.len();
    self
      .pending
      .push(ValueJsonConversionWork::FinishArray(start));
    self.pending_values += values.len();
    self.pending.extend(
      values
        .into_iter()
        .rev()
        .map(|value| ValueJsonConversionWork::Value(value, child_depth)),
    );
    Ok(())
  }
  fn schedule_object(
    &mut self,
    values: BTreeMap<String, Value>,
    depth: usize,
  ) -> Result<(), ValueConversionError> {
    if values.is_empty() {
      self
        .json_values
        .push(serde_json::Value::Object(serde_json::Map::new()));
      return Ok(());
    }
    let child_depth = match self
      .meter
      .reserve_children(values.len(), depth, self.pending_values)
    {
      Ok(depth) => depth,
      Err(error) => {
        self.current = Some(Value::Object(values));
        return self.reject(error);
      }
    };
    let key_bytes = values
      .keys()
      .try_fold(0_usize, |total, key| total.checked_add(key.len()));
    let Some(key_bytes) = key_bytes else {
      self.current = Some(Value::Object(values));
      return self.reject(ValueConversionError::new(
        "value graph byte counter overflowed",
      ));
    };
    if let Err(error) = self.meter.add_bytes(key_bytes) {
      self.current = Some(Value::Object(values));
      return self.reject(error);
    }
    if let Err(error) = self.reserve_conversion_work(values.len()) {
      self.current = Some(Value::Object(values));
      return self.reject(error);
    }
    let mut entries = Vec::new();
    let mut keys = Vec::new();
    if entries.try_reserve_exact(values.len()).is_err()
      || keys.try_reserve_exact(values.len()).is_err()
    {
      self.current = Some(Value::Object(values));
      return self.reject(ValueConversionError::new(
        "value-to-JSON conversion allocation failed",
      ));
    }
    entries.extend(values);
    for (key, _) in &mut entries {
      keys.push(std::mem::take(key));
    }
    let start = self.json_values.len();
    self
      .pending
      .push(ValueJsonConversionWork::FinishObject(start, keys));
    self.pending_values += entries.len();
    self.pending.extend(
      entries
        .into_iter()
        .rev()
        .map(|(_, value)| ValueJsonConversionWork::Value(value, child_depth)),
    );
    Ok(())
  }
  fn reserve_conversion_work(&mut self, children: usize) -> Result<(), ValueConversionError> {
    let pending_work = children
      .checked_add(1)
      .ok_or_else(|| ValueConversionError::new("value-to-JSON work counter overflowed"))?;
    self
      .pending
      .try_reserve(pending_work)
      .map_err(|_| ValueConversionError::new("value-to-JSON conversion allocation failed"))?;
    self
      .json_values
      .try_reserve(children)
      .map_err(|_| ValueConversionError::new("value-to-JSON conversion allocation failed"))
  }
  fn reject<T>(&mut self, error: ValueConversionError) -> Result<T, ValueConversionError> {
    if let Some(value) = self.current.take() {
      value.drain_iteratively();
    }
    for work in std::mem::take(&mut self.pending) {
      if let ValueJsonConversionWork::Value(value, _) = work {
        value.drain_iteratively();
      }
    }
    for value in std::mem::take(&mut self.json_values) {
      drain_json_iteratively(value);
    }
    Err(error)
  }
}
impl From<Value> for serde_json::Value {
  fn from(value: Value) -> Self {
    // `From` cannot report malformed directly constructed values. Preserve its
    // infallible contract by producing JSON null after the fallible path has
    // iteratively drained a rejected input. Call `Value::try_into_json` when
    // the caller needs the rejection reason.
    match value.try_into_json() {
      Ok(value) => value,
      Err(_) => Self::Null,
    }
  }
}
