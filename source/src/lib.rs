//! In-memory DSL parser, canonical AST, compiler, and bounded runtime.

pub mod compile;
pub mod parser;
pub mod rulepack_render;
pub mod runtime;
pub mod sema;
mod serde_support;
pub mod value;

pub use compile::{
  Analyzer, BodyAccess, BodyNeedSummary, BodyPathRule, BodyTarget, CapabilityKind, CapabilityMeta,
  CapabilityTicket, CompileOptions, CompiledExpression, CompiledRegexCache, CostModel, Determinism,
  ExpressionDialect, ExpressionFunction, ExpressionFunctionDiagnostic, ExpressionFunctionLimits,
  ExpressionFunctionMode, ExpressionFunctionScope, Phase, RegexAdmissionLimits, RegexArgMeta,
  RegexFlavor, RegexLiteral, RegexPolicy, RuntimeSchema, SecurityProfile, SecurityProfileId,
  SignatureMatch, TypeClass, VariableMeta, VerifiedExprKindRef, VerifiedExpression,
  VerifiedProgram, compile_expression,
};
pub use parser::{
  AstExpression, AstFormatLimits, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, ParseLimits,
  SourceSpan, UnaryOp, ast, diagnostics, format, format_expression, format_expression_with_limits,
  lexer, parse_expression, parse_expression_with_limits, span,
};
pub use rulepack_render::{
  BlobFileResolver, BlobStore, FileResolver, MemoryFileResolver, RenderedRulepackBundle,
  RenderedRulepackFile, RulepackActionSelector, RulepackBinding, RulepackBindingKind,
  RulepackDiscovery, RulepackException, RulepackGroupFileSummary, RulepackInputMetadata,
  RulepackInspection, RulepackMode, RulepackModeOverride, RulepackOverride,
  RulepackOverrideSelector, RulepackPhase, RulepackProfile, RulepackReferencedFile,
  RulepackReferencedFileKind, RulepackRenderError, RulepackRenderLimits, RulepackRenderOptions,
  RulepackRuleSummary, RulepackSourceProvenance, RulepackSummary, RulepackVariable,
  inspect_rulepack, inspect_rulepack_inputs, inspect_rulepack_inputs_with_limits,
  inspect_rulepack_with_limits, referenced_rulepack_files, referenced_rulepack_files_with_limits,
  render_rulepack_bundle, render_rulepack_bundle_with_limits, render_rulepack_for_install,
  render_rulepack_for_install_with_limits, render_text, render_text_with_limits,
};
pub use runtime::{
  DynamicRegistry, EvalError, EvalLimits, MapRuntime, RuntimeCallContext, RuntimeContext,
  RuntimePatternSetConfig, RuntimePatternSetError, RuntimePatternSetKind, RuntimePatternSetLimits,
  RuntimePatternSets, RuntimeResourceLimits, default_registry, evaluate, evaluate_verified,
  evaluate_verified_with_resource_limits, evaluate_with_resource_limits,
  oxirule_pattern_set_registry, register_oxirule_pattern_set_methods,
};
pub use value::Value;
