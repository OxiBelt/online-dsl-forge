//! In-memory DSL parser, canonical AST, compiler, and bounded runtime.

pub mod compile;
pub mod runtime;
pub mod value;

pub use compile::{CompileOptions, CompiledExpression, RuntimeSchema, compile_expression};
pub use online_dsl_forge_parser::{
  AstExpression, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, SourceSpan, UnaryOp, ast,
  diagnostics, format, format_expression, lexer, parse_expression, parser, span,
};
pub use runtime::{
  DynamicRegistry, EvalError, EvalLimits, MapRuntime, RuntimeContext, default_registry, evaluate,
};
pub use value::Value;
