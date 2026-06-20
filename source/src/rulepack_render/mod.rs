//! In-memory rulepack manifest rendering and referenced-file expansion.

mod error;
mod exceptions;
mod files;
mod input;
mod overrides;
mod provenance;
mod types;
mod validation;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

pub use error::RulepackRenderError;
use error::{RenderResult, fail};
pub use files::{BlobFileResolver, BlobStore, FileResolver, MemoryFileResolver};
pub use types::{
  RenderedRulepackBundle, RenderedRulepackFile, RulepackActionSelector, RulepackBinding,
  RulepackBindingKind, RulepackDiscovery, RulepackException, RulepackGroupFileSummary,
  RulepackInputMetadata, RulepackInspection, RulepackMode, RulepackModeOverride, RulepackOverride,
  RulepackOverrideSelector, RulepackPhase, RulepackProfile, RulepackReferencedFile,
  RulepackReferencedFileKind, RulepackRenderOptions, RulepackRuleSummary, RulepackSourceProvenance,
  RulepackSummary, RulepackVariable,
};
use types::{RulepackDocument, RulepackGroupFile, RulepackMetadata, RulepackRule};
use validation::{validate_label, validate_non_empty};

const SUPPORTED_SCHEMA_VERSION: u32 = 2;

pub fn inspect_rulepack_inputs(raw: &str, source: &str) -> RenderResult<RulepackInputMetadata> {
  let value = toml_value(raw, source)?;
  let document = document_from_value(value, source)?;
  validate_document_shape(&document, source, ExceptionValidation::Skip)?;
  Ok(RulepackInputMetadata {
    summary: summary_from_document(&document, Vec::new()),
    variables: document.variables,
    bindings: document.bindings,
    profiles: document.profiles,
  })
}

pub fn inspect_rulepack(
  raw: &str,
  source: &str,
  options: RulepackRenderOptions,
) -> RenderResult<RulepackInspection> {
  let parsed = ParsedRulepack::parse(raw, source, options)?;
  Ok(RulepackInspection {
    summary: parsed.summary(Vec::new()),
    rendered: parsed.rendered,
  })
}

pub fn render_rulepack_for_install(
  raw: &str,
  source: &str,
  mut options: RulepackRenderOptions,
) -> RenderResult<String> {
  options.pin_variables = true;
  Ok(ParsedRulepack::parse(raw, source, options)?.rendered)
}

pub fn referenced_rulepack_files(
  raw: &str,
  source: &str,
  options: RulepackRenderOptions,
) -> RenderResult<Vec<RulepackReferencedFile>> {
  let parsed = ParsedRulepack::parse(raw, source, options)?;
  parsed.validate_references()?;
  files::referenced_rulepack_files(&parsed.document)
}

pub fn render_rulepack_bundle<R: FileResolver + ?Sized>(
  raw: &str,
  source: &str,
  options: RulepackRenderOptions,
  resolver: &R,
) -> RenderResult<RenderedRulepackBundle> {
  let parsed = ParsedRulepack::parse(raw, source, options)?;
  parsed.validate_references()?;
  let mut loaded_files = Vec::new();
  let mut rendered_files = Vec::new();
  for rule in &parsed.document.rules {
    let label = format!(
      "rulepack {} rule {}",
      parsed.document.rulepack.name, rule.name
    );
    files::validate_rule_content_or_path(&label, rule)?;
    let Some(file) = rule_file(rule) else {
      continue;
    };
    loaded_files.push(file.path.clone());
    rendered_files.push(files::embedded_or_resolved_file(
      file,
      rule.content.as_deref(),
      resolver,
      &parsed.variables,
    )?);
  }
  for group_file in &parsed.document.group_files {
    let label = format!("rulepack {} group file", parsed.document.rulepack.name);
    files::validate_group_content_or_path(&label, group_file)?;
    let Some(file) = group_file_ref(group_file) else {
      continue;
    };
    loaded_files.push(file.path.clone());
    rendered_files.push(files::embedded_or_resolved_file(
      file,
      group_file.content.as_deref(),
      resolver,
      &parsed.variables,
    )?);
  }
  Ok(RenderedRulepackBundle {
    summary: parsed.summary(loaded_files),
    manifest: parsed.rendered,
    files: rendered_files,
  })
}

pub fn render_text(raw: &str, variables: &BTreeMap<String, String>) -> String {
  let mut rendered = raw.to_string();
  for (name, value) in variables {
    rendered = rendered.replace(&format!("{{{{{name}}}}}"), value);
  }
  rendered
}

struct ParsedRulepack {
  document: RulepackDocument,
  rendered: String,
  variables: BTreeMap<String, String>,
}

