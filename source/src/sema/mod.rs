//! Semantic analysis, security profiles, and verified IR.

mod analyzer;
mod profile;
mod schema;
mod verified;

pub use analyzer::{Analyzer, CompileOptions, compile_expression};
pub use profile::{
  BodyAccess, BodyNeedSummary, BodyTarget, Determinism, Phase, RegexPolicy, SecurityProfile,
  SecurityProfileId,
};
pub use schema::{
  BodyPathRule, CapabilityKind, CapabilityMeta, CapabilityTicket, CostModel, ExpressionFunction,
  ExpressionFunctionDiagnostic, ExpressionFunctionScope, RegexArgMeta, RegexFlavor, RuntimeSchema,
  SignatureMatch, TypeClass, VariableMeta,
};
pub use verified::{
  CompiledExpression, CompiledRegexCache, RegexLiteral, VerifiedExprKindRef, VerifiedExpression,
  VerifiedProgram,
};
