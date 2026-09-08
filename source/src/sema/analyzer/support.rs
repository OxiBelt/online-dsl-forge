use crate::parser::{AstExpression, ExprKind, SourceSpan};

use crate::sema::profile::{BodyNeedSummary, BodyTarget};
use crate::sema::schema::CapabilityKind;
use crate::sema::verified::VerifiedExpression;

pub(super) struct ExprAnalysis {
  pub expr: VerifiedExpression,
  pub origin: Option<ObjectOrigin>,
  pub path: Option<Vec<String>>,
  pub body_need: BodyNeedSummary,
  pub mitigation_payload: bool,
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
      mitigation_payload: false,
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

  pub fn with_path_option(mut self, path: Option<Vec<String>>) -> Self {
    self.path = path;
    self
  }

  pub fn with_mitigation_payload(mut self, value: bool) -> Self {
    self.mitigation_payload = value;
    self
  }
}

#[derive(Debug, Clone)]
pub(super) struct LocalBinding {
  pub origin: Option<ObjectOrigin>,
  pub path: Option<Vec<String>>,
  pub mitigation_payload: bool,
}

impl LocalBinding {
  pub fn from_analysis(analysis: &ExprAnalysis) -> Self {
    Self {
      origin: analysis.origin,
      path: analysis.path.clone(),
      mitigation_payload: analysis.mitigation_payload,
    }
  }
}

pub(super) struct ArgsAnalysis {
  pub exprs: Vec<VerifiedExpression>,
  pub bindings: Vec<LocalBinding>,
  pub body_need: BodyNeedSummary,
  pub consumed_body_need: BodyNeedSummary,
  pub mitigation_payload: bool,
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

  pub fn is_mitigation_payload_boundary(self) -> bool {
    matches!(
      self,
      Self::RequestBody | Self::ResponseBody | Self::StreamPayload
    )
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
  let mut stack = vec![expression];
  while let Some(expression) = stack.pop() {
    collect_function_calls(expression, &mut calls, &mut stack);
  }
  calls
}

fn collect_function_calls<'a>(
  expression: &'a AstExpression,
  calls: &mut Vec<FunctionCallSite>,
  stack: &mut Vec<&'a AstExpression>,
) {
  match &expression.kind {
    ExprKind::FunctionCall { name, args } => {
      calls.push(FunctionCallSite {
        name: name.clone(),
        arity: args.len(),
        span: expression.span,
      });
      for arg in args.iter().rev() {
        stack.push(arg);
      }
    }
    ExprKind::Array { items } => {
      for item in items.iter().rev() {
        stack.push(item);
      }
    }
    ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
      stack.push(receiver)
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      for arg in args.iter().rev() {
        stack.push(arg);
      }
      stack.push(receiver);
    }
    ExprKind::Binary { left, right, .. } => {
      stack.push(right);
      stack.push(left);
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => {}
  }
}

pub(super) fn string_literal(expression: &AstExpression) -> Option<&str> {
  match &expression.kind {
    ExprKind::String { value } => Some(value),
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
