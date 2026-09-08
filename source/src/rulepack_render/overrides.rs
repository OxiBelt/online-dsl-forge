use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::limits::RenderMeter;
use crate::rulepack_render::types::{
  RulepackActionSelector, RulepackOverride, RulepackOverrideSelector,
};
use crate::rulepack_render::validation::{validate_label, validate_rate, validate_status};

const STRING_ALLOCATION_OVERHEAD: usize = 32;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum OverrideScope {
  Manifest,
  Local,
}

struct OrderedOverride<'a> {
  scope: OverrideScope,
  index: usize,
  item: &'a RulepackOverride,
}

#[derive(Clone, Copy, Default)]
struct ActionProjection {
  replacement_bytes: usize,
  rate_growth: usize,
  body_growth: usize,
  burst_growth: usize,
  status_growth: usize,
}

#[derive(Clone, Copy, Default)]
struct ActionFieldBounds {
  rate: usize,
  body: usize,
  burst: usize,
  status: usize,
}

pub(crate) fn validate_rulepack_overrides(
  source: &str,
  rulepack_name: &str,
  overrides: &[RulepackOverride],
) -> RenderResult<()> {
  for override_item in overrides {
    validate_override(source, rulepack_name, override_item)?;
  }
  Ok(())
}

pub(crate) fn apply_overrides(
  value: &mut toml::Value,
  source: &str,
  rulepack_name: &str,
  manifest_overrides: &[RulepackOverride],
  local_overrides: &[RulepackOverride],
  meter: &mut RenderMeter,
) -> RenderResult<()> {
  let overrides = ordered_overrides(manifest_overrides, local_overrides);
  if overrides.is_empty() {
    return Ok(());
  }
  let Some(rules) = value.get_mut("rules").and_then(toml::Value::as_array_mut) else {
    return fail(format!(
      "{source} overrides require at least one [[rules]] entry"
    ));
  };
  preflight_override_application(rules, source, rulepack_name, &overrides, meter)?;
  let mut rendered_rules = Vec::with_capacity(rules.len());
  for rule_value in rules.iter() {
    let Some(original_rule) = rule_value.as_table() else {
      return fail(format!("{source} rules entries must be tables"));
    };
    let mut rendered_rule = toml::Value::Table(original_rule.clone());
    let mut enabled = true;
    for ordered in &overrides {
      if selector_matches_rule(rulepack_name, &ordered.item.selector, original_rule) {
        apply_override_to_rule(source, ordered.item, &mut rendered_rule, &mut enabled)?;
      }
    }
    if enabled {
      rendered_rules.push(rendered_rule);
    }
  }
  *rules = rendered_rules;
  Ok(())
}

