//! In-memory DSL parser, canonical AST, compiler, and bounded runtime.

pub mod ast;
pub mod compile;
pub mod diagnostics;
pub mod format;
pub mod lexer;
pub mod parser;
pub mod runtime;
pub mod span;
pub mod value;

pub use ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
pub use compile::{CompileOptions, CompiledExpression, RuntimeSchema, compile_expression};
pub use diagnostics::{Diagnostic, DiagnosticReport};
pub use format::format_expression;
pub use parser::parse_expression;
pub use runtime::{
  DynamicRegistry, EvalError, EvalLimits, MapRuntime, RuntimeContext, default_registry, evaluate,
};
pub use span::SourceSpan;
pub use value::Value;
