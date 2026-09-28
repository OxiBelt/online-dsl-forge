use std::collections::{BTreeMap, HashSet};

use crate::parser::{
  AstExpression, AstFormatLimits, ExprKind, ParseLimits, SourceSpan, format_expression_with_limits,
  parse_expression_with_limits,
};
use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::limits::RenderMeter;
use crate::rulepack_render::types::RulepackReferencedFileKind;

const MAX_TOML_DEPTH: usize = 128;

pub(crate) fn render_manifest_strings(
  value: &mut toml::Value,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  render_manifest_value(value, variables, declared, meter, source, 0, None)
}

pub(crate) fn render_embedded_files(
  value: &mut toml::Value,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  for (section, kind) in [
    ("rules", RulepackReferencedFileKind::Rule),
    ("group_files", RulepackReferencedFileKind::Group),
  ] {
    let Some(entries) = value.get_mut(section).and_then(toml::Value::as_array_mut) else {
      continue;
    };
    for (index, entry) in entries.iter_mut().enumerate() {
      let Some(table) = entry.as_table_mut() else {
        return fail(format!("{source} {section} entry {index} must be a table"));
      };
      let Some(content) = table.get_mut("content") else {
        continue;
      };
      let Some(raw) = content.as_str() else {
        return fail(format!(
          "{source} {section} entry {index} content must be a string"
        ));
      };
      meter.file(raw, source, false)?;
      let rendered = render_nested_file(raw, kind, variables, declared, meter, source)?;
      *content = toml::Value::String(rendered);
    }
  }
  Ok(())
}

pub(crate) fn render_nested_file(
  raw: &str,
  kind: RulepackReferencedFileKind,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<String> {
  super::template::reject_markers_outside_toml_strings(raw, source)?;
  let mut value: toml::Value = toml::from_str(raw).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to parse {source} nested rulepack file: {error}"
    ))
  })?;
  render_nested_value(&mut value, variables, declared, meter, source, 0, None)?;
  validate_nested_shape(&value, kind, source)?;
  super::serialization::reserve_toml_serialization(&value, meter, source, false)?;
  let rendered = toml::to_string_pretty(&value).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to render {source} nested rulepack file: {error}"
    ))
  })?;
  Ok(rendered)
}

fn render_manifest_value(
  value: &mut toml::Value,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
  depth: usize,
  parent: Option<&str>,
) -> RenderResult<()> {
  check_depth(depth, source)?;
  match value {
    toml::Value::String(text) => {
      if parent != Some("content") {
        *text = super::template::render_secure(text, variables, declared, meter, source)?;
      }
    }
    toml::Value::Array(values) => {
      for value in values {
        render_manifest_value(value, variables, declared, meter, source, depth + 1, parent)?;
      }
    }
    toml::Value::Table(table) => {
      for key in table.keys() {
        reject_marker_key(key, source)?;
      }
      for (key, value) in table.iter_mut() {
        let skip_content = key == "content" && matches!(parent, Some("rules" | "group_files"));
        if !skip_content {
          render_manifest_value(
            value,
            variables,
            declared,
            meter,
            source,
            depth + 1,
            Some(key),
          )?;
        }
      }
    }
    _ => {}
  }
  Ok(())
}

fn render_nested_value(
  value: &mut toml::Value,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
  depth: usize,
  key: Option<&str>,
) -> RenderResult<()> {
  check_depth(depth, source)?;
  match value {
    toml::Value::String(text) if key == Some("when") => {
      *text = render_when(text, variables, declared, meter, source)?;
    }
    toml::Value::String(text) => {
      *text = super::template::render_secure(text, variables, declared, meter, source)?;
    }
    toml::Value::Array(values) => {
      for value in values {
        render_nested_value(value, variables, declared, meter, source, depth + 1, key)?;
      }
    }
    toml::Value::Table(table) => {
      for key in table.keys() {
        reject_marker_key(key, source)?;
      }
      for (key, value) in table.iter_mut() {
        render_nested_value(
          value,
          variables,
          declared,
          meter,
          source,
          depth + 1,
          Some(key),
        )?;
      }
    }
    _ => {}
  }
  Ok(())
}

fn render_when(
  raw: &str,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<String> {
  super::template::validate_markers_without_count(raw, variables, declared, source)?;
  let mut expression =
    parse_expression_with_limits(raw, parse_limits(meter, false)).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "failed to parse {source} when expression: {error}"
      ))
    })?;
  let mut string_spans = Vec::new();
  collect_string_spans(&expression, &mut string_spans);
  reject_markers_outside_strings(raw, &string_spans, source)?;
  render_ast_strings(&mut expression, variables, declared, meter, source)?;
  let rendered =
    format_expression_with_limits(&expression, format_limits(meter)).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "failed to format {source} when expression: {error}"
      ))
    })?;
  meter.reserve_render_work(rendered.len(), source)?;
  parse_expression_with_limits(&rendered, parse_limits(meter, true)).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to validate rendered {source} when expression: {error}"
    ))
  })?;
  Ok(rendered)
}

