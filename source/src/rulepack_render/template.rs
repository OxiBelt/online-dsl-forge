use std::collections::{BTreeMap, HashSet};

use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::limits::RenderMeter;

pub(crate) fn render_secure(
  raw: &str,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<String> {
  let length = measure(raw, variables, declared, Some(meter), source, true)?;
  meter.reserve_render_work(length, source)?;
  render_measured(raw, variables, declared, source, true, length)
}

pub(crate) fn validate_markers_without_count(
  raw: &str,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  source: &str,
) -> RenderResult<()> {
  measure(raw, variables, declared, None, source, true).map(drop)
}

pub(crate) fn render_legacy_bounded(
  raw: &str,
  variables: &BTreeMap<String, String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<String> {
  let declared = variables.keys().cloned().collect();
  let length = measure(raw, variables, &declared, Some(meter), source, false)?;
  meter.reserve_render_work(length, source)?;
  render_measured(raw, variables, &declared, source, false, length)
}

pub(crate) fn reject_markers_outside_toml_strings(raw: &str, source: &str) -> RenderResult<()> {
  let bytes = raw.as_bytes();
  let mut cursor = 0usize;
  let mut quote: Option<(u8, bool)> = None;
  while cursor < bytes.len() {
    match quote {
      None if bytes[cursor] == b'#' => {
        let end = raw[cursor..]
          .find('\n')
          .map_or(bytes.len(), |offset| cursor + offset);
        if raw[cursor..end].contains("{{") || raw[cursor..end].contains("}}") {
          return fail(format!(
            "{source} contains placeholder marker outside a TOML string value"
          ));
        }
        cursor = end;
      }
      None if matches!(bytes[cursor], b'\'' | b'"') => {
        let character = bytes[cursor];
        let triple = bytes.get(cursor..cursor + 3) == Some(&[character, character, character]);
        quote = Some((character, triple));
        cursor += if triple { 3 } else { 1 };
      }
      None => {
        if bytes
          .get(cursor..cursor + 2)
          .is_some_and(|pair| pair == b"{{" || pair == b"}}")
        {
          return fail(format!(
            "{source} contains placeholder marker outside a TOML string value"
          ));
        }
        cursor += 1;
      }
      Some((character, triple)) => {
        let width = if triple { 3 } else { 1 };
        let closes = if triple {
          bytes.get(cursor) == Some(&character)
            && bytes.get(cursor + 1) == Some(&character)
            && bytes.get(cursor + 2) == Some(&character)
        } else {
          bytes.get(cursor) == Some(&character)
        };
        if closes {
          quote = None;
          cursor += width;
        } else if character == b'"' && bytes[cursor] == b'\\' {
          cursor = (cursor + 2).min(bytes.len());
        } else {
          cursor += 1;
        }
      }
    }
  }
  Ok(())
}

fn measure(
  raw: &str,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  mut meter: Option<&mut RenderMeter>,
  source: &str,
  strict: bool,
) -> RenderResult<usize> {
  let mut length = 0usize;
  let mut cursor = 0usize;
  while cursor < raw.len() {
    let remainder = &raw[cursor..];
    let open = remainder.find("{{");
    let stray_close = remainder.find("}}");
    if strict && stray_close.is_some_and(|close| open.is_none_or(|open| close < open)) {
      return fail(format!("{source} contains malformed placeholder marker"));
    }
    let Some(open) = open else {
      length = checked_length(length, remainder.len(), source)?;
      break;
    };
    length = checked_length(length, open, source)?;
    let marker_start = cursor + open;
    let after_open = marker_start + 2;
    let Some(close_offset) = raw[after_open..].find("}}") else {
      if strict {
        return fail(format!("{source} contains malformed placeholder marker"));
      }
      length = checked_length(length, raw.len() - marker_start, source)?;
      break;
    };
    let marker_end = after_open + close_offset + 2;
    let name = &raw[after_open..after_open + close_offset];
    if name.is_empty() || name.contains("{{") || name.contains('{') || name.contains('}') {
      if strict {
        return fail(format!("{source} contains malformed placeholder marker"));
      }
      length = checked_length(length, marker_end - marker_start, source)?;
      cursor = marker_end;
      continue;
    }
    if let Some(meter) = meter.as_deref_mut() {
      meter.placeholder(source)?;
    }
    match variables.get(name) {
      Some(value) => length = checked_length(length, value.len(), source)?,
      None if strict && declared.contains(name) => {
        return fail(format!("{source} contains unresolved placeholder {name}"));
      }
      None if strict => return fail(format!("{source} contains unknown placeholder {name}")),
      None => length = checked_length(length, marker_end - marker_start, source)?,
    }
    cursor = marker_end;
  }
  Ok(length)
}

fn render_measured(
  raw: &str,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  source: &str,
  strict: bool,
  length: usize,
) -> RenderResult<String> {
  let mut output = String::with_capacity(length);
  let mut cursor = 0usize;
  while cursor < raw.len() {
    let remainder = &raw[cursor..];
    let Some(open) = remainder.find("{{") else {
      output.push_str(remainder);
      break;
    };
    output.push_str(&remainder[..open]);
    let marker_start = cursor + open;
    let after_open = marker_start + 2;
    let Some(close_offset) = raw[after_open..].find("}}") else {
      output.push_str(&raw[marker_start..]);
      break;
    };
    let marker_end = after_open + close_offset + 2;
    let name = &raw[after_open..after_open + close_offset];
    match variables.get(name) {
      Some(value) => output.push_str(value),
      None if strict && declared.contains(name) => {
        return fail(format!("{source} contains unresolved placeholder {name}"));
      }
      None if strict => return fail(format!("{source} contains unknown placeholder {name}")),
      None => output.push_str(&raw[marker_start..marker_end]),
    }
    cursor = marker_end;
  }
  Ok(output)
}

fn checked_length(current: usize, added: usize, source: &str) -> RenderResult<usize> {
  current.checked_add(added).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "{source} rendered output byte count overflow"
    ))
  })
}
