use std::cell::Cell;

use regex::Regex;

use crate::parser::SourceSpan;
use crate::sema::{CompiledRegexCache, RegexFlavor, RegexPolicy, SecurityProfile};

use super::EvalError;

#[derive(Clone, Copy)]
pub struct RuntimeCallContext<'a> {
  profile: &'a SecurityProfile,
  regex_cache: &'a CompiledRegexCache,
  processed_bytes: &'a Cell<usize>,
  max_total_bytes: usize,
  span: SourceSpan,
}

impl<'a> RuntimeCallContext<'a> {
  pub(crate) fn new(
    profile: &'a SecurityProfile,
    regex_cache: &'a CompiledRegexCache,
    processed_bytes: &'a Cell<usize>,
    max_total_bytes: usize,
    span: SourceSpan,
  ) -> Self {
    Self {
      profile,
      regex_cache,
      processed_bytes,
      max_total_bytes,
      span,
    }
  }

  pub fn profile(&self) -> &'a SecurityProfile {
    self.profile
  }

  pub fn regex_policy(&self) -> RegexPolicy {
    self.profile.default_regex_policy
  }

  /// Borrow the verified regex cache for host-managed operations.
  ///
  /// Matching through the raw cache is not metered automatically. Charge the
  /// projected work first, or use [`Self::precompiled_regex_is_match`].
  pub fn regex_cache(&self) -> &'a CompiledRegexCache {
    self.regex_cache
  }

  pub fn span(&self) -> SourceSpan {
    self.span
  }

  /// Charge input-dependent handler work against the evaluation's cumulative
  /// resource budget before performing it.
  pub fn charge_work(&self, bytes: usize) -> Result<(), EvalError> {
    let processed = self
      .processed_bytes
      .get()
      .checked_add(bytes)
      .ok_or_else(|| EvalError::new("runtime work byte counter overflowed", self.span))?;
    if processed > self.max_total_bytes {
      return Err(EvalError::new(
        "runtime cumulative work byte limit exceeded",
        self.span,
      ));
    }
    self.processed_bytes.set(processed);
    Ok(())
  }

  /// Borrow one verified regex for host-managed operations.
  ///
  /// Direct use is not metered automatically. Charge the projected work first,
  /// or use [`Self::precompiled_regex_is_match`].
  pub fn precompiled_regex(&self, flavor: RegexFlavor, pattern: &str) -> Option<&'a Regex> {
    self.regex_cache.get(flavor, pattern)
  }

  /// Require one verified regex for host-managed operations.
  ///
  /// Direct use is not metered automatically. Charge the projected work first,
  /// or use [`Self::precompiled_regex_is_match`].
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

  /// Match with a verified regex after charging candidate bytes multiplied by
  /// the admitted pattern's source-byte complexity.
  pub fn precompiled_regex_is_match(
    &self,
    flavor: RegexFlavor,
    pattern: &str,
    haystack: &str,
  ) -> Result<bool, EvalError> {
    let candidate_bytes = haystack
      .len()
      .checked_add(1)
      .ok_or_else(|| EvalError::new("runtime regex work counter overflowed", self.span))?;
    let pattern_complexity = pattern
      .len()
      .checked_add(1)
      .ok_or_else(|| EvalError::new("runtime regex work counter overflowed", self.span))?;
    let work = candidate_bytes
      .checked_mul(pattern_complexity)
      .ok_or_else(|| EvalError::new("runtime regex work counter overflowed", self.span))?;
    self.charge_work(work)?;
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
