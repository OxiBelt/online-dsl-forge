/// Resource limits applied while lexing and parsing untrusted source text.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ParseLimits {
  pub max_source_bytes: usize,
  pub max_decoded_scalar_bytes: usize,
  pub max_tokens: usize,
  pub max_diagnostics: usize,
  pub max_ast_nodes: usize,
  pub max_collection_items: usize,
}

impl Default for ParseLimits {
  fn default() -> Self {
    Self {
      max_source_bytes: 1024 * 1024,
      max_decoded_scalar_bytes: 1024 * 1024,
      max_tokens: 262_144,
      max_diagnostics: 1024,
      max_ast_nodes: 65_536,
      max_collection_items: 65_536,
    }
  }
}

/// Resource limits applied while formatting an arbitrary public AST.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AstFormatLimits {
  pub max_depth: usize,
  pub max_nodes: usize,
  pub max_output_bytes: usize,
}

impl Default for AstFormatLimits {
  fn default() -> Self {
    Self {
      max_depth: 127,
      max_nodes: 65_536,
      max_output_bytes: 4 * 1024 * 1024,
    }
  }
}
