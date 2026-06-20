//! Syntax-only parser, canonical AST, diagnostics, spans, and formatter.

pub mod ast;
pub mod diagnostics;
pub mod format;
pub mod lexer;
pub mod parser;
pub mod span;

pub use ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
pub use diagnostics::{Diagnostic, DiagnosticReport};
pub use format::format_expression;
pub use parser::parse_expression;
pub use span::SourceSpan;
