use crate::parser::{AstExpression, Diagnostic, ExprKind, SourceSpan};
use crate::sema::schema::ExpressionFunctionScope;

use super::{AnalyzeState, ExpressionFunctionMode};

const MAX_LOWERED_SCALAR_BYTES: usize = 64 * 1024 * 1024;

impl<'a> AnalyzeState<'a> {
  pub(super) fn preflight_lowering(&mut self, expression: &'a AstExpression) -> bool {
    let mut nodes = 0_usize;
    let mut scalar_bytes = 0_usize;
    let mut bindings = Vec::new();
    let mut active = Vec::new();
    self.count_lowered(
      expression,
      0,
      &mut bindings,
      &mut active,
      &mut nodes,
      &mut scalar_bytes,
    )
  }

  fn count_lowered(
    &mut self,
    expression: &'a AstExpression,
    depth: usize,
    bindings: &mut Vec<(&'a [String], &'a [AstExpression])>,
    active: &mut Vec<(ExpressionFunctionScope, &'a str)>,
    nodes: &mut usize,
    scalar_bytes: &mut usize,
  ) -> bool {
    if depth > self.analyzer.profile.max_call_depth.min(128) {
      self.report_depth_limit(expression.span);
      return false;
    }
    if self.analyzer.expression_function_mode == ExpressionFunctionMode::Inline
      && let ExprKind::Identifier { name } = &expression.kind
      && let Some((params, args)) = bindings.last().copied()
      && let Some(argument) = params
        .iter()
        .zip(args.iter())
        .find_map(|(param, argument)| (param == name).then_some(argument))
      && let Some(frame) = bindings.pop()
    {
      let accepted = self.count_lowered(argument, depth, bindings, active, nodes, scalar_bytes);
      bindings.push(frame);
      return accepted;
    }

    if !self.charge_preflight_scalar_bytes(
      expression_scalar_bytes(expression),
      scalar_bytes,
      expression.span,
    ) {
      return false;
    }

    let Some(next) = nodes.checked_add(1) else {
      self.report_node_limit(expression.span);
      return false;
    };
    if next > self.analyzer.profile.max_ast_nodes {
      self.report_node_limit(expression.span);
      return false;
    }
    *nodes = next;
    let Some(child_depth) = depth.checked_add(1) else {
      self.report_depth_limit(expression.span);
      return false;
    };

    match &expression.kind {
      ExprKind::Array { items } => {
        self.count_all(items, child_depth, bindings, active, nodes, scalar_bytes)
      }
      ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
        self.count_lowered(receiver, child_depth, bindings, active, nodes, scalar_bytes)
      }
      ExprKind::FunctionCall { name, args } => {
        let scope = active
          .last()
          .map(|(scope, _)| *scope)
          .unwrap_or(self.analyzer.expression_function_scope);
        let Some(function) = self.schema.expression_function_for_scope(name, scope) else {
          return self.count_all(args, child_depth, bindings, active, nodes, scalar_bytes);
        };
        if function.params.len() != args.len() {
          return true;
        }
        let key = (function.scope, function.name.as_str());
        if active.contains(&key) {
          return true;
        }
        if self.analyzer.expression_function_mode == ExpressionFunctionMode::CallFrame
          && (!function.params.iter().all(|param| {
            self.charge_preflight_scalar_bytes(param.len(), scalar_bytes, expression.span)
          }) || !self.count_all(args, child_depth, bindings, active, nodes, scalar_bytes))
        {
          return false;
        }
        active.push(key);
        if self.analyzer.expression_function_mode == ExpressionFunctionMode::Inline {
          bindings.push((&function.params, args));
        }
        let accepted = self.count_lowered(
          &function.expression,
          child_depth,
          bindings,
          active,
          nodes,
          scalar_bytes,
        );
        if self.analyzer.expression_function_mode == ExpressionFunctionMode::Inline {
          bindings.pop();
        }
        active.pop();
        accepted
      }
      ExprKind::MethodCall { receiver, args, .. } => {
        self.count_lowered(receiver, child_depth, bindings, active, nodes, scalar_bytes)
          && self.count_all(args, child_depth, bindings, active, nodes, scalar_bytes)
      }
      ExprKind::Binary { left, right, .. } => {
        self.count_lowered(left, child_depth, bindings, active, nodes, scalar_bytes)
          && self.count_lowered(right, child_depth, bindings, active, nodes, scalar_bytes)
      }
      ExprKind::Null
      | ExprKind::Bool { .. }
      | ExprKind::Int { .. }
      | ExprKind::Float { .. }
      | ExprKind::String { .. }
      | ExprKind::Identifier { .. } => true,
    }
  }

