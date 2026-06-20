use std::collections::BTreeMap;

use crate::parser::{AstExpression, ExprKind, SourceSpan};

use crate::sema::profile::{BodyNeedSummary, BodyTarget};
use crate::sema::schema::CapabilityKind;
use crate::sema::verified::VerifiedExpression;

pub(super) struct ExprAnalysis {
  pub expr: VerifiedExpression,
  pub origin: Option<ObjectOrigin>,
  pub path: Option<Vec<String>>,
  pub body_need: BodyNeedSummary,
  pub nodes: usize,
  pub cost: u64,
}

impl ExprAnalysis {
  pub fn new(
    expr: VerifiedExpression,
    origin: Option<ObjectOrigin>,
    path: Option<Vec<String>>,
    body_need: BodyNeedSummary,
    nodes: usize,
    cost: u64,
  ) -> Self {
    Self {
      expr,
      origin,
      path,
      body_need,
      nodes,
      cost,
    }
  }

  pub fn leaf(expr: VerifiedExpression, origin: Option<ObjectOrigin>) -> Self {
    Self::new(expr, origin, None, BodyNeedSummary::default(), 1, 1)
  }

  pub fn with_path(mut self, path: Vec<String>) -> Self {
    self.path = Some(path);
    self
  }
}

pub(super) struct ArgsAnalysis {
  pub exprs: Vec<VerifiedExpression>,
  pub body_need: BodyNeedSummary,
  pub nodes: usize,
  pub cost: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ObjectOrigin {
  Request,
  RequestHttp,
  RequestBody,
  RequestBodyBytes,
  Response,
  ResponseHttp,
  ResponseBody,
  ResponseBodyBytes,
  Stream,
  StreamPayload,
}

impl ObjectOrigin {
  pub fn root(name: &str) -> Option<Self> {
    match name {
      "Request" => Some(Self::Request),
      "Response" => Some(Self::Response),
      "Stream" => Some(Self::Stream),
      _ => None,
    }
  }

  pub fn body_target(self) -> Option<BodyTarget> {
    match self {
      Self::RequestBody | Self::RequestBodyBytes => Some(BodyTarget::Request),
      Self::ResponseBody | Self::ResponseBodyBytes => Some(BodyTarget::Response),
      Self::StreamPayload => Some(BodyTarget::Stream),
      _ => None,
    }
  }
}

pub(super) fn member_origin(receiver: ObjectOrigin, field: &str) -> Option<ObjectOrigin> {
  match (receiver, field) {
    (ObjectOrigin::Request, "Http") => Some(ObjectOrigin::RequestHttp),
    (ObjectOrigin::Response, "Http") => Some(ObjectOrigin::ResponseHttp),
    (ObjectOrigin::Request | ObjectOrigin::RequestHttp, "Body") => Some(ObjectOrigin::RequestBody),
    (ObjectOrigin::Response | ObjectOrigin::ResponseHttp, "Body") => {
      Some(ObjectOrigin::ResponseBody)
    }
    (ObjectOrigin::RequestBody, "Bytes") => Some(ObjectOrigin::RequestBodyBytes),
    (ObjectOrigin::ResponseBody, "Bytes") => Some(ObjectOrigin::ResponseBodyBytes),
    (ObjectOrigin::Stream, "Payload") => Some(ObjectOrigin::StreamPayload),
    _ => None,
  }
}

#[derive(Clone)]
pub(super) struct FunctionCallSite {
  pub name: String,
  pub arity: usize,
  pub span: SourceSpan,
}

pub(super) fn function_calls(expression: &AstExpression) -> Vec<FunctionCallSite> {
  let mut calls = Vec::new();
  collect_function_calls(expression, &mut calls);
  calls
}

fn collect_function_calls(expression: &AstExpression, calls: &mut Vec<FunctionCallSite>) {
  match &expression.kind {
    ExprKind::FunctionCall { name, args } => {
      calls.push(FunctionCallSite {
        name: name.clone(),
        arity: args.len(),
        span: expression.span,
      });
      for arg in args {
        collect_function_calls(arg, calls);
      }
    }
    ExprKind::Array { items } => {
      for item in items {
        collect_function_calls(item, calls);
      }
    }
    ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
      collect_function_calls(receiver, calls)
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      collect_function_calls(receiver, calls);
      for arg in args {
        collect_function_calls(arg, calls);
      }
    }
    ExprKind::Binary { left, right, .. } => {
      collect_function_calls(left, calls);
      collect_function_calls(right, calls);
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => {}
  }
}

pub(super) fn substitute_expression(
  expression: &AstExpression,
  replacements: &BTreeMap<String, AstExpression>,
) -> AstExpression {
  match &expression.kind {
    ExprKind::Identifier { name } => replacements
      .get(name)
      .cloned()
      .unwrap_or_else(|| expression.clone()),
    ExprKind::Array { items } => AstExpression::new(
      ExprKind::Array {
        items: items
          .iter()
          .map(|item| substitute_expression(item, replacements))
          .collect(),
      },
      expression.span,
    ),
    ExprKind::Member { receiver, name } => AstExpression::new(
      ExprKind::Member {
        receiver: Box::new(substitute_expression(receiver, replacements)),
        name: name.clone(),
      },
      expression.span,
    ),
    ExprKind::FunctionCall { name, args } => AstExpression::new(
      ExprKind::FunctionCall {
        name: name.clone(),
        args: args
          .iter()
          .map(|arg| substitute_expression(arg, replacements))
          .collect(),
      },
      expression.span,
    ),
    ExprKind::MethodCall {
      receiver,
      name,
      args,
    } => AstExpression::new(
      ExprKind::MethodCall {
        receiver: Box::new(substitute_expression(receiver, replacements)),
        name: name.clone(),
        args: args
          .iter()
          .map(|arg| substitute_expression(arg, replacements))
          .collect(),
      },
      expression.span,
    ),
    ExprKind::Unary { op, expr } => AstExpression::new(
      ExprKind::Unary {
        op: *op,
        expr: Box::new(substitute_expression(expr, replacements)),
      },
      expression.span,
    ),
    ExprKind::Binary { left, op, right } => AstExpression::new(
      ExprKind::Binary {
        left: Box::new(substitute_expression(left, replacements)),
        op: *op,
        right: Box::new(substitute_expression(right, replacements)),
      },
      expression.span,
    ),
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. } => expression.clone(),
  }
}

pub(super) fn string_literal(expression: &AstExpression) -> Option<String> {
  match &expression.kind {
    ExprKind::String { value } => Some(value.clone()),
    _ => None,
  }
}

pub(super) fn capability_kind_label(kind: CapabilityKind) -> &'static str {
  match kind {
    CapabilityKind::Function => "function",
    CapabilityKind::Method => "method",
    CapabilityKind::UnaryOp => "unary operator",
    CapabilityKind::BinaryOp => "binary operator",
  }
}
