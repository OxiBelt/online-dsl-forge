//! Syntax-only parser, canonical AST, diagnostics, spans, and formatter.

pub mod ast;
pub mod diagnostics;
pub mod format;
pub mod lexer;
mod parse;
pub mod span;

pub use self::ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
pub use self::diagnostics::{Diagnostic, DiagnosticReport};
pub use self::format::format_expression;
pub use self::parse::parse_expression;
pub use self::span::SourceSpan;