fn render_ast_strings(
  expression: &mut AstExpression,
  variables: &BTreeMap<String, String>,
  declared: &HashSet<String>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  match &mut expression.kind {
    ExprKind::String { value } => {
      *value = super::template::render_secure(value, variables, declared, meter, source)?;
    }
    ExprKind::Array { items } => {
      for item in items {
        render_ast_strings(item, variables, declared, meter, source)?;
      }
    }
    ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
      render_ast_strings(receiver, variables, declared, meter, source)?;
    }
    ExprKind::FunctionCall { args, .. } => {
      for arg in args {
        render_ast_strings(arg, variables, declared, meter, source)?;
      }
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      render_ast_strings(receiver, variables, declared, meter, source)?;
      for arg in args {
        render_ast_strings(arg, variables, declared, meter, source)?;
      }
    }
    ExprKind::Binary { left, right, .. } => {
      render_ast_strings(left, variables, declared, meter, source)?;
      render_ast_strings(right, variables, declared, meter, source)?;
    }
    _ => {}
  }
  Ok(())
}

fn collect_string_spans(expression: &AstExpression, spans: &mut Vec<SourceSpan>) {
  match &expression.kind {
    ExprKind::String { .. } => spans.push(expression.span),
    ExprKind::Array { items } => items
      .iter()
      .for_each(|item| collect_string_spans(item, spans)),
    ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
      collect_string_spans(receiver, spans);
    }
    ExprKind::FunctionCall { args, .. } => {
      args.iter().for_each(|arg| collect_string_spans(arg, spans));
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      collect_string_spans(receiver, spans);
      args.iter().for_each(|arg| collect_string_spans(arg, spans));
    }
    ExprKind::Binary { left, right, .. } => {
      collect_string_spans(left, spans);
      collect_string_spans(right, spans);
    }
    _ => {}
  }
}

fn reject_markers_outside_strings(
  raw: &str,
  string_spans: &[SourceSpan],
  source: &str,
) -> RenderResult<()> {
  let mut cursor = 0usize;
  // collect_string_spans visits leaves in source order. Advance past each
  // string once instead of scanning every string for every placeholder.
  let mut span_index = 0usize;
  while let Some(offset) = raw[cursor..].find("{{") {
    let start = cursor + offset;
    let end = raw[start + 2..]
      .find("}}")
      .map(|offset| start + 2 + offset + 2)
      .ok_or_else(|| {
        crate::rulepack_render::RulepackRenderError::new(format!(
          "{source} contains malformed placeholder marker"
        ))
      })?;
    while string_spans
      .get(span_index)
      .is_some_and(|span| span.end <= start)
    {
      span_index += 1;
    }
    if !string_spans
      .get(span_index)
      .is_some_and(|span| start > span.start && end < span.end)
    {
      return fail(format!(
        "{source} when expression placeholder must be inside a string literal"
      ));
    }
    cursor = end;
  }
  Ok(())
}

fn validate_nested_shape(
  value: &toml::Value,
  kind: RulepackReferencedFileKind,
  source: &str,
) -> RenderResult<()> {
  let Some(table) = value.as_table() else {
    return fail(format!(
      "{source} nested rulepack file must be a TOML table"
    ));
  };
  match kind {
    RulepackReferencedFileKind::Rule => {
      if !table.get("when").is_some_and(toml::Value::is_str) {
        return fail(format!(
          "{source} rule file must contain a string when field"
        ));
      }
    }
    RulepackReferencedFileKind::Group => {
      let Some(groups) = table.get("rule_groups").and_then(toml::Value::as_array) else {
        return fail(format!(
          "{source} rule group file must contain [[rule_groups]]"
        ));
      };
      if groups.is_empty() || groups.iter().any(|group| !group.is_table()) {
        return fail(format!(
          "{source} rule_groups must be a non-empty array of tables"
        ));
      }
    }
  }
  Ok(())
}

fn reject_marker_key(key: &str, source: &str) -> RenderResult<()> {
  if key.contains("{{") || key.contains("}}") {
    fail(format!(
      "{source} contains placeholder marker in a TOML key"
    ))
  } else {
    Ok(())
  }
}

fn check_depth(depth: usize, source: &str) -> RenderResult<()> {
  if depth > MAX_TOML_DEPTH {
    fail(format!("{source} TOML nesting exceeds {MAX_TOML_DEPTH}"))
  } else {
    Ok(())
  }
}

fn parse_limits(meter: &RenderMeter, rendered: bool) -> ParseLimits {
  let defaults = ParseLimits::default();
  let byte_limit = if rendered {
    meter.max_single_output()
  } else {
    meter.limits().max_referenced_file_bytes
  };
  ParseLimits {
    max_source_bytes: byte_limit,
    max_decoded_scalar_bytes: byte_limit,
    ..defaults
  }
}

fn format_limits(meter: &RenderMeter) -> AstFormatLimits {
  AstFormatLimits {
    max_output_bytes: meter.remaining_render_work(),
    ..AstFormatLimits::default()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn many_string_literals_and_late_placeholders_render_within_default_limits() {
    let literals = vec!["'ok'"; 8_192].join(", ");
    let markers = "{{x}}".repeat(8_192);
    let raw = format!("[{literals}, '{markers}']");
    let variables = BTreeMap::from([("x".to_string(), "ok".to_string())]);
    let declared = HashSet::from(["x".to_string()]);
    let mut meter = RenderMeter::new(Default::default());

    let rendered = render_when(&raw, &variables, &declared, &mut meter, "test rule")
      .expect("admitted placeholders must render");
    assert!(rendered.ends_with(&format!("\"{}\"]", "ok".repeat(8_192))));
  }
}