fn preflight_override_application(
  rules: &[toml::Value],
  source: &str,
  rulepack_name: &str,
  overrides: &[OrderedOverride<'_>],
  meter: &mut RenderMeter,
) -> RenderResult<()> {
  reserve_override_selector_work(rules, overrides, meter, source)?;
  let projections = overrides
    .iter()
    .map(|ordered| action_projection(ordered.item, source))
    .collect::<RenderResult<Vec<_>>>()?;

  let mut match_counts = vec![0usize; overrides.len()];
  let mut projected_render_work = 0usize;
  for rule_value in rules {
    projected_render_work = checked_add(
      projected_render_work,
      crate::rulepack_render::serialization::estimate_value(rule_value, 1, source)?,
      "projected override render byte count",
      source,
    )?;
    let Some(rule) = rule_value.as_table() else {
      return fail(format!("{source} rules entries must be tables"));
    };
    let mut matches = Vec::new();
    for (position, ordered) in overrides.iter().enumerate() {
      if selector_matches_rule(rulepack_name, &ordered.item.selector, rule) {
        match_counts[position] += 1;
        matches.push(position);
      }
    }
    projected_render_work = checked_add(
      projected_render_work,
      preflight_action_overrides(rule, &matches, overrides, &projections, source, meter)?,
      "projected override render byte count",
      source,
    )?;
  }
  for (position, count) in match_counts.into_iter().enumerate() {
    if count == 0 {
      let ordered = &overrides[position];
      return fail(format!(
        "{source} {} override {} did not match any rule",
        scope_name(ordered.scope),
        ordered.index + 1
      ));
    }
  }
  meter.reserve_render_work(projected_render_work, source)
}

fn reserve_override_selector_work(
  rules: &[toml::Value],
  overrides: &[OrderedOverride<'_>],
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  meter.reserve_selector_product(rules.len(), overrides.len(), source)?;
  meter.reserve_selector_product(rules.len(), overrides.len(), source)?;
  let total_rule_tags = rules.iter().try_fold(0usize, |total, rule| {
    let tags = rule
      .get("tags")
      .and_then(toml::Value::as_array)
      .map_or(0, Vec::len);
    checked_add(total, tags, "selector work unit count", source)
  })?;
  let total_selector_tags = overrides.iter().try_fold(0usize, |total, ordered| {
    checked_add(
      total,
      ordered.item.selector.tags.len(),
      "selector work unit count",
      source,
    )
  })?;
  meter.reserve_selector_product(total_rule_tags, total_selector_tags, source)?;
  meter.reserve_selector_product(total_rule_tags, total_selector_tags, source)
}

fn preflight_action_overrides(
  rule: &toml::value::Table,
  matches: &[usize],
  overrides: &[OrderedOverride<'_>],
  projections: &[ActionProjection],
  source: &str,
  meter: &mut RenderMeter,
) -> RenderResult<usize> {
  let action_overrides = matches
    .iter()
    .filter(|position| overrides[**position].item.action.is_some())
    .copied()
    .collect::<Vec<_>>();
  if action_overrides.is_empty() {
    return Ok(0);
  }
  if rule.get("path").is_some() {
    let name = rule
      .get("name")
      .and_then(toml::Value::as_str)
      .unwrap_or("<unknown>");
    return fail(format!(
      "{source} rule {name} uses path; action overrides require inline content"
    ));
  }
  let Some(content) = rule.get("content").and_then(toml::Value::as_str) else {
    return fail(format!(
      "{source} action overrides require inline rule content"
    ));
  };
  let value: toml::Value = toml::from_str(content).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to parse {source} rule content: {error}"
    ))
  })?;
  let Some(actions) = value.get("actions").and_then(toml::Value::as_array) else {
    return fail(format!(
      "{source} action override found no [[actions]] entries"
    ));
  };
  meter.reserve_selector_product(actions.len(), action_overrides.len(), source)?;
  meter.reserve_selector_product(actions.len(), action_overrides.len(), source)?;

  let mut projected = 0usize;
  let mut current_content_bound =
    crate::rulepack_render::serialization::estimate_value(&value, 0, source)?;
  let mut action_field_bounds = vec![ActionFieldBounds::default(); actions.len()];
  for position in action_overrides {
    let override_item = overrides[position].item;
    let action = override_item.action.as_ref().ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} action override is missing action selector"
      ))
    })?;
    let mut matching_actions = 0usize;
    let mut matching_index = None;
    for (index, action_value) in actions.iter().enumerate() {
      let Some(table) = action_value.as_table() else {
        return fail(format!("{source} action entries must be tables"));
      };
      if action_matches_selector(table, action) {
        matching_actions += 1;
        matching_index = Some(index);
      }
    }
    if matching_actions != 1 {
      return fail(format!(
        "{source} action override for {} matched {matching_actions} actions; expected exactly one",
        action.action_type
      ));
    }
    projected = checked_add(
      projected,
      current_content_bound,
      "projected override render byte count",
      source,
    )?;
    let projection = projections[position];
    projected = checked_add(
      projected,
      projection.replacement_bytes,
      "projected override render byte count",
      source,
    )?;
    let action_bounds = &mut action_field_bounds[matching_index.ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} action override lost its validated action match"
      ))
    })?];
    apply_field_growth(
      &mut current_content_bound,
      &mut action_bounds.rate,
      projection.rate_growth,
      source,
    )?;
    apply_field_growth(
      &mut current_content_bound,
      &mut action_bounds.body,
      projection.body_growth,
      source,
    )?;
    apply_field_growth(
      &mut current_content_bound,
      &mut action_bounds.burst,
      projection.burst_growth,
      source,
    )?;
    apply_field_growth(
      &mut current_content_bound,
      &mut action_bounds.status,
      projection.status_growth,
      source,
    )?;
    projected = checked_add(
      projected,
      current_content_bound,
      "projected override render byte count",
      source,
    )?;
  }
  Ok(projected)
}