  fn count_all(
    &mut self,
    expressions: &'a [AstExpression],
    depth: usize,
    bindings: &mut Vec<(&'a [String], &'a [AstExpression])>,
    active: &mut Vec<(ExpressionFunctionScope, &'a str)>,
    nodes: &mut usize,
    scalar_bytes: &mut usize,
  ) -> bool {
    expressions.iter().all(|expression| {
      self.count_lowered(expression, depth, bindings, active, nodes, scalar_bytes)
    })
  }

  pub(super) fn charge_node(&mut self, span: SourceSpan) -> bool {
    let Some(nodes) = self.lowered_nodes.checked_add(1) else {
      self.report_node_limit(span);
      return false;
    };
    if nodes > self.analyzer.profile.max_ast_nodes {
      self.report_node_limit(span);
      return false;
    }
    self.lowered_nodes = nodes;
    true
  }

  pub(super) fn charge_expression_scalar_bytes(&mut self, expression: &AstExpression) -> bool {
    self.charge_lowered_scalar_bytes(expression_scalar_bytes(expression), expression.span)
  }

  pub(super) fn charge_lowered_scalar_bytes(&mut self, bytes: usize, span: SourceSpan) -> bool {
    let Some(total) = self.lowered_scalar_bytes.checked_add(bytes) else {
      self.report_scalar_byte_limit(span);
      return false;
    };
    if total > MAX_LOWERED_SCALAR_BYTES {
      self.report_scalar_byte_limit(span);
      return false;
    }
    self.lowered_scalar_bytes = total;
    true
  }

  fn charge_preflight_scalar_bytes(
    &mut self,
    bytes: usize,
    total: &mut usize,
    span: SourceSpan,
  ) -> bool {
    let Some(next) = total.checked_add(bytes) else {
      self.report_scalar_byte_limit(span);
      return false;
    };
    if next > MAX_LOWERED_SCALAR_BYTES {
      self.report_scalar_byte_limit(span);
      return false;
    }
    *total = next;
    true
  }

  pub(super) fn report_node_limit(&mut self, span: SourceSpan) {
    if !self.node_limit_reported {
      self
        .diagnostics
        .push(Diagnostic::new("AST node limit exceeded", span));
      self.node_limit_reported = true;
    }
  }

  pub(super) fn report_depth_limit(&mut self, span: SourceSpan) {
    if !self.depth_limit_reported {
      self
        .diagnostics
        .push(Diagnostic::new("semantic call depth limit exceeded", span));
      self.depth_limit_reported = true;
    }
  }

  fn report_scalar_byte_limit(&mut self, span: SourceSpan) {
    if !self.scalar_byte_limit_reported {
      self
        .diagnostics
        .push(Diagnostic::new("lowered scalar byte limit exceeded", span));
      self.scalar_byte_limit_reported = true;
    }
  }
}

fn expression_scalar_bytes(expression: &AstExpression) -> usize {
  match &expression.kind {
    ExprKind::String { value } => value.len(),
    ExprKind::Identifier { name }
    | ExprKind::Member { name, .. }
    | ExprKind::FunctionCall { name, .. }
    | ExprKind::MethodCall { name, .. } => name.len(),
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::Array { .. }
    | ExprKind::Unary { .. }
    | ExprKind::Binary { .. } => 0,
  }
}
