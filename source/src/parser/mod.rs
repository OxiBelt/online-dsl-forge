//! Syntax-only parser, canonical AST, diagnostics, spans, and formatter.

pub mod ast;
pub mod diagnostics;
pub mod format;
pub mod lexer;
mod limits;
mod parse;
pub(crate) mod preflight;
pub mod span;
pub(crate) mod validation;

pub use self::ast::{AstExpression, BinaryOp, ExprKind, UnaryOp};
pub use self::diagnostics::{Diagnostic, DiagnosticReport};
pub use self::format::{format_expression, format_expression_with_limits};
pub use self::limits::{AstFormatLimits, ParseLimits};
pub use self::parse::{parse_expression, parse_expression_with_limits};
pub use self::span::SourceSpan;
