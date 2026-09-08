use crate::parser::{AstExpression, Diagnostic, SourceSpan};
use crate::sema::profile::RegexPolicy;
use crate::sema::schema::{CapabilityMeta, RegexFlavor};
use crate::sema::verified::RegexLiteral;

use super::AnalyzeState;
use super::support::string_literal;

impl<'a> AnalyzeState<'a> {
  pub(super) fn validate_regex_args(
    &mut self,
    capability: &CapabilityMeta,
    args: &'a [AstExpression],
    span: SourceSpan,
  ) {
    for regex_arg in &capability.regex_args {
      let Some(arg) = args.get(regex_arg.index) else {
        continue;
      };
      let arg = self.resolve_inline_argument(arg);
      match self.analyzer.profile.default_regex_policy {
        RegexPolicy::Forbid => self.diagnostics.push(Diagnostic::new(
          "regex arguments are forbidden by profile",
          span,
        )),
        RegexPolicy::LiteralOnlyPrecompiled => {
          let Some(pattern) = string_literal(arg) else {
            self.diagnostics.push(Diagnostic::new(
              "regex argument must be a string literal",
              arg.span,
            ));
            continue;
          };
          self.admit_regex(pattern, regex_arg.flavor, arg.span, true);
        }
        RegexPolicy::DynamicWithBudget => {
          if let Some(pattern) = string_literal(arg) {
            self.admit_regex(pattern, regex_arg.flavor, arg.span, false);
          }
        }
      }
    }
  }

  fn admit_regex(
    &mut self,
    pattern: &str,
    flavor: RegexFlavor,
    span: SourceSpan,
    report_invalid: bool,
  ) {
    if let Some(error) = self
      .regex_attempts
      .get(&flavor)
      .and_then(|attempts| attempts.get(pattern))
    {
      if let Some(error) = error
        && report_invalid
      {
        self.diagnostics.push(Diagnostic::new(
          format!("invalid regex pattern: {error}"),
          span,
        ));
      } else if error.is_none() {
        self.regex_literals.push(RegexLiteral {
          pattern: pattern.to_owned(),
          flavor,
          span,
        });
      }
      return;
    }

    if !self.reserve_regex_source(pattern.len(), span) {
      return;
    }
    let literal = RegexLiteral {
      pattern: pattern.to_owned(),
      flavor,
      span,
    };
    let error = self
      .regex_cache
      .insert_with_size_limit(
        &literal,
        self.analyzer.regex_limits.max_compiled_regex_bytes,
      )
      .err()
      .map(|error| error.to_string());
    self
      .regex_attempts
      .entry(flavor)
      .or_default()
      .insert(literal.pattern.clone(), error.clone());
    self.regex_attempt_count = self.regex_attempt_count.saturating_add(1);
    if let Some(error) = error {
      if report_invalid {
        self.diagnostics.push(Diagnostic::new(
          format!("invalid regex pattern: {error}"),
          span,
        ));
      }
    } else {
      self.regex_literals.push(literal);
    }
  }

  fn reserve_regex_source(&mut self, bytes: usize, span: SourceSpan) -> bool {
    if self.regex_attempt_count >= self.analyzer.regex_limits.max_unique_regexes {
      if !self.regex_count_limit_reported {
        self
          .diagnostics
          .push(Diagnostic::new("unique regex literal limit exceeded", span));
        self.regex_count_limit_reported = true;
      }
      return false;
    }
    let Some(total) = self.regex_source_bytes.checked_add(bytes) else {
      self.report_regex_source_limit(span);
      return false;
    };
    if total > self.analyzer.regex_limits.max_total_regex_source_bytes {
      self.report_regex_source_limit(span);
      return false;
    }
    self.regex_source_bytes = total;
    true
  }

  fn report_regex_source_limit(&mut self, span: SourceSpan) {
    if !self.regex_source_limit_reported {
      self.diagnostics.push(Diagnostic::new(
        "total regex source byte limit exceeded",
        span,
      ));
      self.regex_source_limit_reported = true;
    }
  }
}
