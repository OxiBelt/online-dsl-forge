use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::parser::{AstExpression, Diagnostic, ExprKind, SourceSpan};
use crate::sema::schema::{ExpressionFunction, ExpressionFunctionScope, SignatureMatch};
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
    for diagnostic in self.schema.expression_function_diagnostics() {
      self.diagnostics.push(diagnostic.diagnostic());
    }

    for function in self.schema.expression_functions() {
      self
        .analyzer
        .dialect
        .validate(&function.expression, &mut self.diagnostics);
      self.validate_function_signature(function);
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
      self.diagnostics.push(Diagnostic::new(
        format!(
          "function {} does not accept {} arguments",
          function.name,
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
      self.diagnostics.push(Diagnostic::new(
        format!("recursive expression function {}", function.name),
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
      self.diagnostics.push(Diagnostic::new(
        format!(
          "function {} does not accept {} arguments",
          function.name,
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
      self.diagnostics.push(Diagnostic::new(
        format!("recursive expression function {}", function.name),
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

  fn validate_function_signature(&mut self, function: &ExpressionFunction) {
    if !valid_oxirule_identifier(&function.name) || is_top_level_oxirule_object(&function.name) {
      self.diagnostics.push(Diagnostic::new(
        format!("function name {} must be a valid identifier", function.name),
        function.expression.span,
      ));
    }

    let mut params = HashSet::new();
    for param in &function.params {
      if !valid_oxirule_identifier(param) || is_top_level_oxirule_object(param) {
        self.diagnostics.push(Diagnostic::new(
          format!(
            "function {} parameter {param} must be a valid identifier",
            function.name
          ),
          function.expression.span,
        ));
      }
      if !params.insert(param.as_str()) {
        self.diagnostics.push(Diagnostic::new(
          format!(
            "function {} contains duplicate parameter {param}",
            function.name
          ),
          function.expression.span,
        ));
      }
    }
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
          self.diagnostics.push(Diagnostic::new(
            format!(
              "function {} does not accept {} arguments",
              call.name, call.arity
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
            self.diagnostics.push(Diagnostic::new(
              format!("recursive expression function {}", function.name),
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
                self.diagnostics.push(Diagnostic::new(
                  format!("recursive expression function {}", function.name),
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
      SignatureMatch::Unknown => self
        .diagnostics
        .push(Diagnostic::new(format!("unknown function {name}"), span)),
      SignatureMatch::ArityMismatch => self.diagnostics.push(Diagnostic::new(
        format!("function {name} does not accept {arity} arguments"),
        span,
      )),
    }
  }
}

fn function_key(function: &ExpressionFunction) -> FunctionKey {
  (function.scope, function.name.clone())
}

fn valid_oxirule_identifier(identifier: &str) -> bool {
  let mut chars = identifier.chars();
  let Some(first) = chars.next() else {
    return false;
  };
  (first.is_ascii_alphabetic() || first == '_')
    && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    && !is_reserved_identifier(identifier)
}

fn is_reserved_identifier(identifier: &str) -> bool {
  matches!(
    identifier,
    "if"
      | "else"
      | "for"
      | "while"
      | "do"
      | "switch"
      | "let"
      | "const"
      | "function"
      | "import"
      | "export"
      | "new"
      | "try"
      | "catch"
      | "throw"
      | "await"
      | "return"
      | "true"
      | "false"
      | "null"
  )
}

fn is_top_level_oxirule_object(identifier: &str) -> bool {
  matches!(
    identifier,
    "Context" | "Request" | "DynamicPolicy" | "Response" | "Stream"
  )
}
