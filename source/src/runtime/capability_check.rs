use crate::sema::{CapabilityKind, VerifiedProgram};

use super::{DynamicRegistry, EvalError};

pub(super) fn verify_runtime_capabilities(
  program: &VerifiedProgram,
  registry: &DynamicRegistry,
) -> Result<(), EvalError> {
  for ticket in program.required_capabilities() {
    if !registry.has_capability_ticket(ticket) {
      return Err(EvalError::new(
        format!(
          "runtime registry is missing verified {} {} with {} arguments",
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

fn capability_label(kind: CapabilityKind) -> &'static str {
  match kind {
    CapabilityKind::Function => "function",
    CapabilityKind::Method => "method",
    CapabilityKind::UnaryOp => "unary operator",
    CapabilityKind::BinaryOp => "binary operator",
  }
}