impl ParsedRulepack {
  fn parse(raw: &str, source: &str, options: RulepackRenderOptions) -> RenderResult<Self> {
    let mut value = toml_value(raw, source)?;
    let initial = document_from_value(value.clone(), source)?;
    validate_document_shape(&initial, source, ExceptionValidation::Skip)?;
    exceptions::validate_rulepack_exception_list(source, &options.local_exceptions)?;
    overrides::validate_rulepack_overrides(
      source,
      &initial.rulepack.name,
      &options.local_overrides,
    )?;
    let variables = resolve_variables(
      &initial.variables,
      &initial.bindings,
      &options.variables,
      source,
    )?;
    render_toml_strings(&mut value, &variables);
    overrides::apply_overrides(
      &mut value,
      source,
      &initial.rulepack.name,
      &initial.overrides,
      &options.local_overrides,
    )?;
    apply_mode_override(&mut value, options.mode_override)?;
    exceptions::append_local_exceptions(&mut value, source, &options.local_exceptions)?;
    if options.pin_variables {
      pin_variable_defaults(&mut value, &variables)?;
      if let Some(table) = value.as_table_mut() {
        table.remove("bindings");
        table.remove("profiles");
        table.remove("overrides");
      }
    }
    if let Some(commit) = options.source_commit {
      set_rulepack_string(&mut value, "source_commit", commit)?;
    }
    if let Some(provenance) = options.source_provenance {
      provenance::set_rulepack_provenance(&mut value, provenance)?;
    }
    let document = document_from_value(value.clone(), source)?;
    validate_document_shape(&document, source, ExceptionValidation::Full)?;
    let rendered = toml::to_string_pretty(&value)
      .map_err(|error| RulepackRenderError::new(format!("failed to render {source}: {error}")))?;
    Ok(Self {
      document,
      rendered,
      variables,
    })
  }

  fn validate_references(&self) -> RenderResult<()> {
    for rule in &self.document.rules {
      files::validate_rule_content_or_path(
        &format!(
          "rulepack {} rule {}",
          self.document.rulepack.name, rule.name
        ),
        rule,
      )?;
    }
    for group_file in &self.document.group_files {
      files::validate_group_content_or_path(
        &format!("rulepack {} group file", self.document.rulepack.name),
        group_file,
      )?;
    }
    Ok(())
  }

  fn summary(&self, loaded_files: Vec<PathBuf>) -> RulepackSummary {
    summary_from_document(&self.document, loaded_files)
  }
}

fn toml_value(raw: &str, source: &str) -> RenderResult<toml::Value> {
  toml::from_str(raw)
    .map_err(|error| RulepackRenderError::new(format!("failed to parse {source}: {error}")))
}

fn document_from_value(value: toml::Value, source: &str) -> RenderResult<RulepackDocument> {
  input::reject_legacy_variable_discovery(&value, source)?;
  value
    .try_into()
    .map_err(|error| RulepackRenderError::new(format!("failed to decode {source}: {error}")))
}

#[derive(Clone, Copy)]
enum ExceptionValidation {
  Skip,
  Full,
}

fn validate_document_shape(
  document: &RulepackDocument,
  source: &str,
  exception_validation: ExceptionValidation,
) -> RenderResult<()> {
  validate_metadata(source, &document.rulepack)?;
  if document.rules.is_empty() && document.group_files.is_empty() {
    return fail(format!(
      "{source} must contain at least one [[rules]] or [[group_files]] entry"
    ));
  }
  input::validate_rulepack_inputs(
    source,
    &document.variables,
    &document.bindings,
    &document.profiles,
  )?;
  overrides::validate_rulepack_overrides(source, &document.rulepack.name, &document.overrides)?;
  if matches!(exception_validation, ExceptionValidation::Full) {
    exceptions::validate_rulepack_exceptions(source, &document.exceptions, &document.rules)?;
  }
  let mut rule_names = HashSet::new();
  for rule in &document.rules {
    validate_label(source, "rules.name", &rule.name)?;
    if !rule_names.insert(rule.name.clone()) {
      return fail(format!("{source} contains duplicate rule {}", rule.name));
    }
    for tag in &rule.tags {
      validate_label(source, "rules.tags", tag)?;
    }
  }
  Ok(())
}

fn validate_metadata(source: &str, metadata: &RulepackMetadata) -> RenderResult<()> {
  if metadata.schema_version != SUPPORTED_SCHEMA_VERSION {
    return fail(format!(
      "{source} uses unsupported rulepack schema_version {}; only schema_version {SUPPORTED_SCHEMA_VERSION} is supported",
      metadata.schema_version
    ));
  }
  validate_label(source, "rulepack.name", &metadata.name)?;
  validate_non_empty(source, "rulepack.version", &metadata.version)?;
  for target in &metadata.targets {
    validate_label(source, "rulepack.targets", target)?;
  }
  for requirement in &metadata.requires {
    validate_label(source, "rulepack.requires", requirement)?;
  }
  provenance::validate_manifest_provenance(
    source,
    metadata.source_url.as_deref(),
    metadata.source_sha256.as_deref(),
    metadata.source_openpgp_signature_url.as_deref(),
    metadata.source_openpgp_signer_fingerprint.as_deref(),
  )?;
  Ok(())
}

