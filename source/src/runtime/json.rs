use crate::parser::SourceSpan;
use crate::value::{Value, ValueGraphLimits, convert_json_with_limits as convert_value};

use super::{EvalError, RuntimeResourceLimits};

pub(super) fn convert_json_with_limits(
  bindings: serde_json::Value,
  limits: RuntimeResourceLimits,
) -> Result<Value, EvalError> {
  convert_value(
    bindings,
    ValueGraphLimits {
      max_depth: limits.max_value_depth,
      max_nodes: limits.max_value_nodes,
      max_items: limits.max_value_items,
      max_bytes: limits.max_value_bytes,
    },
  )
  .map_err(|error| EvalError::new(error.to_string(), SourceSpan::default()))
}
