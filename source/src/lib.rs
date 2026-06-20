//! In-memory DSL parser, canonical AST, compiler, and bounded runtime.

pub mod compile;
pub mod parser;
pub mod rulepack_render;
pub mod runtime;
pub mod sema;
pub mod value;

pub use compile::{
  Analyzer, BodyAccess, BodyNeedSummary, BodyPathRule, BodyTarget, CapabilityKind, CapabilityMeta,
  CapabilityTicket, CompileOptions, CompiledExpression, CompiledRegexCache, CostModel, Determinism,
  ExpressionFunction, ExpressionFunctionDiagnostic, ExpressionFunctionScope, Phase, RegexArgMeta,
  RegexFlavor, RegexLiteral, RegexPolicy, RuntimeSchema, SecurityProfile, SecurityProfileId,
  SignatureMatch, TypeClass, VariableMeta, VerifiedExprKindRef, VerifiedExpression,
  VerifiedProgram, compile_expression,
};
pub use parser::{
  AstExpression, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, SourceSpan, UnaryOp, ast,
  diagnostics, format, format_expression, lexer, parse_expression, span,
};
pub use rulepack_render::{
  BlobFileResolver, BlobStore, FileResolver, MemoryFileResolver, RenderedRulepackBundle,
  RenderedRulepackFile, RulepackActionSelector, RulepackBinding, RulepackBindingKind,
  RulepackDiscovery, RulepackException, RulepackGroupFileSummary, RulepackInputMetadata,
  RulepackInspection, RulepackMode, RulepackModeOverride, RulepackOverride,
  RulepackOverrideSelector, RulepackPhase, RulepackProfile, RulepackReferencedFile,
  RulepackReferencedFileKind, RulepackRenderError, RulepackRenderOptions, RulepackRuleSummary,
  RulepackSourceProvenance, RulepackSummary, RulepackVariable, inspect_rulepack,
  inspect_rulepack_inputs, referenced_rulepack_files, render_rulepack_bundle,
  render_rulepack_for_install, render_text,
};
pub use runtime::{
  DynamicRegistry, EvalError, EvalLimits, MapRuntime, RuntimeCallContext, RuntimeContext,
  default_registry, evaluate, evaluate_verified,
};
pub use value::Value;
