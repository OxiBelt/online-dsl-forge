use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::span::SourceSpan;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
  pub message: String,
  pub span: SourceSpan,
}

impl Diagnostic {
  pub fn new(message: impl Into<String>, span: SourceSpan) -> Self {
    Self {
      message: message.into(),
      span,
    }
  }
}

#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticReport {
  pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
  pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
    Self { diagnostics }
  }

  pub fn single(message: impl Into<String>, span: SourceSpan) -> Self {
    Self {
      diagnostics: vec![Diagnostic::new(message, span)],
    }
  }

  pub fn push(&mut self, diagnostic: Diagnostic) {
    self.diagnostics.push(diagnostic);
  }

  pub fn is_empty(&self) -> bool {
    self.diagnostics.is_empty()
  }
}

impl fmt::Display for DiagnosticReport {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for (index, diagnostic) in self.diagnostics.iter().enumerate() {
      if index > 0 {
        writeln!(formatter)?;
      }
      write!(
        formatter,
        "{} at {}..{}",
        diagnostic.message, diagnostic.span.start, diagnostic.span.end
      )?;
    }
    Ok(())
  }
}

impl Error for DiagnosticReport {}
