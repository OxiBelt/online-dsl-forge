use std::collections::BTreeMap;

use crate::rulepack_render::error::RenderResult;
use crate::rulepack_render::limits::RenderMeter;
use crate::rulepack_render::types::{
  RulepackDocument, RulepackException, RulepackOverride, RulepackRenderOptions,
};

const LOCAL_OVERRIDE_BASE_BYTES: usize = 512;
const LOCAL_EXCEPTION_BASE_BYTES: usize = 512;
const LOCAL_STRING_ITEM_BYTES: usize = 32;
const TOML_FIELD_OVERHEAD: usize = 64;
const TOML_VALUE_SEPARATOR_BYTES: usize = 2;

pub(crate) fn admit_rulepack_structure(
  document: &RulepackDocument,
  options: Option<&RulepackRenderOptions>,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  let profile_assignments = document
    .profiles
    .iter()
    .try_fold(0usize, |total, profile| {
      checked_add(
        total,
        profile.values.len(),
        "profile assignment count",
        source,
      )
    })?;
  meter.profile_assignments(profile_assignments, source)?;

  let local_overrides = options.map_or(0, |options| options.local_overrides.len());
  let override_count = checked_add(
    document.overrides.len(),
    local_overrides,
    "override count",
    source,
  )?;
  meter.overrides(override_count, source)?;

  let local_exceptions = options.map_or(0, |options| options.local_exceptions.len());
  let exception_count = checked_add(
    document.exceptions.len(),
    local_exceptions,
    "exception count",
    source,
  )?;
  meter.exceptions(exception_count, source)?;

  for override_item in &document.overrides {
    admit_override_body(override_item, meter, source)?;
  }
  if let Some(options) = options {
    for override_item in &options.local_overrides {
      admit_override_body(override_item, meter, source)?;
    }
    meter.local_options(
      local_option_bytes(options, meter.limits().max_local_option_bytes, source)?,
      source,
    )?;
  }
  Ok(())
}

pub(crate) fn reserve_option_mutations(
  value: &toml::Value,
  document: &RulepackDocument,
  variables: &BTreeMap<String, String>,
  options: &RulepackRenderOptions,
  meter: &mut RenderMeter,
  source: &str,
) -> RenderResult<()> {
  let growth = projected_option_growth(value, document, variables, options, source)?;
  meter.reserve_render_work(growth, source)?;
  crate::rulepack_render::serialization::reserve_projected_toml_serialization(
    value, growth, meter, source, true,
  )
}

fn admit_override_body(
  override_item: &RulepackOverride,
  meter: &RenderMeter,
  source: &str,
) -> RenderResult<()> {
  if let Some(body) = &override_item.body {
    meter.override_body(body.len(), source)?;
  }
  Ok(())
}

fn local_option_bytes(
  options: &RulepackRenderOptions,
  limit: usize,
  source: &str,
) -> RenderResult<usize> {
  let mut total = 0usize;
  for override_item in &options.local_overrides {
    total = add_local_option_bytes(total, LOCAL_OVERRIDE_BASE_BYTES, limit, source)?;
    total = add_override_bytes(total, override_item, limit, source)?;
  }
  for exception in &options.local_exceptions {
    total = add_local_option_bytes(total, LOCAL_EXCEPTION_BASE_BYTES, limit, source)?;
    total = add_exception_bytes(total, exception, limit, source)?;
  }
  if options.mode_override.is_some() {
    total = add_local_option_bytes(total, 1, limit, source)?;
  }
  if let Some(commit) = &options.source_commit {
    total = add_text(total, commit, limit, source)?;
  }
  if let Some(provenance) = &options.source_provenance {
    total = add_text(total, &provenance.source_url, limit, source)?;
    total = add_text(total, &provenance.source_sha256, limit, source)?;
    if let Some(value) = &provenance.source_openpgp_signature_url {
      total = add_text(total, value, limit, source)?;
    }
    if let Some(value) = &provenance.source_openpgp_signer_fingerprint {
      total = add_text(total, value, limit, source)?;
    }
  }
  Ok(total)
}

