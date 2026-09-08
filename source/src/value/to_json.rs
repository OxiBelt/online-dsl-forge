use std::collections::BTreeMap;

use super::{
  Value, ValueConversionError, ValueGraphLimits, ValueGraphMeter, drain_json_iteratively,
};

enum ValueJsonConversionWork {
  Value(Value, usize),
  FinishArray(usize),
  FinishObject(usize, Vec<String>),
}
pub(super) struct ValueJsonConverter {
  meter: ValueGraphMeter,
  pending: Vec<ValueJsonConversionWork>,
  pending_values: usize,
  json_values: Vec<serde_json::Value>,
  current: Option<Value>,
}
impl ValueJsonConverter {
  pub(super) fn convert(
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
