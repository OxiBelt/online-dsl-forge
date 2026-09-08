use super::{AstExpression, DiagnosticReport, ExprKind};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct AstMetrics {
  pub nodes: usize,
  pub depth: usize,
}

pub(crate) fn preflight_ast(
  expression: &AstExpression,
  max_nodes: usize,
  max_depth: usize,
) -> Result<AstMetrics, DiagnosticReport> {
  let mut stack = vec![(expression, 1_usize)];
  let mut nodes = 0_usize;
  let mut deepest = 0_usize;

  while let Some((current, depth)) = stack.pop() {
    if depth > max_depth {
      return Err(DiagnosticReport::single(
        "semantic call depth limit exceeded",
        current.span,
      ));
    }
    nodes = nodes
      .checked_add(1)
      .ok_or_else(|| DiagnosticReport::single("AST node limit exceeded", current.span))?;
    if nodes > max_nodes {
      return Err(DiagnosticReport::single(
        "AST node limit exceeded",
        current.span,
      ));
    }
    deepest = deepest.max(depth);
    let child_count = child_count(current);
    if child_count > max_nodes.saturating_sub(nodes) {
      return Err(DiagnosticReport::single(
        "AST node limit exceeded",
        current.span,
      ));
    }
    if child_count > 0 && depth >= max_depth {
      return Err(DiagnosticReport::single(
        "semantic call depth limit exceeded",
        current.span,
      ));
    }
    let child_depth = depth.checked_add(1).ok_or_else(|| {
      DiagnosticReport::single("semantic call depth limit exceeded", current.span)
    })?;
    push_children(&mut stack, current, child_depth);
  }

  Ok(AstMetrics {
    nodes,
    depth: deepest,
  })
}

fn child_count(expression: &AstExpression) -> usize {
  match &expression.kind {
    ExprKind::Array { items } | ExprKind::FunctionCall { args: items, .. } => items.len(),
    ExprKind::Member { .. } | ExprKind::Unary { .. } => 1,
    ExprKind::MethodCall { args, .. } => args.len().saturating_add(1),
    ExprKind::Binary { .. } => 2,
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => 0,
  }
}

fn push_children<'a>(
  stack: &mut Vec<(&'a AstExpression, usize)>,
  expression: &'a AstExpression,
  depth: usize,
) {
  match &expression.kind {
    ExprKind::Array { items } | ExprKind::FunctionCall { args: items, .. } => {
      for item in items.iter().rev() {
        stack.push((item, depth));
      }
    }
    ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
      stack.push((receiver, depth));
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      for arg in args.iter().rev() {
        stack.push((arg, depth));
      }
      stack.push((receiver, depth));
    }
    ExprKind::Binary { left, right, .. } => {
      stack.push((right, depth));
      stack.push((left, depth));
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => {}
  }
}
