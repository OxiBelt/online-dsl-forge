use crate::rulepack_render::error::{RenderResult, fail};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RulepackRenderLimits {
  pub max_manifest_bytes: usize,
  pub max_referenced_file_bytes: usize,
  pub max_variable_value_bytes: usize,
  pub max_total_variable_bytes: usize,
  pub max_variables: usize,
  pub max_rulepack_files: usize,
  pub max_placeholders: usize,
  pub max_total_input_bytes: usize,
  pub max_total_output_bytes: usize,
}

impl Default for RulepackRenderLimits {
  fn default() -> Self {
    Self {
      max_manifest_bytes: 8 * 1024 * 1024,
      max_referenced_file_bytes: 8 * 1024 * 1024,
      max_variable_value_bytes: 1024 * 1024,
      max_total_variable_bytes: 8 * 1024 * 1024,
      max_variables: 4096,
      max_rulepack_files: 4096,
      max_placeholders: 262_144,
      max_total_input_bytes: 64 * 1024 * 1024,
      max_total_output_bytes: 64 * 1024 * 1024,
    }
  }
}

pub(crate) struct RenderMeter {
  limits: RulepackRenderLimits,
  input_bytes: usize,
  output_bytes: usize,
  render_work_bytes: usize,
  placeholder_count: usize,
}

impl RenderMeter {
  pub(crate) fn new(limits: RulepackRenderLimits) -> Self {
    Self {
      limits,
      input_bytes: 0,
      output_bytes: 0,
      render_work_bytes: 0,
      placeholder_count: 0,
    }
  }

  pub(crate) fn limits(&self) -> RulepackRenderLimits {
    self.limits
  }

  pub(crate) fn manifest(&mut self, raw: &str, source: &str) -> RenderResult<()> {
    check_limit(
      raw.len(),
      self.limits.max_manifest_bytes,
      "manifest bytes",
      source,
    )?;
    self.add_input(raw.len(), source)
  }

  pub(crate) fn file(
    &mut self,
    raw: &str,
    source: &str,
    add_to_aggregate: bool,
  ) -> RenderResult<()> {
    check_limit(
      raw.len(),
      self.limits.max_referenced_file_bytes,
      "referenced file bytes",
      source,
    )?;
    if add_to_aggregate {
      self.add_input(raw.len(), source)?;
    }
    Ok(())
  }

  pub(crate) fn admit_supplied_variables(
    &mut self,
    variables: &BTreeMap<String, String>,
    source: &str,
  ) -> RenderResult<()> {
    let total = self.check_variable_entries(
      variables
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str())),
      variables.len(),
      source,
    )?;
    self.add_input(total, source)
  }

  pub(crate) fn check_resolved_variables(
    &self,
    variables: &BTreeMap<String, String>,
    count: usize,
    source: &str,
  ) -> RenderResult<()> {
    self
      .check_variable_entries(
        variables
          .iter()
          .map(|(name, value)| (name.as_str(), value.as_str())),
        count,
        source,
      )
      .map(drop)
  }

  pub(crate) fn check_variable_entries<'a>(
    &self,
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
    count: usize,
    source: &str,
  ) -> RenderResult<usize> {
    check_limit(count, self.limits.max_variables, "variables", source)?;
    let mut total = 0usize;
    for (name, value) in entries {
      check_limit(
        value.len(),
        self.limits.max_variable_value_bytes,
        "variable value bytes",
        source,
      )?;
      total = checked_add(total, name.len(), "variable byte count", source)?;
      total = checked_add(total, value.len(), "variable byte count", source)?;
      check_limit(
        total,
        self.limits.max_total_variable_bytes,
        "total variable bytes",
        source,
      )?;
    }
    Ok(total)
  }

  pub(crate) fn files(&self, rules: usize, groups: usize, source: &str) -> RenderResult<()> {
    let count = checked_add(rules, groups, "rulepack file count", source)?;
    check_limit(
      count,
      self.limits.max_rulepack_files,
      "rulepack files",
      source,
    )
  }

  pub(crate) fn placeholder(&mut self, source: &str) -> RenderResult<()> {
    self.placeholder_count = checked_add(self.placeholder_count, 1, "placeholder count", source)?;
    check_limit(
      self.placeholder_count,
      self.limits.max_placeholders,
      "placeholders",
      source,
    )
  }

  pub(crate) fn output(&mut self, bytes: usize, source: &str) -> RenderResult<()> {
    self.output_bytes = checked_add(
      self.output_bytes,
      bytes,
      "rendered output byte count",
      source,
    )?;
    check_limit(
      self.output_bytes,
      self.limits.max_total_output_bytes,
      "total rendered output bytes",
      source,
    )
  }

  pub(crate) fn reserve_render_work(&mut self, bytes: usize, source: &str) -> RenderResult<()> {
    self.render_work_bytes = checked_add(
      self.render_work_bytes,
      bytes,
      "render work byte count",
      source,
    )?;
    check_limit(
      self.render_work_bytes,
      self.limits.max_total_output_bytes,
      "aggregate retained render bytes",
      source,
    )
  }

  pub(crate) fn remaining_render_work(&self) -> usize {
    self
      .limits
      .max_total_output_bytes
      .saturating_sub(self.render_work_bytes)
  }

  pub(crate) fn max_single_output(&self) -> usize {
    self.limits.max_total_output_bytes
  }

  fn add_input(&mut self, bytes: usize, source: &str) -> RenderResult<()> {
    self.input_bytes = checked_add(
      self.input_bytes,
      bytes,
      "aggregate input byte count",
      source,
    )?;
    check_limit(
      self.input_bytes,
      self.limits.max_total_input_bytes,
      "aggregate input bytes",
      source,
    )
  }
}

fn checked_add(left: usize, right: usize, label: &str, source: &str) -> RenderResult<usize> {
  left.checked_add(right).ok_or_else(|| {
    crate::rulepack_render::RulepackRenderError::new(format!("{source} {label} overflow"))
  })
}

fn check_limit(value: usize, limit: usize, label: &str, source: &str) -> RenderResult<()> {
  if value > limit {
    fail(format!("{source} exceeds {label} limit of {limit}"))
  } else {
    Ok(())
  }
}
