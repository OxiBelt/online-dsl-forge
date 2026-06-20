//! In-memory DSL parser, canonical AST, compiler, and bounded runtime.

pub mod compile;
pub mod runtime;
pub mod value;

pub use compile::{
  Analyzer, BodyAccess, BodyNeedSummary, BodyPathRule, BodyTarget, CapabilityKind, CapabilityMeta,
  CapabilityTicket, CompileOptions, CompiledExpression, CostModel, Determinism, ExpressionFunction,
  Phase, RegexArgMeta, RegexFlavor, RegexPolicy, RuntimeSchema, SecurityProfile, SecurityProfileId,
  SignatureMatch, TypeClass, VariableMeta, VerifiedExprKindRef, VerifiedExpression,
  VerifiedProgram, compile_expression,
};
pub use online_dsl_forge_parser::{
  AstExpression, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, SourceSpan, UnaryOp, ast,
  diagnostics, format, format_expression, lexer, parse_expression, parser, span,
};
pub use online_dsl_forge_sema as sema;
pub use runtime::{
  DynamicRegistry, EvalError, EvalLimits, MapRuntime, RuntimeContext, default_registry, evaluate,
  evaluate_verified,
};
pub use value::Value;