fn projected_option_growth(
  value: &toml::Value,
  document: &RulepackDocument,
  variables: &BTreeMap<String, String>,
  options: &RulepackRenderOptions,
  source: &str,
) -> RenderResult<usize> {
  let mut total = 0usize;
  if !options.local_exceptions.is_empty() {
    total = add_table_value(total, "exceptions", TOML_VALUE_SEPARATOR_BYTES, 0, source)?;
    for exception in &options.local_exceptions {
      total = checked_add(
        total,
        exception_estimate(exception, source)?,
        "projected option byte count",
        source,
      )?;
      total = checked_add(
        total,
        TOML_VALUE_SEPARATOR_BYTES,
        "projected option byte count",
        source,
      )?;
    }
  }
  if let Some(mode_override) = &options.mode_override {
    total = add_table_string(
      total,
      "default_mode",
      mode_override.mode.as_str(),
      1,
      source,
    )?;
    if mode_override.force {
      let rules = value
        .get("rules")
        .and_then(toml::Value::as_array)
        .map_or(0, Vec::len);
      for _ in 0..rules {
        total = add_table_string(total, "mode", mode_override.mode.as_str(), 1, source)?;
      }
    }
  }
  if options.pin_variables {
    for variable in &document.variables {
      if let Some(resolved) = variables.get(&variable.name) {
        total = add_table_string(total, "default", resolved, 1, source)?;
        total = add_table_value(total, "required", 5, 1, source)?;
      }
    }
  }
  if let Some(commit) = &options.source_commit {
    total = add_table_string(total, "source_commit", commit, 1, source)?;
  }
  if let Some(provenance) = &options.source_provenance {
    total = add_table_string(total, "source_url", &provenance.source_url, 1, source)?;
    total = add_table_string(total, "source_sha256", &provenance.source_sha256, 1, source)?;
    if let Some(value) = &provenance.source_openpgp_signature_url {
      total = add_table_string(total, "source_openpgp_signature_url", value, 1, source)?;
    }
    if let Some(value) = &provenance.source_openpgp_signer_fingerprint {
      total = add_table_string(total, "source_openpgp_signer_fingerprint", value, 1, source)?;
    }
  }
  Ok(total)
}

fn exception_estimate(exception: &RulepackException, source: &str) -> RenderResult<usize> {
  let mut total = TOML_VALUE_SEPARATOR_BYTES;
  total = add_table_string(total, "name", &exception.name, 1, source)?;
  for (key, values) in [
    ("rule_ids", exception.rule_ids.as_slice()),
    ("rule_names", exception.rule_names.as_slice()),
    ("tags", exception.tags.as_slice()),
    ("routes", exception.routes.as_slice()),
    ("methods", exception.methods.as_slice()),
    ("path_prefixes", exception.path_prefixes.as_slice()),
    ("source_cidrs", exception.source_cidrs.as_slice()),
  ] {
    if !values.is_empty() {
      total = add_table_value(
        total,
        key,
        string_array_estimate(values, source)?,
        1,
        source,
      )?;
    }
  }
  total = add_table_string(total, "reason", &exception.reason, 1, source)?;
  if let Some(value) = &exception.expires_at {
    total = add_table_string(total, "expires_at", value, 1, source)?;
  }
  Ok(total)
}

fn string_array_estimate(values: &[String], source: &str) -> RenderResult<usize> {
  let mut total = TOML_VALUE_SEPARATOR_BYTES;
  for value in values {
    total = checked_add(
      total,
      crate::rulepack_render::serialization::encoded_string_bound(value, source)?,
      "projected option byte count",
      source,
    )?;
    total = checked_add(
      total,
      TOML_VALUE_SEPARATOR_BYTES,
      "projected option byte count",
      source,
    )?;
  }
  Ok(total)
}

fn add_table_string(
  total: usize,
  key: &str,
  value: &str,
  depth: usize,
  source: &str,
) -> RenderResult<usize> {
  add_table_value(
    total,
    key,
    crate::rulepack_render::serialization::encoded_string_bound(value, source)?,
    depth,
    source,
  )
}