fn action_projection(
  override_item: &RulepackOverride,
  source: &str,
) -> RenderResult<ActionProjection> {
  if override_item.action.is_none() {
    return Ok(ActionProjection::default());
  }
  let mut replacement_bytes = 0usize;
  let mut string_projection = |value: &str| -> RenderResult<usize> {
    replacement_bytes = checked_add(
      replacement_bytes,
      checked_add(
        STRING_ALLOCATION_OVERHEAD,
        value.len(),
        "projected override render byte count",
        source,
      )?,
      "projected override render byte count",
      source,
    )?;
    checked_add(
      crate::rulepack_render::serialization::encoded_string_bound(value, source)?,
      16,
      "projected override render byte count",
      source,
    )
  };
  let rate_growth = override_item
    .rate
    .as_deref()
    .map(&mut string_projection)
    .transpose()?
    .unwrap_or(0);
  let body_growth = override_item
    .body
    .as_deref()
    .map(string_projection)
    .transpose()?
    .unwrap_or(0);
  Ok(ActionProjection {
    replacement_bytes,
    rate_growth,
    body_growth,
    burst_growth: usize::from(override_item.burst.is_some()) * 32,
    status_growth: usize::from(override_item.status.is_some()) * 32,
  })
}

fn apply_field_growth(
  content_bound: &mut usize,
  current_field_bound: &mut usize,
  next_field_bound: usize,
  source: &str,
) -> RenderResult<()> {
  if next_field_bound > *current_field_bound {
    *content_bound = checked_add(
      *content_bound,
      next_field_bound - *current_field_bound,
      "projected override render byte count",
      source,
    )?;
    *current_field_bound = next_field_bound;
  }
  Ok(())
}

fn ordered_overrides<'a>(
  manifest_overrides: &'a [RulepackOverride],
  local_overrides: &'a [RulepackOverride],
) -> Vec<OrderedOverride<'a>> {
  let mut overrides = Vec::with_capacity(manifest_overrides.len() + local_overrides.len());
  for (index, item) in manifest_overrides.iter().enumerate() {
    overrides.push(OrderedOverride {
      scope: OverrideScope::Manifest,
      index,
      item,
    });
  }
  for (index, item) in local_overrides.iter().enumerate() {
    overrides.push(OrderedOverride {
      scope: OverrideScope::Local,
      index,
      item,
    });
  }
  overrides.sort_by_key(|ordered| {
    (
      ordered.scope,
      selector_precedence(&ordered.item.selector),
      ordered.index,
    )
  });
  overrides
}

fn validate_override(
  source: &str,
  rulepack_name: &str,
  override_item: &RulepackOverride,
) -> RenderResult<()> {
  validate_selector(source, rulepack_name, &override_item.selector)?;
  let has_rule_field = override_item.mode.is_some()
    || override_item.priority.is_some()
    || override_item.enabled.is_some();
  let has_action_field = override_item.rate.is_some()
    || override_item.burst.is_some()
    || override_item.status.is_some()
    || override_item.body.is_some();
  if !has_rule_field && !has_action_field {
    return fail(format!(
      "{source} override must set at least one supported field"
    ));
  }
  if override_item.action.is_some() && !has_action_field {
    return fail(format!(
      "{source} override action selector requires an action field"
    ));
  }
  if has_action_field {
    let Some(action) = &override_item.action else {
      return fail(format!("{source} action fields require an action selector"));
    };
    validate_action_selector(source, action)?;
    validate_action_fields(source, override_item, action)?;
  }
  if let Some(rate) = &override_item.rate {
    validate_rate(rate).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} override rate must be valid: {error}"
      ))
    })?;
  }
  if let Some(status) = override_item.status {
    validate_status(source, "override status", status)?;
  }
  Ok(())
}

