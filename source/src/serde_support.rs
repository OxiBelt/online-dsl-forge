use serde::Deserialize;
use serde::de::Error;

pub(crate) fn deserialize_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let value = serde_json::Number::deserialize(deserializer)?
    .as_f64()
    .ok_or_else(|| D::Error::custom("expected a finite f64"))?;
  if value.is_finite() {
    Ok(value)
  } else {
    Err(D::Error::custom("expected a finite f64"))
  }
}
