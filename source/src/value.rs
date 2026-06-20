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
  Float(f64),
  String(String),
  Array(Vec<Value>),
  Object(BTreeMap<String, Value>),
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
}

impl TryFrom<serde_json::Value> for Value {
  type Error = ValueConversionError;

  fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
    match value {
      serde_json::Value::Null => Ok(Self::Null),
      serde_json::Value::Bool(value) => Ok(Self::Bool(value)),
      serde_json::Value::Number(number) => {
        if let Some(value) = number.as_i64() {
          Ok(Self::Int(value))
        } else if let Some(value) = number.as_f64() {
          Ok(Self::Float(value))
        } else {
          Err(ValueConversionError::new("unsupported JSON number"))
        }
      }
      serde_json::Value::String(value) => Ok(Self::String(value)),
      serde_json::Value::Array(values) => values
        .into_iter()
        .map(Value::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map(Self::Array),
      serde_json::Value::Object(values) => values
        .into_iter()
        .map(|(key, value)| Value::try_from(value).map(|value| (key, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Self::Object),
    }
  }
}

impl From<Value> for serde_json::Value {
  fn from(value: Value) -> Self {
    match value {
      Value::Null => Self::Null,
      Value::Bool(value) => Self::Bool(value),
      Value::Int(value) => Self::Number(value.into()),
      Value::Float(value) => serde_json::Number::from_f64(value)
        .map(Self::Number)
        .unwrap_or(Self::Null),
      Value::String(value) => Self::String(value),
      Value::Array(values) => Self::Array(values.into_iter().map(Self::from).collect()),
      Value::Object(values) => Self::Object(
        values
          .into_iter()
          .map(|(key, value)| (key, Self::from(value)))
          .collect(),
      ),
    }
  }
}
