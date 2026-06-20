use std::collections::HashSet;

use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::types::{
  RulepackBinding, RulepackDiscovery, RulepackProfile, RulepackVariable,
};
use crate::rulepack_render::validation::{
  validate_cidr, validate_human_text, validate_label, validate_rate,
};

pub(crate) fn reject_legacy_variable_discovery(
  value: &toml::Value,
  source: &str,
) -> RenderResult<()> {
  for variable in value
    .get("variables")
    .and_then(toml::Value::as_array)
    .into_iter()
    .flatten()
  {
    let Some(table) = variable.as_table() else {
      continue;
    };
    let name = table
      .get("name")
      .and_then(toml::Value::as_str)
      .unwrap_or("<unknown>");
    if table.contains_key("discovery") {
      return fail(format!(
        "{source} variable {name} uses [variables.discovery]; route and other environment objects must be declared with explicit [[bindings]] and bind_as"
      ));
    }
  }
  Ok(())
}

pub(crate) fn validate_rulepack_inputs(
  source: &str,
  variables: &[RulepackVariable],
  bindings: &[RulepackBinding],
  profiles: &[RulepackProfile],
) -> RenderResult<()> {
  let mut variable_names = HashSet::new();
  for variable in variables {
    validate_label(source, "variables.name", &variable.name)?;
    validate_variable_type(source, variable)?;
    validate_optional_human_text(
      source,
      "variables.description",
      variable.description.as_deref(),
    )?;
    validate_optional_human_text(source, "variables.prompt", variable.prompt.as_deref())?;
    if let Some(default) = &variable.default {
      validate_variable_value(source, variable, default)?;
    }
    if !variable_names.insert(variable.name.clone()) {
      return fail(format!(
        "{source} contains duplicate variable {}",
        variable.name
      ));
    }
  }

  let mut binding_names = HashSet::new();
  let mut binding_targets = HashSet::new();
  for binding in bindings {
    validate_label(source, "bindings.name", &binding.name)?;
    validate_label(source, "bindings.bind_as", &binding.bind_as)?;
    if variable_names.contains(&binding.bind_as) {
      return fail(format!(
        "{source} binding {} bind_as {} conflicts with a declared variable; route and other environment objects must use [[bindings]], while [[variables]] is only for scalar values",
        binding.name, binding.bind_as
      ));
    }
    if !binding_targets.insert(binding.bind_as.clone()) {
      return fail(format!(
        "{source} contains duplicate binding render target {}",
        binding.bind_as
      ));
    }
    if !binding_names.insert(binding.name.clone()) {
      return fail(format!(
        "{source} contains duplicate binding {}",
        binding.name
      ));
    }
    if variable_names.contains(&binding.name) {
      return fail(format!(
        "{source} binding {} conflicts with a declared variable; use distinct names for bind and var inputs",
        binding.name
      ));
    }
    validate_optional_human_text(
      source,
      "bindings.description",
      binding.description.as_deref(),
    )?;
    validate_optional_human_text(source, "bindings.prompt", binding.prompt.as_deref())?;
    validate_discovery(source, "bindings.discovery", &binding.discovery)?;
  }

  let mut profile_names = HashSet::new();
  for profile in profiles {
    validate_label(source, "profiles.name", &profile.name)?;
    if !profile_names.insert(profile.name.clone()) {
      return fail(format!(
        "{source} contains duplicate profile {}",
        profile.name
      ));
    }
    for (name, value) in &profile.values {
      let Some(variable) = variables.iter().find(|variable| variable.name == *name) else {
        return fail(format!(
          "{source} profile {} sets unknown variable {name}",
          profile.name
        ));
      };
      validate_variable_value(source, variable, value)?;
    }
  }

  Ok(())
}

pub(crate) fn validate_variable_value(
  source: &str,
  variable: &RulepackVariable,
  value: &str,
) -> RenderResult<()> {
  match variable.value_type.as_deref() {
    Some("cidr") => validate_cidr(value).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} variable {} must be a valid CIDR: {error}",
        variable.name
      ))
    }),
    Some("rate") => validate_rate(value).map_err(|error| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "{source} variable {} must be a valid rate: {error}",
        variable.name
      ))
    }),
    Some("string") | None => Ok(()),
    Some("route") => fail(format!(
      "{source} variable {} uses type = \"route\"; route objects must be declared with [[bindings]] and bind_as",
      variable.name
    )),
    Some(other) => fail(format!(
      "{source} variable {} uses unsupported type {}; supported types are string, cidr, and rate",
      variable.name, other
    )),
  }
}

fn validate_variable_type(source: &str, variable: &RulepackVariable) -> RenderResult<()> {
  let Some(value_type) = &variable.value_type else {
    return Ok(());
  };
  validate_label(source, "variables.type", value_type)?;
  match value_type.as_str() {
    "string" | "cidr" | "rate" => Ok(()),
    "route" => fail(format!(
      "{source} variable {} uses type = \"route\"; route objects must be declared with [[bindings]] and bind_as",
      variable.name
    )),
    _ => fail(format!(
      "{source} variable {} uses unsupported type {}; supported types are string, cidr, and rate",
      variable.name, value_type
    )),
  }
}

fn validate_optional_human_text(
  source: &str,
  field: &str,
  value: Option<&str>,
) -> RenderResult<()> {
  if let Some(value) = value {
    validate_human_text(source, field, value)?;
  }
  Ok(())
}

fn validate_discovery(
  source: &str,
  field: &str,
  discovery: &RulepackDiscovery,
) -> RenderResult<()> {
  for token in discovery
    .name_any
    .iter()
    .chain(discovery.host_contains_any.iter())
    .chain(discovery.upstream_contains_any.iter())
  {
    validate_discovery_token(source, field, token)?;
  }
  for prefix in &discovery.path_prefix_any {
    if prefix.trim().is_empty()
      || prefix.len() > 512
      || prefix.bytes().any(|byte| byte.is_ascii_control())
    {
      return fail(format!(
        "{source} {field}.path_prefix_any entries must be 1 to 512 printable bytes"
      ));
    }
    if !prefix.starts_with('/') {
      return fail(format!(
        "{source} {field}.path_prefix_any entries must start with /"
      ));
    }
  }
  Ok(())
}

fn validate_discovery_token(source: &str, field: &str, value: &str) -> RenderResult<()> {
  if value.trim().is_empty()
    || value.len() > 128
    || value.bytes().any(|byte| byte.is_ascii_control())
  {
    return fail(format!(
      "{source} {field} entries must be 1 to 128 printable bytes"
    ));
  }
  Ok(())
}
