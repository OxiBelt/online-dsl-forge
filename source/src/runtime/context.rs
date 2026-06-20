use regex::Regex;

use crate::parser::SourceSpan;
use crate::sema::{CompiledRegexCache, RegexFlavor, RegexPolicy, SecurityProfile};

use super::EvalError;

#[derive(Clone, Copy)]
pub struct RuntimeCallContext<'a> {
  profile: &'a SecurityProfile,
  regex_cache: &'a CompiledRegexCache,
  span: SourceSpan,
}

impl<'a> RuntimeCallContext<'a> {
  pub(crate) fn new(
    profile: &'a SecurityProfile,
    regex_cache: &'a CompiledRegexCache,
    span: SourceSpan,
  ) -> Self {
    Self {
      profile,
      regex_cache,
      span,
    }
  }

  pub fn profile(&self) -> &'a SecurityProfile {
    self.profile
  }

  pub fn regex_policy(&self) -> RegexPolicy {
    self.profile.default_regex_policy
  }

  pub fn regex_cache(&self) -> &'a CompiledRegexCache {
    self.regex_cache
  }

  pub fn span(&self) -> SourceSpan {
    self.span
  }

  pub fn precompiled_regex(&self, flavor: RegexFlavor, pattern: &str) -> Option<&'a Regex> {
    self.regex_cache.get(flavor, pattern)
  }

  pub fn require_precompiled_regex(
    &self,
    flavor: RegexFlavor,
    pattern: &str,
  ) -> Result<&'a Regex, EvalError> {
    self.precompiled_regex(flavor, pattern).ok_or_else(|| {
      EvalError::new(
        format!(
          "precompiled {} regex is missing",
          regex_flavor_label(flavor)
        ),
        self.span,
      )
    })
  }

  pub fn precompiled_regex_is_match(
    &self,
    flavor: RegexFlavor,
    pattern: &str,
    haystack: &str,
  ) -> Result<bool, EvalError> {
    self
      .require_precompiled_regex(flavor, pattern)
      .map(|regex| regex.is_match(haystack))
  }
}

fn regex_flavor_label(flavor: RegexFlavor) -> &'static str {
  match flavor {
    RegexFlavor::Default => "default",
    RegexFlavor::HeaderName => "header_name",
  }
}
