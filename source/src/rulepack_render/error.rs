use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RulepackRenderError {
  message: String,
}

impl RulepackRenderError {
  pub fn new(message: impl Into<String>) -> Self {
    Self {
      message: message.into(),
    }
  }
}

impl fmt::Display for RulepackRenderError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(&self.message)
  }
}

impl Error for RulepackRenderError {}

pub(crate) type RenderResult<T> = Result<T, RulepackRenderError>;

pub(crate) fn fail<T>(message: impl Into<String>) -> RenderResult<T> {
  Err(RulepackRenderError::new(message))
}
