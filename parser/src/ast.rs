use serde::{Deserialize, Serialize};

use crate::span::SourceSpan;

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct AstExpression {
    pub kind: ExprKind,
    pub span: SourceSpan,
}

impl AstExpression {
    pub fn new(kind: ExprKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExprKind {
    Null,
    Bool {
        value: bool,
    },
    Int {
        value: i64,
    },
    Float {
        value: f64,
    },
    String {
        value: String,
    },
    Array {
        items: Vec<AstExpression>,
    },
    Identifier {
        name: String,
    },
    Member {
        receiver: Box<AstExpression>,
        name: String,
    },
    FunctionCall {
        name: String,
        args: Vec<AstExpression>,
    },
    MethodCall {
        receiver: Box<AstExpression>,
        name: String,
        args: Vec<AstExpression>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<AstExpression>,
    },
    Binary {
        left: Box<AstExpression>,
        op: BinaryOp,
        right: Box<AstExpression>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Or => "||",
            Self::And => "&&",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
        }
    }
}

impl UnaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Not => "!",
            Self::Neg => "-",
        }
    }
}
