use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityProfileId {
  GenericSafe,
  GenericTransform,
  WafRequest,
  WafResponse,
  WafStream,
  MitigationField,
  Custom(String),
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
  Generic,
  Request,
  Response,
  Stream,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyTarget {
  Request,
  Response,
  Stream,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyAccess {
  #[default]
  None,
  SizeOnly,
  PrefixBytes,
}

impl BodyAccess {
  pub fn merge(self, other: Self) -> Self {
    self.max(other)
  }

  pub fn reads_payload(self) -> bool {
    matches!(self, Self::PrefixBytes)
  }
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegexPolicy {
  Forbid,
  LiteralOnlyPrecompiled,
  DynamicWithBudget,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
  Required,
  BestEffort,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BodyNeedSummary {
  pub request: BodyAccess,
  pub response: BodyAccess,
  pub stream: BodyAccess,
}

impl BodyNeedSummary {
  pub fn merge(self, other: Self) -> Self {
    Self {
      request: self.request.merge(other.request),
      response: self.response.merge(other.response),
      stream: self.stream.merge(other.stream),
    }
  }

  pub fn merge_target(&mut self, target: BodyTarget, access: BodyAccess) {
    match target {
      BodyTarget::Request => self.request = self.request.merge(access),
      BodyTarget::Response => self.response = self.response.merge(access),
      BodyTarget::Stream => self.stream = self.stream.merge(access),
    }
  }

  pub fn reads_payload(self) -> bool {
    self.request.reads_payload() || self.response.reads_payload() || self.stream.reads_payload()
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecurityProfile {
  pub id: SecurityProfileId,
  pub allowed_phases: BTreeSet<Phase>,
  pub max_ast_nodes: usize,
  pub max_call_depth: usize,
  pub default_regex_policy: RegexPolicy,
  pub max_cost_units: u64,
  pub determinism: Determinism,
  pub fail_closed: bool,
}

impl SecurityProfile {
  pub fn generic_safe() -> Self {
    Self {
      id: SecurityProfileId::GenericSafe,
      allowed_phases: BTreeSet::from([Phase::Generic]),
      max_ast_nodes: 4096,
      max_call_depth: 64,
      default_regex_policy: RegexPolicy::DynamicWithBudget,
      max_cost_units: 100_000,
      determinism: Determinism::Required,
      fail_closed: true,
    }
  }

  pub fn generic_transform() -> Self {
    Self {
      id: SecurityProfileId::GenericTransform,
      max_ast_nodes: 8192,
      max_call_depth: 128,
      max_cost_units: 250_000,
      ..Self::generic_safe()
    }
  }

  pub fn waf_request() -> Self {
    Self::waf(SecurityProfileId::WafRequest, Phase::Request)
  }

  pub fn waf_response() -> Self {
    Self::waf(SecurityProfileId::WafResponse, Phase::Response)
  }

  pub fn waf_stream() -> Self {
    Self::waf(SecurityProfileId::WafStream, Phase::Stream)
  }

  pub fn oxirule_waf_request() -> Self {
    Self::oxirule_waf(SecurityProfileId::WafRequest, Phase::Request)
  }

  pub fn oxirule_waf_response() -> Self {
    Self::oxirule_waf(SecurityProfileId::WafResponse, Phase::Response)
  }

  pub fn oxirule_waf_stream() -> Self {
    Self::oxirule_waf(SecurityProfileId::WafStream, Phase::Stream)
  }

  pub fn mitigation_field(phase: Phase) -> Self {
    let allowed_phases = BTreeSet::from([phase]);
    Self {
      id: SecurityProfileId::MitigationField,
      allowed_phases,
      max_ast_nodes: 2048,
      max_call_depth: 32,
      default_regex_policy: RegexPolicy::LiteralOnlyPrecompiled,
      max_cost_units: 50_000,
      determinism: Determinism::Required,
      fail_closed: true,
    }
  }

  pub fn active_phase(&self) -> Option<Phase> {
    if self.allowed_phases.len() == 1 {
      self.allowed_phases.iter().next().copied()
    } else {
      None
    }
  }

  fn waf(id: SecurityProfileId, phase: Phase) -> Self {
    Self {
      id,
      allowed_phases: BTreeSet::from([phase]),
      max_ast_nodes: 4096,
      max_call_depth: 64,
      default_regex_policy: RegexPolicy::LiteralOnlyPrecompiled,
      max_cost_units: 100_000,
      determinism: Determinism::Required,
      fail_closed: true,
    }
  }

  fn oxirule_waf(id: SecurityProfileId, phase: Phase) -> Self {
    Self {
      default_regex_policy: RegexPolicy::DynamicWithBudget,
      ..Self::waf(id, phase)
    }
  }
}

impl Default for SecurityProfile {
  fn default() -> Self {
    Self::generic_safe()
  }
}
