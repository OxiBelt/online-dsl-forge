use crate::parser::SourceSpan;
use crate::value::Value;

use super::{DynamicRegistry, EvalError};

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
