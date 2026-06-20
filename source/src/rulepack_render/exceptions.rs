use std::collections::HashSet;

use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::types::{RulepackException, RulepackPhase, RulepackRule};
use crate::rulepack_render::validation::{
  validate_cidr, validate_human_text, validate_label, validate_method,
};

pub(crate) fn append_local_exceptions(
  value: &mut toml::Value,
  source: &str,
  exceptions: &[RulepackException],
) -> RenderResult<()> {
  if exceptions.is_empty() {
    return Ok(());
  }
  let Some(table) = value.as_table_mut() else {
    return fail(format!("{source} must contain a TOML table"));
  };
  let entry = table
    .entry("exceptions".to_string())
    .or_insert_with(|| toml::Value::Array(Vec::new()));
  let Some(items) = entry.as_array_mut() else {
    return fail(format!("{source} exceptions must be an array of tables"));
  };
  for exception in exceptions {
    let encoded = toml::Value::try_from(exception.clone()).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "failed to encode local rulepack exception {}: {error}",
        exception.name
      ))
    })?;
    items.push(encoded);
  }
  Ok(())
}

pub(crate) fn validate_rulepack_exception_list(
  source: &str,
  exceptions: &[RulepackException],
) -> RenderResult<()> {
  validate_exception_shapes(source, exceptions)
}

pub(crate) fn validate_rulepack_exceptions(
  source: &str,
  exceptions: &[RulepackException],
  rules: &[RulepackRule],
) -> RenderResult<()> {
  validate_exception_shapes(source, exceptions)?;
  for exception in active_exception_entries(source, exceptions)? {
    let matches = rules
      .iter()
      .filter(|rule| exception_matches_rule(exception, rule))
      .collect::<Vec<_>>();
    if matches.is_empty() {
      return fail(format!(
        "{source} exception {} did not match any rule",
        exception.name
      ));
    }
    if matches
      .iter()
      .any(|rule| rule.phase == RulepackPhase::Stream)
    {
      return fail(format!(
        "{source} exception {} matched a stream-phase rule; rulepack exceptions only support HTTP request-context selectors",
        exception.name
      ));
    }
  }
  Ok(())
}

fn validate_exception_shapes(source: &str, exceptions: &[RulepackException]) -> RenderResult<()> {
  let mut names = HashSet::new();
  for exception in exceptions {
    validate_label(source, "exceptions.name", &exception.name)?;
    if !names.insert(exception.name.clone()) {
      return fail(format!(
        "{source} contains duplicate exception {}",
        exception.name
      ));
    }
    validate_selector(source, exception)?;
    validate_traffic_selector(source, exception)?;
    validate_human_text(source, "exceptions.reason", &exception.reason)?;
    if let Some(expires_at) = &exception.expires_at {
      parse_strict_utc_rfc3339(expires_at).map_err(|error| {
        crate::rulepack_render::RulepackRenderError::new(format!(
          "{source} exception {} expires_at is invalid: {error}",
          exception.name
        ))
      })?;
    }
  }
  Ok(())
}

fn validate_selector(source: &str, exception: &RulepackException) -> RenderResult<()> {
  if exception.rule_ids.is_empty() && exception.rule_names.is_empty() && exception.tags.is_empty() {
    return fail(format!(
      "{source} exception {} must include at least one rule selector",
      exception.name
    ));
  }
  for value in &exception.rule_ids {
    validate_label(source, "exceptions.rule_ids", value)?;
  }
  for value in &exception.rule_names {
    validate_label(source, "exceptions.rule_names", value)?;
  }
  for value in &exception.tags {
    validate_label(source, "exceptions.tags", value)?;
  }
  Ok(())
}

