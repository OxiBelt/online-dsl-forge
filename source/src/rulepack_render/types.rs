use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RulepackMode {
  Monitor,
  Enforcing,
}

impl RulepackMode {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Monitor => "monitor",
      Self::Enforcing => "enforcing",
    }
  }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RulepackPhase {
  Request,
  Response,
  Stream,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct RulepackModeOverride {
  pub mode: RulepackMode,
  pub force: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RulepackRenderOptions {
  pub variables: BTreeMap<String, String>,
  pub local_overrides: Vec<RulepackOverride>,
  pub local_exceptions: Vec<RulepackException>,
  pub mode_override: Option<RulepackModeOverride>,
  pub source_commit: Option<String>,
  pub source_provenance: Option<RulepackSourceProvenance>,
  pub pin_variables: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RulepackSummary {
  pub name: String,
  pub version: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  pub targets: Vec<String>,
  pub requires: Vec<String>,
  pub default_mode: String,
  pub rules: usize,
  pub group_files: usize,
  pub exceptions: usize,
  pub loaded_files: Vec<PathBuf>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_commit: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_sha256: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_openpgp_signature_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_openpgp_signer_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RulepackInspection {
  pub summary: RulepackSummary,
  pub rendered: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RulepackInputMetadata {
  pub summary: RulepackSummary,
  pub variables: Vec<RulepackVariable>,
  pub bindings: Vec<RulepackBinding>,
  pub profiles: Vec<RulepackProfile>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulepackVariable {
  pub name: String,
  #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
  pub value_type: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default: Option<String>,
  #[serde(default)]
  pub required: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompt: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulepackBinding {
  pub name: String,
  pub kind: RulepackBindingKind,
  pub bind_as: String,
  #[serde(default)]
  pub required: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompt: Option<String>,
  #[serde(default)]
  pub discovery: RulepackDiscovery,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RulepackBindingKind {
  Route,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulepackDiscovery {
  #[serde(default)]
  pub name_any: Vec<String>,
  #[serde(default)]
  pub host_contains_any: Vec<String>,
  #[serde(default)]
  pub upstream_contains_any: Vec<String>,
  #[serde(default)]
  pub path_prefix_any: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RulepackProfile {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<RulepackMode>,
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackOverride {
  pub selector: RulepackOverrideSelector,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub action: Option<RulepackActionSelector>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<RulepackMode>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub priority: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub enabled: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rate: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub burst: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub status: Option<u16>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub body: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackOverrideSelector {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rulepack: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tags: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rule_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rule_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackActionSelector {
  #[serde(rename = "type", alias = "action_type")]
  pub action_type: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackException {
  pub name: String,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub rule_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub rule_names: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tags: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub routes: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub methods: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub path_prefixes: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub source_cidrs: Vec<String>,
  pub reason: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RulepackSourceProvenance {
  pub source_url: String,
  pub source_sha256: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_openpgp_signature_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub source_openpgp_signer_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RulepackDocument {
  pub rulepack: RulepackMetadata,
  #[serde(default)]
  pub variables: Vec<RulepackVariable>,
  #[serde(default)]
  pub bindings: Vec<RulepackBinding>,
  #[serde(default)]
  pub profiles: Vec<RulepackProfile>,
  #[serde(default)]
  pub overrides: Vec<RulepackOverride>,
  #[serde(default)]
  pub exceptions: Vec<RulepackException>,
  #[serde(default)]
  pub rules: Vec<RulepackRule>,
  #[serde(default)]
  pub group_files: Vec<RulepackGroupFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RulepackMetadata {
  pub schema_version: u32,
  pub name: String,
  pub version: String,
  #[serde(default)]
  pub description: Option<String>,
  #[serde(default)]
  pub targets: Vec<String>,
  #[serde(default)]
  pub requires: Vec<String>,
  #[serde(default = "default_rulepack_mode")]
  pub default_mode: RulepackMode,
  #[serde(default)]
  pub source_commit: Option<String>,
  #[serde(default)]
  pub source_url: Option<String>,
  #[serde(default)]
  pub source_sha256: Option<String>,
  #[serde(default)]
  pub source_openpgp_signature_url: Option<String>,
  #[serde(default)]
  pub source_openpgp_signer_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackRuleSummary {
  pub name: String,
  pub phase: RulepackPhase,
  pub priority: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tags: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<RulepackMode>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub content: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<PathBuf>,
}

pub(crate) type RulepackRule = RulepackRuleSummary;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulepackGroupFileSummary {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub content: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<PathBuf>,
}

pub(crate) type RulepackGroupFile = RulepackGroupFileSummary;

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RulepackReferencedFile {
  pub kind: RulepackReferencedFileKind,
  pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RulepackReferencedFileKind {
  Rule,
  Group,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RenderedRulepackFile {
  pub kind: RulepackReferencedFileKind,
  pub path: PathBuf,
  pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderedRulepackBundle {
  pub summary: RulepackSummary,
  pub manifest: String,
  pub files: Vec<RenderedRulepackFile>,
}

pub(crate) fn default_rulepack_mode() -> RulepackMode {
  RulepackMode::Monitor
}