fn validate_selector(
  source: &str,
  rulepack_name: &str,
  selector: &RulepackOverrideSelector,
) -> RenderResult<()> {
  let mut kinds = 0;
  if let Some(value) = &selector.rulepack {
    kinds += 1;
    validate_label(source, "overrides.selector.rulepack", value)?;
    if value != rulepack_name {
      return fail(format!(
        "{source} override selector rulepack {value} does not match rulepack {rulepack_name}"
      ));
    }
  }
  if !selector.tags.is_empty() {
    kinds += 1;
    for tag in &selector.tags {
      validate_label(source, "overrides.selector.tags", tag)?;
    }
  }
  if let Some(value) = &selector.rule_id {
    kinds += 1;
    validate_label(source, "overrides.selector.rule_id", value)?;
  }
  if let Some(value) = &selector.rule_name {
    kinds += 1;
    validate_label(source, "overrides.selector.rule_name", value)?;
  }
  if kinds != 1 {
    return fail(format!(
      "{source} override selector must set exactly one selector kind"
    ));
  }
  Ok(())
}

fn validate_action_selector(source: &str, action: &RulepackActionSelector) -> RenderResult<()> {
  validate_label(source, "overrides.action.type", &action.action_type)?;
  if let Some(name) = &action.name {
    validate_label(source, "overrides.action.name", name)?;
  }
  match action.action_type.as_str() {
    "rate_limit" | "reject" | "replace_response" | "reject_response" => {}
    other => {
      return fail(format!(
        "{source} override action type {other} is not supported"
      ));
    }
  }
  if action.action_type == "rate_limit" && action.name.is_none() {
    return fail(format!(
      "{source} rate_limit action overrides require action.name"
    ));
  }
  Ok(())
}

fn validate_action_fields(
  source: &str,
  override_item: &RulepackOverride,
  action: &RulepackActionSelector,
) -> RenderResult<()> {
  if (override_item.rate.is_some() || override_item.burst.is_some())
    && action.action_type != "rate_limit"
  {
    return fail(format!(
      "{source} rate and burst overrides are only supported for rate_limit actions"
    ));
  }
  if (override_item.status.is_some() || override_item.body.is_some())
    && !matches!(
      action.action_type.as_str(),
      "rate_limit" | "reject" | "replace_response" | "reject_response"
    )
  {
    return fail(format!(
      "{source} status and body overrides are not supported for this action"
    ));
  }
  Ok(())
}

fn selector_precedence(selector: &RulepackOverrideSelector) -> usize {
  if selector.rulepack.is_some() {
    0
  } else if !selector.tags.is_empty() {
    1
  } else {
    2
  }
}

fn selector_matches_rule(
  rulepack_name: &str,
  selector: &RulepackOverrideSelector,
  rule: &toml::value::Table,
) -> bool {
  if selector.rulepack.as_deref() == Some(rulepack_name) {
    return true;
  }
  if !selector.tags.is_empty()
    && rule
      .get("tags")
      .and_then(toml::Value::as_array)
      .is_some_and(|tags| {
        tags.iter().any(|tag| {
          tag
            .as_str()
            .is_some_and(|tag| selector.tags.iter().any(|wanted| wanted == tag))
        })
      })
  {
    return true;
  }
  if let Some(rule_id) = &selector.rule_id
    && rule.get("id").and_then(toml::Value::as_str) == Some(rule_id)
  {
    return true;
  }
  if let Some(rule_name) = &selector.rule_name
    && rule.get("name").and_then(toml::Value::as_str) == Some(rule_name)
  {
    return true;
  }
  false
}

