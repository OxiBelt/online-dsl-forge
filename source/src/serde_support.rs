use serde::de::Error as DeserializeError;
use serde::ser::Error as SerializeError;
use serde::{Deserialize, Serializer};

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

pub(crate) fn serialize_f64<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
  S: Serializer,
{
  if value.is_finite() {
    serializer.serialize_f64(*value)
  } else {
    Err(S::Error::custom("expected a finite f64"))
  }
}
