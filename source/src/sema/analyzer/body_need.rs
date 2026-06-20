use crate::parser::SourceSpan;
use crate::sema::profile::{BodyAccess, BodyNeedSummary, BodyTarget};
use crate::sema::schema::CapabilityMeta;

use super::AnalyzeState;
use super::support::ObjectOrigin;

impl<'a> AnalyzeState<'a> {
  pub(super) fn merge_body_access_for_origin(
    &self,
    body_need: &mut BodyNeedSummary,
    origin: Option<ObjectOrigin>,
    field: &str,
    span: SourceSpan,
  ) {
    let Some(origin) = origin else {
      return;
    };
    match (origin, field) {
      (ObjectOrigin::RequestBody, "Size") => {
        body_need.merge_target(BodyTarget::Request, BodyAccess::SizeOnly)
      }
      (ObjectOrigin::ResponseBody, "Size") => {
        body_need.merge_target(BodyTarget::Response, BodyAccess::SizeOnly)
      }
      (ObjectOrigin::RequestBody, "Bytes" | "Text" | "IsTruncated") => {
        body_need.merge_target(BodyTarget::Request, BodyAccess::PrefixBytes)
      }
      (ObjectOrigin::ResponseBody, "Bytes" | "Text" | "IsTruncated") => {
        body_need.merge_target(BodyTarget::Response, BodyAccess::PrefixBytes)
      }
      (ObjectOrigin::Stream, "Payload") => {
        body_need.merge_target(BodyTarget::Stream, BodyAccess::PrefixBytes)
      }
      _ => {
        let _ = span;
      }
    }
  }

  pub(super) fn merge_body_access_for_method(
    &self,
    body_need: &mut BodyNeedSummary,
    origin: Option<ObjectOrigin>,
    capability: &CapabilityMeta,
  ) {
    let Some(origin) = origin else {
      return;
    };
    let Some(target) = origin.body_target() else {
      return;
    };
    body_need.merge_target(target, capability.body_access);
  }
}
