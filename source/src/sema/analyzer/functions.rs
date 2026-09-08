use std::collections::{BTreeMap, BTreeSet};

use crate::parser::{AstExpression, Diagnostic, ExprKind, SourceSpan};
use crate::sema::schema::{
  DIAGNOSTIC_BUDGET_EXCEEDED, ExpressionFunction, ExpressionFunctionScope, SignatureMatch,
  bounded_diagnostic_message, diagnostic_name, expression_function_diagnostic_byte_limit,
  expression_function_diagnostic_limit,
};
use crate::sema::verified::{VerifiedExprKind, VerifiedExpression};

use super::support::{ExprAnalysis, LocalBinding, function_calls};
use super::{AnalyzeState, ExpressionFunctionMode};

type FunctionKey = (ExpressionFunctionScope, String);

impl<'a> AnalyzeState<'a> {
  pub(super) fn inline_binding(&self, name: &str) -> Option<&'a AstExpression> {
    self.inline_bindings.last().and_then(|(params, args)| {
      params
        .iter()
        .zip(args.iter())
        .find_map(|(param, arg)| (param == name).then_some(arg))
    })
  }

  pub(super) fn resolve_inline_argument(
    &self,
    mut expression: &'a AstExpression,
  ) -> &'a AstExpression {
    if self.analyzer.expression_function_mode != ExpressionFunctionMode::Inline {
      return expression;
    }
    for (params, args) in self.inline_bindings.iter().rev() {
      let ExprKind::Identifier { name } = &expression.kind else {
        break;
      };
      let Some(argument) = params
        .iter()
        .zip(args.iter())
        .find_map(|(param, argument)| (param == name).then_some(argument))
      else {
        break;
      };
      expression = argument;
    }
    expression
  }

  pub(super) fn current_function_scope(&self) -> ExpressionFunctionScope {
    self
      .active_functions
      .last()
      .map(|(scope, _)| *scope)
      .unwrap_or(self.analyzer.expression_function_scope)
  }

  pub(super) fn validate_function_graph(&mut self) {
    for function in self.schema.expression_functions() {
      if self.function_diagnostic_limit_reached() {
        break;
      }
      let mut diagnostics = Vec::new();
      self
        .analyzer
        .dialect
        .validate(&function.expression, &mut diagnostics);
      for diagnostic in diagnostics {
        self.push_function_diagnostic(diagnostic);
      }
    }

    self.validate_function_cycles();
  }

  pub(super) fn analyze_expression_function(
    &mut self,
    function: &'a ExpressionFunction,
    args: &'a [AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    if self.analyzer.expression_function_mode == ExpressionFunctionMode::CallFrame {
      return self.analyze_expression_function_call_frame(function, args, span, depth);
    }

    if function.params.len() != args.len() {
      self.push_function_diagnostic(Diagnostic::new(
        format!(
          "function {} does not accept {} arguments",
          diagnostic_name(&function.name),
          args.len()
        ),
        span,
      ));
      return ExprAnalysis::leaf(
        VerifiedExpression::new(
          VerifiedExprKind::FunctionCall {
            name: function.name.clone(),
            args: Vec::new(),
          },
          span,
        ),
        None,
      );
    }

    let key = function_key(function);
    if self.active_functions.contains(&key) {
      self.push_function_diagnostic(Diagnostic::new(
        format!(
          "recursive expression function {}",
          diagnostic_name(&function.name)
        ),
        span,
      ));
      return ExprAnalysis::leaf(VerifiedExpression::new(VerifiedExprKind::Null, span), None);
    }

    self.active_functions.push(key);
    self.inline_bindings.push((&function.params, args));
    let mut analysis = self.analyze_expression(&function.expression, depth + 1);
    self.inline_bindings.pop();
    self.active_functions.pop();
    analysis.cost = analysis.cost.saturating_add(1);
    analysis.nodes = analysis.nodes.saturating_add(1);
    analysis
  }

  fn analyze_expression_function_call_frame(
    &mut self,
    function: &'a ExpressionFunction,
    args: &'a [AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    if function.params.len() != args.len() {
      self.push_function_diagnostic(Diagnostic::new(
        format!(
          "function {} does not accept {} arguments",
          diagnostic_name(&function.name),
          args.len()
        ),
        span,
      ));
      return ExprAnalysis::leaf(
        VerifiedExpression::new(
          VerifiedExprKind::ExpressionFunctionCall {
            name: function.name.clone(),
            params: function.params.clone(),
            args: Vec::new(),
            body: Box::new(VerifiedExpression::new(VerifiedExprKind::Null, span)),
          },
          span,
        ),
        None,
      );
    }

    if !function
      .params
      .iter()
      .all(|param| self.charge_lowered_scalar_bytes(param.len(), span))
    {
      return ExprAnalysis::leaf(VerifiedExpression::new(VerifiedExprKind::Null, span), None);
    }

    let key = function_key(function);
    if self.active_functions.contains(&key) {
      self.push_function_diagnostic(Diagnostic::new(
        format!(
          "recursive expression function {}",
          diagnostic_name(&function.name)
        ),
        span,
      ));
      return ExprAnalysis::leaf(VerifiedExpression::new(VerifiedExprKind::Null, span), None);
    }

    let args_analysis = self.analyze_args(args, depth);
    let bindings = function
      .params
      .iter()
      .cloned()
      .zip(args_analysis.bindings.iter().cloned())
      .collect::<BTreeMap<String, LocalBinding>>();

    self.active_functions.push(key);
    self.local_bindings.push(bindings);
    let body = self.analyze_expression(&function.expression, depth + 1);
    self.local_bindings.pop();
    self.active_functions.pop();
    let origin = body.origin;
    let path = body.path.clone();

    ExprAnalysis::new(
      VerifiedExpression::new(
        VerifiedExprKind::ExpressionFunctionCall {
          name: function.name.clone(),
          params: function.params.clone(),
          args: args_analysis.exprs,
          body: Box::new(body.expr),
        },
        span,
      ),
      origin,
      path,
      args_analysis.body_need.merge(body.body_need),
      args_analysis
        .nodes
        .checked_add(body.nodes)
        .and_then(|nodes| nodes.checked_add(1))
        .unwrap_or(usize::MAX),
      args_analysis
        .cost
        .checked_add(body.cost)
        .and_then(|cost| cost.checked_add(1))
        .unwrap_or(u64::MAX),
    )
    .with_mitigation_payload(args_analysis.mitigation_payload || body.mitigation_payload)
  }

  fn validate_function_cycles(&mut self) {
    let functions = self
      .schema
      .expression_functions()
      .map(|function| (function_key(function), function))
      .collect::<BTreeMap<_, _>>();
    let mut adjacency = BTreeMap::<FunctionKey, Vec<FunctionKey>>::new();
    for (key, function) in &functions {
      let mut edges = Vec::new();
      for call in function_calls(&function.expression) {
        let Some(callee) = self
          .schema
          .expression_function_for_scope(&call.name, function.scope)
        else {
          self.validate_host_function_call(&call.name, call.arity, call.span);
          continue;
        };
        if callee.params.len() != call.arity {
          self.push_function_diagnostic(Diagnostic::new(
            format!(
              "function {} does not accept {} arguments",
              diagnostic_name(&call.name),
              call.arity
            ),
            call.span,
          ));
        }
        edges.push(function_key(callee));
      }
      adjacency.insert(key.clone(), edges);
    }

    let mut permanent = BTreeSet::new();
    for start in functions.keys() {
      if permanent.contains(start) {
        continue;
      }
      let mut active = BTreeSet::new();
      let mut stack = vec![(start.clone(), false)];
      while let Some((key, exiting)) = stack.pop() {
        if exiting {
          active.remove(&key);
          permanent.insert(key);
          continue;
        }
        if permanent.contains(&key) {
          continue;
        }
        if !active.insert(key.clone()) {
          if let Some(function) = functions.get(&key) {
            self.push_function_diagnostic(Diagnostic::new(
              format!(
                "recursive expression function {}",
                diagnostic_name(&function.name)
              ),
              function.expression.span,
            ));
          }
          continue;
        }
        stack.push((key.clone(), true));
        if let Some(edges) = adjacency.get(&key) {
          for edge in edges.iter().rev() {
            if active.contains(edge) {
              if let Some(function) = functions.get(edge) {
                self.push_function_diagnostic(Diagnostic::new(
                  format!(
                    "recursive expression function {}",
                    diagnostic_name(&function.name)
                  ),
                  function.expression.span,
                ));
              }
            } else if !permanent.contains(edge) {
              stack.push((edge.clone(), false));
            }
          }
        }
      }
    }
  }

  fn validate_host_function_call(&mut self, name: &str, arity: usize, span: SourceSpan) {
    match self.schema.function_accepts(name, arity) {
      SignatureMatch::Matches => {}
      SignatureMatch::Unknown if self.analyzer.options.allow_unknown_functions => {}
      SignatureMatch::Unknown => {
        self.push_function_diagnostic(Diagnostic::new(
          format!("unknown function {}", diagnostic_name(name)),
          span,
        ));
      }
      SignatureMatch::ArityMismatch => self.push_function_diagnostic(Diagnostic::new(
        format!(
          "function {} does not accept {arity} arguments",
          diagnostic_name(name)
        ),
        span,
      )),
    }
  }

  fn push_function_diagnostic(&mut self, diagnostic: Diagnostic) {
    let limits = self.schema.expression_function_limits();
    let message = bounded_diagnostic_message(&diagnostic.message);
    let retained_bytes = self.diagnostics.iter().try_fold(0usize, |total, existing| {
      total.checked_add(existing.message.len())
    });
    let fits = retained_bytes
      .and_then(|total| total.checked_add(message.len()))
      .is_some_and(|total| total <= expression_function_diagnostic_byte_limit(limits));
    if self.diagnostics.len() < expression_function_diagnostic_limit(limits) && fits {
      self
        .diagnostics
        .push(Diagnostic::new(message, diagnostic.span));
    } else if self.diagnostics.is_empty() {
      self
        .diagnostics
        .push(Diagnostic::new(DIAGNOSTIC_BUDGET_EXCEEDED, diagnostic.span));
    }
  }

  fn function_diagnostic_limit_reached(&self) -> bool {
    self.diagnostics.len()
      >= expression_function_diagnostic_limit(self.schema.expression_function_limits()).max(1)
  }
}

fn function_key(function: &ExpressionFunction) -> FunctionKey {
  (function.scope, function.name.clone())
}