fn add_table_value(
  mut total: usize,
  key: &str,
  value_estimate: usize,
  depth: usize,
  source: &str,
) -> RenderResult<usize> {
  let key_estimate = checked_mul(
    crate::rulepack_render::serialization::encoded_string_bound(key, source)?,
    depth.checked_add(1).ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} projected option depth overflow"
      ))
    })?,
    "projected option byte count",
    source,
  )?;
  for added in [key_estimate, TOML_FIELD_OVERHEAD, value_estimate] {
    total = checked_add(total, added, "projected option byte count", source)?;
  }
  Ok(total)
}

fn add_override_bytes(
  mut total: usize,
  override_item: &RulepackOverride,
  limit: usize,
  source: &str,
) -> RenderResult<usize> {
  if let Some(value) = &override_item.selector.rulepack {
    total = add_text(total, value, limit, source)?;
  }
  total = add_text_list(total, &override_item.selector.tags, limit, source)?;
  if let Some(value) = &override_item.selector.rule_id {
    total = add_text(total, value, limit, source)?;
  }
  if let Some(value) = &override_item.selector.rule_name {
    total = add_text(total, value, limit, source)?;
  }
  if let Some(action) = &override_item.action {
    total = add_text(total, &action.action_type, limit, source)?;
    if let Some(value) = &action.name {
      total = add_text(total, value, limit, source)?;
    }
  }
  if let Some(value) = &override_item.rate {
    total = add_text(total, value, limit, source)?;
  }
  if let Some(value) = &override_item.body {
    total = add_text(total, value, limit, source)?;
  }
  Ok(total)
}

fn add_exception_bytes(
  mut total: usize,
  exception: &RulepackException,
  limit: usize,
  source: &str,
) -> RenderResult<usize> {
  total = add_text(total, &exception.name, limit, source)?;
  total = add_text_list(total, &exception.rule_ids, limit, source)?;
  total = add_text_list(total, &exception.rule_names, limit, source)?;
  total = add_text_list(total, &exception.tags, limit, source)?;
  total = add_text_list(total, &exception.routes, limit, source)?;
  total = add_text_list(total, &exception.methods, limit, source)?;
  total = add_text_list(total, &exception.path_prefixes, limit, source)?;
  total = add_text_list(total, &exception.source_cidrs, limit, source)?;
  total = add_text(total, &exception.reason, limit, source)?;
  if let Some(value) = &exception.expires_at {
    total = add_text(total, value, limit, source)?;
  }
  Ok(total)
}

fn add_text_list(
  mut total: usize,
  values: &[String],
  limit: usize,
  source: &str,
) -> RenderResult<usize> {
  total = add_local_option_bytes(
    total,
    checked_mul(
      values.len(),
      LOCAL_STRING_ITEM_BYTES,
      "local option byte count",
      source,
    )?,
    limit,
    source,
  )?;
  for value in values {
    total = add_text(total, value, limit, source)?;
  }
  Ok(total)
}

fn add_text(total: usize, value: &str, limit: usize, source: &str) -> RenderResult<usize> {
  add_local_option_bytes(total, value.len(), limit, source)
}

fn add_local_option_bytes(
  total: usize,
  added: usize,
  limit: usize,
  source: &str,
) -> RenderResult<usize> {
  let total = checked_add(total, added, "local option byte count", source)?;
  if total > limit {
    return Err(crate::rulepack_render::RulepackRenderError::new(format!(
      "{source} exceeds local option bytes limit of {limit}"
    )));
  }
  Ok(total)
}

fn checked_add(left: usize, right: usize, label: &str, source: &str) -> RenderResult<usize> {
  left.checked_add(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!("{source} {label} overflow"))
  })
}

fn checked_mul(left: usize, right: usize, label: &str, source: &str) -> RenderResult<usize> {
  left.checked_mul(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!("{source} {label} overflow"))
  })
}
