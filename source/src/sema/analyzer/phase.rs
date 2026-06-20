use crate::parser::{Diagnostic, SourceSpan};
use crate::sema::profile::Phase;
use crate::sema::schema::CapabilityMeta;

use super::AnalyzeState;
use super::support::{ObjectOrigin, capability_kind_label};

impl<'a> AnalyzeState<'a> {
  pub(super) fn validate_variable_phase(&mut self, name: &str, span: SourceSpan) {
    let Some(phase) = self.analyzer.profile.active_phase() else {
      return;
    };
    if phase == Phase::Request && name == "Response" {
      self.diagnostics.push(Diagnostic::new(
        "Response is unavailable in request phase",
        span,
      ));
    }
    if phase != Phase::Stream && name == "Stream" {
      self.diagnostics.push(Diagnostic::new(
        "Stream is available only in stream phase",
        span,
      ));
    }
    if phase == Phase::Stream && name == "Response" {
      self.diagnostics.push(Diagnostic::new(
        "Response is unavailable in stream phase",
        span,
      ));
    }
    if let Some(variable) = self.schema.variable(name)
      && !variable.is_available_in(phase)
    {
      self.diagnostics.push(Diagnostic::new(
        format!("variable {name} is unavailable in {phase:?} phase"),
        span,
      ));
    }
  }

  pub(super) fn validate_capability_phase(
    &mut self,
    capability: &CapabilityMeta,
    span: SourceSpan,
  ) {
    let Some(phase) = self.analyzer.profile.active_phase() else {
      return;
    };
    if !capability.is_available_in(phase) {
      self.diagnostics.push(Diagnostic::new(
        format!(
          "{} {} is unavailable in {phase:?} phase",
          capability_kind_label(capability.kind),
          capability.name
        ),
        span,
      ));
    }
  }

  pub(super) fn validate_origin_phase(&mut self, origin: Option<ObjectOrigin>, span: SourceSpan) {
    if self.analyzer.profile.active_phase() == Some(Phase::Stream)
      && matches!(
        origin,
        Some(ObjectOrigin::RequestBody | ObjectOrigin::RequestBodyBytes)
      )
    {
      self.diagnostics.push(Diagnostic::new(
        "Request.Body is unavailable in stream phase",
        span,
      ));
    }
  }
}
