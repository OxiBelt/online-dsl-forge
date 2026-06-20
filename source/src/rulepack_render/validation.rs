use std::net::IpAddr;

use crate::rulepack_render::error::{RenderResult, fail};

pub(crate) fn validate_non_empty(source: &str, field: &str, value: &str) -> RenderResult<()> {
  if value.trim().is_empty() {
    return fail(format!("{source} {field} must not be empty"));
  }
  Ok(())
}

pub(crate) fn validate_label(source: &str, field: &str, value: &str) -> RenderResult<()> {
  validate_non_empty(source, field, value)?;
  if value.len() > 128 {
    return fail(format!("{source} {field} exceeds 128 bytes"));
  }
  if !value
    .bytes()
    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
  {
    return fail(format!("{source} {field} contains unsupported characters"));
  }
  Ok(())
}

pub(crate) fn validate_human_text(source: &str, field: &str, value: &str) -> RenderResult<()> {
  validate_non_empty(source, field, value)?;
  if value.len() > 512 || value.bytes().any(|byte| byte.is_ascii_control()) {
    return fail(format!("{source} {field} must be 1 to 512 printable bytes"));
  }
  Ok(())
}

pub(crate) fn validate_source_text(source: &str, field: &str, value: &str) -> RenderResult<()> {
  validate_non_empty(source, field, value)?;
  if value.len() > 2048 {
    return fail(format!("{source} {field} exceeds 2048 bytes"));
  }
  Ok(())
}

pub(crate) fn validate_source_sha256(source: &str, field: &str, value: &str) -> RenderResult<()> {
  validate_non_empty(source, field, value)?;
  if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return fail(format!(
      "{source} {field} must be a 64-character hex SHA-256 digest"
    ));
  }
  Ok(())
}

pub(crate) fn validate_source_fingerprint(
  source: &str,
  field: &str,
  value: &str,
) -> RenderResult<()> {
  validate_non_empty(source, field, value)?;
  if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return fail(format!(
      "{source} {field} must be a full 40- or 64-character hex OpenPGP fingerprint"
    ));
  }
  Ok(())
}

pub(crate) fn validate_cidr(value: &str) -> RenderResult<()> {
  let Some((address, prefix)) = value.split_once('/') else {
    return fail(format!("CIDR {value} must use address/prefix format"));
  };
  let address = address.parse::<IpAddr>().map_err(|_| {
    crate::rulepack_render::RulepackRenderError::new(format!("invalid CIDR {value}"))
  })?;
  let prefix = prefix.parse::<u8>().map_err(|_| {
    crate::rulepack_render::RulepackRenderError::new(format!("invalid CIDR prefix in {value}"))
  })?;
  let max_prefix = match address {
    IpAddr::V4(_) => 32,
    IpAddr::V6(_) => 128,
  };
  if prefix > max_prefix {
    return fail(format!("CIDR prefix in {value} exceeds {max_prefix}"));
  }
  Ok(())
}

pub(crate) fn validate_rate(value: &str) -> RenderResult<()> {
  let Some((amount, unit)) = value.split_once("r/") else {
    return fail("rate must use format like 10r/s or 600r/m");
  };
  let amount = amount
    .parse::<f64>()
    .map_err(|_| crate::rulepack_render::RulepackRenderError::new("invalid rate amount"))?;
  if amount <= 0.0 {
    return fail("rate amount must be greater than 0");
  }
  if !matches!(unit, "s" | "m" | "h") {
    return fail("rate unit must be s, m, or h");
  }
  Ok(())
}

pub(crate) fn validate_status(source: &str, field: &str, status: u16) -> RenderResult<()> {
  if !(100..=599).contains(&status) {
    return fail(format!("{source} {field} {status} is invalid"));
  }
  Ok(())
}

pub(crate) fn validate_method(value: &str) -> RenderResult<()> {
  validate_token(value, "HTTP method")
}

fn validate_token(value: &str, label: &str) -> RenderResult<()> {
  if value.is_empty()
    || value.len() > 32
    || !value.bytes().all(|byte| {
      byte.is_ascii_alphanumeric()
        || matches!(
          byte,
          b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
        )
    })
  {
    return fail(format!("{label} must be a valid HTTP token"));
  }
  Ok(())
}
