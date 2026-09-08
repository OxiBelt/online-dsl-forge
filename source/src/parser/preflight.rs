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

pub(crate) fn drain_ast_iteratively(expression: AstExpression) {
  let mut pending = vec![AstDrainWork::Expression(expression)];
  while let Some(work) = pending.pop() {
    match work {
      AstDrainWork::Expression(AstExpression { kind, .. }) => match kind {
        ExprKind::Array { items } | ExprKind::FunctionCall { args: items, .. } => {
          pending.push(AstDrainWork::Expressions(items.into_iter()));
        }
        ExprKind::Member { receiver, .. } | ExprKind::Unary { expr: receiver, .. } => {
          pending.push(AstDrainWork::Expression(*receiver));
        }
        ExprKind::MethodCall { receiver, args, .. } => {
          pending.push(AstDrainWork::Expressions(args.into_iter()));
          pending.push(AstDrainWork::Expression(*receiver));
        }
        ExprKind::Binary { left, right, .. } => {
          pending.push(AstDrainWork::Expression(*left));
          pending.push(AstDrainWork::Expression(*right));
        }
        ExprKind::Null
        | ExprKind::Bool { .. }
        | ExprKind::Int { .. }
        | ExprKind::Float { .. }
        | ExprKind::String { .. }
        | ExprKind::Identifier { .. } => {}
      },
      AstDrainWork::Expressions(mut expressions) => {
        if let Some(expression) = expressions.next() {
          pending.push(AstDrainWork::Expressions(expressions));
          pending.push(AstDrainWork::Expression(expression));
        }
      }
    }
  }
}

enum AstDrainWork {
  Expression(AstExpression),
  Expressions(std::vec::IntoIter<AstExpression>),
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