fn summary_from_document(
  document: &RulepackDocument,
  loaded_files: Vec<PathBuf>,
) -> RulepackSummary {
  RulepackSummary {
    name: document.rulepack.name.clone(),
    version: document.rulepack.version.clone(),
    description: document.rulepack.description.clone(),
    targets: document.rulepack.targets.clone(),
    requires: document.rulepack.requires.clone(),
    default_mode: document.rulepack.default_mode.as_str().to_string(),
    rules: document.rules.len(),
    group_files: document.group_files.len(),
    exceptions: document.exceptions.len(),
    loaded_files,
    source_commit: document.rulepack.source_commit.clone(),
    source_url: document.rulepack.source_url.clone(),
    source_sha256: document.rulepack.source_sha256.clone(),
    source_openpgp_signature_url: document.rulepack.source_openpgp_signature_url.clone(),
    source_openpgp_signer_fingerprint: document.rulepack.source_openpgp_signer_fingerprint.clone(),
  }
}

fn resolve_variables(
  variables: &[RulepackVariable],
  bindings: &[RulepackBinding],
  overrides: &BTreeMap<String, String>,
  source: &str,
) -> RenderResult<BTreeMap<String, String>> {
  let mut values = BTreeMap::new();
  let known = variables
    .iter()
    .map(|variable| variable.name.as_str())
    .collect::<HashSet<_>>();
  let binding_targets = bindings
    .iter()
    .map(|binding| binding.bind_as.as_str())
    .collect::<HashSet<_>>();
  for key in overrides.keys() {
    if !known.contains(key.as_str()) && !binding_targets.contains(key.as_str()) {
      return fail(format!(
        "{source} does not declare variable or binding render target {key}"
      ));
    }
  }
  for variable in variables {
    let value = overrides
      .get(&variable.name)
      .cloned()
      .or_else(|| variable.default.clone());
    match value {
      Some(value) => {
        input::validate_variable_value(source, variable, &value)?;
        values.insert(variable.name.clone(), value);
      }
      None if variable.required => {
        return fail(format!("{source} requires variable {}", variable.name));
      }
      None => {}
    }
  }
  for binding in bindings {
    match overrides.get(&binding.bind_as) {
      Some(value) => {
        values.insert(binding.bind_as.clone(), value.clone());
      }
      None if binding.required => {
        return fail(format!("{source} requires binding {}", binding.name));
      }
      None => {}
    }
  }
  Ok(values)
}

fn render_toml_strings(value: &mut toml::Value, variables: &BTreeMap<String, String>) {
  match value {
    toml::Value::String(text) => {
      *text = render_text(text, variables);
    }
    toml::Value::Array(values) => {
      for value in values {
        render_toml_strings(value, variables);
      }
    }
    toml::Value::Table(table) => {
      for (_, value) in table.iter_mut() {
        render_toml_strings(value, variables);
      }
    }
    toml::Value::Integer(_)
    | toml::Value::Float(_)
    | toml::Value::Boolean(_)
    | toml::Value::Datetime(_) => {}
  }
}

fn apply_mode_override(
  value: &mut toml::Value,
  mode_override: Option<RulepackModeOverride>,
) -> RenderResult<()> {
  let Some(mode_override) = mode_override else {
    return Ok(());
  };
  set_rulepack_string(
    value,
    "default_mode",
    mode_override.mode.as_str().to_string(),
  )?;
  if mode_override.force {
    let Some(rules) = value.get_mut("rules").and_then(toml::Value::as_array_mut) else {
      return Ok(());
    };
    for rule in rules {
      let Some(table) = rule.as_table_mut() else {
        return fail("rulepack rules entries must be tables");
      };
      table.insert(
        "mode".to_string(),
        toml::Value::String(mode_override.mode.as_str().to_string()),
      );
    }
  }
  Ok(())
}

fn pin_variable_defaults(
  value: &mut toml::Value,
  variables: &BTreeMap<String, String>,
) -> RenderResult<()> {
  let Some(items) = value
    .get_mut("variables")
    .and_then(toml::Value::as_array_mut)
  else {
    return Ok(());
  };
  for item in items {
    let Some(table) = item.as_table_mut() else {
      return fail("rulepack variables entries must be tables");
    };
    let Some(name) = table.get("name").and_then(toml::Value::as_str) else {
      return fail("rulepack variable entry is missing name");
    };
    if let Some(value) = variables.get(name) {
      table.insert("default".to_string(), toml::Value::String(value.clone()));
      table.insert("required".to_string(), toml::Value::Boolean(false));
    }
  }
  Ok(())
}

fn set_rulepack_string(
  value: &mut toml::Value,
  key: &str,
  field_value: String,
) -> RenderResult<()> {
  let Some(table) = value
    .get_mut("rulepack")
    .and_then(toml::Value::as_table_mut)
  else {
    return fail("rulepack manifest is missing [rulepack]");
  };
  table.insert(key.to_string(), toml::Value::String(field_value));
  Ok(())
}

fn rule_file(rule: &RulepackRule) -> Option<RulepackReferencedFile> {
  rule.path.as_ref().map(|path| RulepackReferencedFile {
    kind: RulepackReferencedFileKind::Rule,
    path: path.clone(),
  })
}

fn group_file_ref(group_file: &RulepackGroupFile) -> Option<RulepackReferencedFile> {
  group_file.path.as_ref().map(|path| RulepackReferencedFile {
    kind: RulepackReferencedFileKind::Group,
    path: path.clone(),
  })
}
