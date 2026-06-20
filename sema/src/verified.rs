use std::collections::{BTreeMap, BTreeSet};

use online_dsl_forge_parser::{AstExpression, BinaryOp, SourceSpan, UnaryOp};
use regex::{Regex, RegexBuilder};

use crate::profile::{BodyNeedSummary, SecurityProfile};
use crate::schema::{CapabilityTicket, RegexFlavor};

#[derive(Debug, Clone)]
pub struct VerifiedProgram {
    ast: AstExpression,
    root: VerifiedExpression,
    profile: SecurityProfile,
    body_need: BodyNeedSummary,
    static_cost_upper_bound: u64,
    regex_literals: Vec<RegexLiteral>,
    regex_cache: CompiledRegexCache,
    required_capabilities: BTreeSet<CapabilityTicket>,
}

impl VerifiedProgram {
    pub(crate) fn new(parts: VerifiedProgramParts) -> Self {
        Self {
            ast: parts.ast,
            root: parts.root,
            profile: parts.profile,
            body_need: parts.body_need,
            static_cost_upper_bound: parts.static_cost_upper_bound,
            regex_literals: parts.regex_literals,
            regex_cache: parts.regex_cache,
            required_capabilities: parts.required_capabilities,
        }
    }

    pub fn ast(&self) -> &AstExpression {
        &self.ast
    }

    pub fn root(&self) -> &VerifiedExpression {
        &self.root
    }

    pub fn profile(&self) -> &SecurityProfile {
        &self.profile
    }

    pub fn body_need(&self) -> BodyNeedSummary {
        self.body_need
    }

    pub fn static_cost_upper_bound(&self) -> u64 {
        self.static_cost_upper_bound
    }

    pub fn regex_literals(&self) -> &[RegexLiteral] {
        &self.regex_literals
    }

    pub fn regex_cache(&self) -> &CompiledRegexCache {
        &self.regex_cache
    }

    pub fn required_capabilities(&self) -> &BTreeSet<CapabilityTicket> {
        &self.required_capabilities
    }
}

pub(crate) struct VerifiedProgramParts {
    pub ast: AstExpression,
    pub root: VerifiedExpression,
    pub profile: SecurityProfile,
    pub body_need: BodyNeedSummary,
    pub static_cost_upper_bound: u64,
    pub regex_literals: Vec<RegexLiteral>,
    pub regex_cache: CompiledRegexCache,
    pub required_capabilities: BTreeSet<CapabilityTicket>,
}

#[derive(Debug, Clone)]
pub struct CompiledExpression {
    verified: VerifiedProgram,
}

impl CompiledExpression {
    pub(crate) fn new(verified: VerifiedProgram) -> Self {
        Self { verified }
    }

    pub fn ast(&self) -> &AstExpression {
        self.verified.ast()
    }

    pub fn into_ast(self) -> AstExpression {
        self.verified.ast
    }

    pub fn verified_program(&self) -> &VerifiedProgram {
        &self.verified
    }

    pub fn into_verified_program(self) -> VerifiedProgram {
        self.verified
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedExpression {
    kind: VerifiedExprKind,
    span: SourceSpan,
}

impl VerifiedExpression {
    pub(crate) fn new(kind: VerifiedExprKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }

    pub fn span(&self) -> SourceSpan {
        self.span
    }

    pub fn kind(&self) -> VerifiedExprKindRef<'_> {
        match &self.kind {
            VerifiedExprKind::Null => VerifiedExprKindRef::Null,
            VerifiedExprKind::Bool(value) => VerifiedExprKindRef::Bool(*value),
            VerifiedExprKind::Int(value) => VerifiedExprKindRef::Int(*value),
            VerifiedExprKind::Float(value) => VerifiedExprKindRef::Float(*value),
            VerifiedExprKind::String(value) => VerifiedExprKindRef::String(value),
            VerifiedExprKind::Array(items) => VerifiedExprKindRef::Array(items),
            VerifiedExprKind::Identifier(name) => VerifiedExprKindRef::Identifier(name),
            VerifiedExprKind::Member { receiver, name } => {
                VerifiedExprKindRef::Member { receiver, name }
            }
            VerifiedExprKind::FunctionCall { name, args } => {
                VerifiedExprKindRef::FunctionCall { name, args }
            }
            VerifiedExprKind::MethodCall {
                receiver,
                name,
                args,
            } => VerifiedExprKindRef::MethodCall {
                receiver,
                name,
                args,
            },
            VerifiedExprKind::Unary { op, expr } => VerifiedExprKindRef::Unary { op: *op, expr },
            VerifiedExprKind::Binary { left, op, right } => VerifiedExprKindRef::Binary {
                left,
                op: *op,
                right,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum VerifiedExprKind {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<VerifiedExpression>),
    Identifier(String),
    Member {
        receiver: Box<VerifiedExpression>,
        name: String,
    },
    FunctionCall {
        name: String,
        args: Vec<VerifiedExpression>,
    },
    MethodCall {
        receiver: Box<VerifiedExpression>,
        name: String,
        args: Vec<VerifiedExpression>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<VerifiedExpression>,
    },
    Binary {
        left: Box<VerifiedExpression>,
        op: BinaryOp,
        right: Box<VerifiedExpression>,
    },
}

pub enum VerifiedExprKindRef<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(&'a str),
    Array(&'a [VerifiedExpression]),
    Identifier(&'a str),
    Member {
        receiver: &'a VerifiedExpression,
        name: &'a str,
    },
    FunctionCall {
        name: &'a str,
        args: &'a [VerifiedExpression],
    },
    MethodCall {
        receiver: &'a VerifiedExpression,
        name: &'a str,
        args: &'a [VerifiedExpression],
    },
    Unary {
        op: UnaryOp,
        expr: &'a VerifiedExpression,
    },
    Binary {
        left: &'a VerifiedExpression,
        op: BinaryOp,
        right: &'a VerifiedExpression,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RegexLiteral {
    pub pattern: String,
    pub flavor: RegexFlavor,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledRegexCache {
    default: BTreeMap<String, Regex>,
    header_name: BTreeMap<String, Regex>,
}

impl CompiledRegexCache {
    pub fn insert(&mut self, literal: &RegexLiteral) -> Result<(), regex::Error> {
        let target = match literal.flavor {
            RegexFlavor::Default => &mut self.default,
            RegexFlavor::HeaderName => &mut self.header_name,
        };
        if target.contains_key(&literal.pattern) {
            return Ok(());
        }
        target.insert(literal.pattern.clone(), compile_regex(literal)?);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.default.len() + self.header_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn compile_regex(literal: &RegexLiteral) -> Result<Regex, regex::Error> {
    match literal.flavor {
        RegexFlavor::Default => Regex::new(&literal.pattern),
        RegexFlavor::HeaderName => RegexBuilder::new(&literal.pattern)
            .case_insensitive(true)
            .build(),
    }
}
