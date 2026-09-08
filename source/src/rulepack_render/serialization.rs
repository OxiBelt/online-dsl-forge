use crate::rulepack_render::error::RenderResult;
use crate::rulepack_render::limits::RenderMeter;

const SERIALIZER_WORK_MULTIPLIER: usize = 3;

pub(crate) fn reserve_toml_serialization(
  value: &toml::Value,
  meter: &mut RenderMeter,
  source: &str,
  includes_value_clone: bool,
) -> RenderResult<()> {
  reserve_serialization_estimate(
    estimate_value(value, 0, source)?,
    meter,
    source,
    includes_value_clone,
  )
}

pub(crate) fn reserve_projected_toml_serialization(
  value: &toml::Value,
  projected_growth: usize,
  meter: &mut RenderMeter,
  source: &str,
  includes_value_clone: bool,
) -> RenderResult<()> {
  let estimate = checked_add(estimate_value(value, 0, source)?, projected_growth, source)?;
  reserve_serialization_estimate(estimate, meter, source, includes_value_clone)
}

fn reserve_serialization_estimate(
  estimate: usize,
  meter: &mut RenderMeter,
  source: &str,
  includes_value_clone: bool,
) -> RenderResult<()> {
  let multiplier = SERIALIZER_WORK_MULTIPLIER + usize::from(includes_value_clone);
  let work = checked_mul(estimate, multiplier, source)?;
  meter.reserve_render_work(work, source)
}

pub(crate) fn estimate_value(
  value: &toml::Value,
  depth: usize,
  source: &str,
) -> RenderResult<usize> {
  match value {
    toml::Value::String(text) => encoded_string_bound(text, source),
    toml::Value::Integer(_) | toml::Value::Float(_) | toml::Value::Datetime(_) => Ok(64),
    toml::Value::Boolean(_) => Ok(5),
    toml::Value::Array(values) => {
      let mut total = 2usize;
      for value in values {
        total = checked_add(total, estimate_value(value, depth, source)?, source)?;
        total = checked_add(total, 2, source)?;
      }
      Ok(total)
    }
    toml::Value::Table(table) => {
      let mut total = 2usize;
      for (key, value) in table {
        let key_bound = encoded_string_bound(key, source)?;
        let repeated_path_bound = checked_mul(key_bound, depth.saturating_add(1), source)?;
        total = checked_add(total, repeated_path_bound, source)?;
        total = checked_add(total, 64, source)?;
        total = checked_add(
          total,
          estimate_value(value, depth.saturating_add(1), source)?,
          source,
        )?;
      }
      Ok(total)
    }
  }
}

pub(crate) fn encoded_string_bound(value: &str, source: &str) -> RenderResult<usize> {
  let mut total = 2usize;
  for character in value.chars() {
    let bytes = match character {
      '\u{0000}'..='\u{0008}' | '\u{000b}' | '\u{000e}'..='\u{001f}' | '\u{007f}' => 6,
      '\t' | '\n' | '\u{000c}' | '\r' | '"' | '\\' => 2,
      character => character.len_utf8(),
    };
    total = checked_add(total, bytes, source)?;
  }
  Ok(total)
}

fn checked_add(left: usize, right: usize, source: &str) -> RenderResult<usize> {
  left.checked_add(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "{source} TOML serialization estimate overflow"
    ))
  })
}

fn checked_mul(left: usize, right: usize, source: &str) -> RenderResult<usize> {
  left.checked_mul(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "{source} TOML serialization estimate overflow"
    ))
  })
}