fn apply_override_to_rule(
  source: &str,
  override_item: &RulepackOverride,
  rule: &mut toml::Value,
  enabled: &mut bool,
) -> RenderResult<()> {
  let Some(table) = rule.as_table_mut() else {
    return fail(format!("{source} rules entries must be tables"));
  };
  if let Some(value) = override_item.enabled {
    *enabled = value;
  }
  if let Some(mode) = override_item.mode {
    table.insert(
      "mode".to_string(),
      toml::Value::String(mode.as_str().to_string()),
    );
  }
  if let Some(priority) = override_item.priority {
    table.insert("priority".to_string(), toml::Value::Integer(priority));
  }
  if override_item.action.is_some() {
    if table.get("path").is_some() {
      let name = table
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or("<unknown>");
      return fail(format!(
        "{source} rule {name} uses path; action overrides require inline content"
      ));
    }
    let Some(content) = table.get("content").and_then(toml::Value::as_str) else {
      return fail(format!(
        "{source} action overrides require inline rule content"
      ));
    };
    let content = apply_override_to_content(source, content, override_item)?;
    table.insert("content".to_string(), toml::Value::String(content));
  }
  Ok(())
}

fn apply_override_to_content(
  source: &str,
  content: &str,
  override_item: &RulepackOverride,
) -> RenderResult<String> {
  let Some(action) = override_item.action.as_ref() else {
    return fail(format!(
      "{source} action override is missing action selector"
    ));
  };
  let mut value: toml::Value = toml::from_str(content).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to parse {source} rule content: {error}"
    ))
  })?;
  let Some(actions) = value.get_mut("actions").and_then(toml::Value::as_array_mut) else {
    return fail(format!(
      "{source} action override found no [[actions]] entries"
    ));
  };
  let mut matches = Vec::new();
  for (index, action_value) in actions.iter().enumerate() {
    let Some(table) = action_value.as_table() else {
      return fail(format!("{source} action entries must be tables"));
    };
    if action_matches_selector(table, action) {
      matches.push(index);
    }
  }
  if matches.len() != 1 {
    return fail(format!(
      "{source} action override for {} matched {} actions; expected exactly one",
      action.action_type,
      matches.len()
    ));
  }
  let Some(table) = actions[matches[0]].as_table_mut() else {
    return fail(format!("{source} matched action entry must be a table"));
  };
  if let Some(rate) = &override_item.rate {
    table.insert("rate".to_string(), toml::Value::String(rate.clone()));
  }
  if let Some(burst) = override_item.burst {
    table.insert("burst".to_string(), toml::Value::Integer(i64::from(burst)));
  }
  if let Some(status) = override_item.status {
    table.insert(
      "status".to_string(),
      toml::Value::Integer(i64::from(status)),
    );
  }
  if let Some(body) = &override_item.body {
    table.insert("body".to_string(), toml::Value::String(body.clone()));
  }
  toml::to_string_pretty(&value).map_err(|error| {
    crate::rulepack_render::RulepackRenderError::new(format!(
      "failed to render overridden rule content: {error}"
    ))
  })
}

fn action_matches_selector(table: &toml::value::Table, selector: &RulepackActionSelector) -> bool {
  if table.get("type").and_then(toml::Value::as_str) != Some(selector.action_type.as_str()) {
    return false;
  }
  match selector.name.as_deref() {
    Some(name) => table.get("name").and_then(toml::Value::as_str) == Some(name),
    None => true,
  }
}

fn scope_name(scope: OverrideScope) -> &'static str {
  match scope {
    OverrideScope::Manifest => "manifest",
    OverrideScope::Local => "local",
  }
}

fn checked_add(left: usize, right: usize, label: &str, source: &str) -> RenderResult<usize> {
  left.checked_add(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!("{source} {label} overflow"))
  })
}