fn validate_traffic_selector(source: &str, exception: &RulepackException) -> RenderResult<()> {
  if exception.routes.is_empty()
    && exception.methods.is_empty()
    && exception.path_prefixes.is_empty()
    && exception.source_cidrs.is_empty()
  {
    return fail(format!(
      "{source} exception {} must include at least one traffic selector",
      exception.name
    ));
  }
  for value in &exception.routes {
    validate_label(source, "exceptions.routes", value)?;
  }
  for value in &exception.methods {
    validate_method(value).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} exception {} has invalid HTTP method {value}: {error}",
        exception.name
      ))
    })?;
  }
  for value in &exception.path_prefixes {
    if value.trim().is_empty()
      || value.len() > 512
      || value.bytes().any(|byte| byte.is_ascii_control())
    {
      return fail(format!(
        "{source} exception {} path_prefixes entries must be 1 to 512 printable bytes",
        exception.name
      ));
    }
    if !value.starts_with('/') {
      return fail(format!(
        "{source} exception {} path_prefixes entries must start with /",
        exception.name
      ));
    }
  }
  for value in &exception.source_cidrs {
    validate_cidr(value).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} exception {} source_cidrs entry {value} is invalid: {error}",
        exception.name
      ))
    })?;
  }
  Ok(())
}

fn active_exception_entries<'a>(
  source: &str,
  exceptions: &'a [RulepackException],
) -> RenderResult<Vec<&'a RulepackException>> {
  let now = now_unix_seconds();
  let mut active = Vec::new();
  for exception in exceptions {
    if let Some(expires_at) = &exception.expires_at {
      let expires_at = parse_strict_utc_rfc3339(expires_at).map_err(|error| {
        crate::rulepack_render::RulepackRenderError::new(format!(
          "{source} exception {} expires_at is invalid: {error}",
          exception.name
        ))
      })?;
      if expires_at <= now {
        continue;
      }
    }
    active.push(exception);
  }
  Ok(active)
}

fn exception_matches_rule(exception: &RulepackException, rule: &RulepackRule) -> bool {
  exception
    .rule_ids
    .iter()
    .any(|id| rule.id.as_deref() == Some(id.as_str()))
    || exception.rule_names.iter().any(|name| name == &rule.name)
    || exception
      .tags
      .iter()
      .any(|wanted| rule.tags.iter().any(|tag| tag == wanted))
}

fn parse_strict_utc_rfc3339(value: &str) -> RenderResult<i64> {
  let bytes = value.as_bytes();
  if bytes.len() != 20
    || bytes[4] != b'-'
    || bytes[7] != b'-'
    || bytes[10] != b'T'
    || bytes[13] != b':'
    || bytes[16] != b':'
    || bytes[19] != b'Z'
    || !bytes
      .iter()
      .enumerate()
      .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
      .all(|(_, byte)| byte.is_ascii_digit())
  {
    return fail("timestamp must use YYYY-MM-DDTHH:MM:SSZ");
  }
  let year = parse_i64(&value[0..4])?;
  let month = parse_u32(&value[5..7])?;
  let day = parse_u32(&value[8..10])?;
  let hour = parse_u32(&value[11..13])?;
  let minute = parse_u32(&value[14..16])?;
  let second = parse_u32(&value[17..19])?;
  if !(1..=12).contains(&month) {
    return fail("month is out of range");
  }
  let max_day = days_in_month(year, month);
  if day == 0 || day > max_day {
    return fail("day is out of range");
  }
  if hour > 23 || minute > 59 || second > 59 {
    return fail("time is out of range");
  }
  let days = days_from_civil(year, month, day);
  Ok(days * 86_400 + i64::from(hour * 3_600 + minute * 60 + second))
}

fn parse_i64(value: &str) -> RenderResult<i64> {
  value
    .parse::<i64>()
    .map_err(|_| crate::rulepack_render::RulepackRenderError::new("invalid integer"))
}

fn parse_u32(value: &str) -> RenderResult<u32> {
  value
    .parse::<u32>()
    .map_err(|_| crate::rulepack_render::RulepackRenderError::new("invalid integer"))
}

fn days_in_month(year: i64, month: u32) -> u32 {
  match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 if is_leap_year(year) => 29,
    2 => 28,
    _ => 0,
  }
}

fn is_leap_year(year: i64) -> bool {
  (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
  let year = year - i64::from(month <= 2);
  let era = if year >= 0 { year } else { year - 399 } / 400;
  let year_of_era = year - era * 400;
  let month = i64::from(month);
  let day = i64::from(day);
  let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
  let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
  era * 146_097 + day_of_era - 719_468
}

fn now_unix_seconds() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or(std::time::Duration::ZERO)
    .as_secs()
    .min(i64::MAX as u64) as i64
}
