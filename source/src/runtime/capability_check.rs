use crate::sema::{CapabilityKind, CapabilityMeta, VerifiedProgram};

use super::{DynamicRegistry, EvalError};

pub(super) fn verify_runtime_capabilities(
  program: &VerifiedProgram,
  registry: &DynamicRegistry,
) -> Result<(), EvalError> {
  for (ticket, expected) in program.required_capability_metadata() {
    let Some(actual) = registry.capability_for_ticket(ticket) else {
      return Err(EvalError::new(
        format!(
          "runtime registry is missing verified {} {} with {} arguments",
          capability_label(ticket.kind),
          ticket.name,
          ticket.arity
        ),
        program.root().span(),
      ));
    };
    if !capability_metadata_matches(expected, &actual) {
      return Err(EvalError::new(
        format!(
          "runtime registry metadata for verified {} {} with {} arguments does not match analyzed metadata",
          capability_label(ticket.kind),
          ticket.name,
          ticket.arity
        ),
        program.root().span(),
      ));
    }
  }
  Ok(())
}

fn capability_metadata_matches(expected: &CapabilityMeta, actual: &CapabilityMeta) -> bool {
  expected == actual
}

fn capability_label(kind: CapabilityKind) -> &'static str {
  match kind {
    CapabilityKind::Function => "function",
    CapabilityKind::Method => "method",
    CapabilityKind::UnaryOp => "unary operator",
    CapabilityKind::BinaryOp => "binary operator",
  }
}
