use std::collections::{BTreeMap, HashSet};

use crate::parser::{AstExpression, Diagnostic, SourceSpan};
use crate::sema::schema::{ExpressionFunction, ExpressionFunctionScope, SignatureMatch};
use crate::sema::verified::{VerifiedExprKind, VerifiedExpression};

use super::support::{ExprAnalysis, LocalBinding, function_calls, substitute_expression};
use super::{AnalyzeState, ExpressionFunctionMode};

type FunctionKey = (ExpressionFunctionScope, String);

impl<'a> AnalyzeState<'a> {
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

    let mut permanent = HashSet::new();
    let mut temporary = HashSet::new();
    for function in self.schema.expression_functions() {
      self.validate_function_node(function, &mut permanent, &mut temporary);
    }
  }

  pub(super) fn analyze_expression_function(
    &mut self,
    function: &ExpressionFunction,
    args: &[AstExpression],
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

    let replacements = function
      .params
      .iter()
      .cloned()
      .zip(args.iter().cloned())
      .collect::<BTreeMap<_, _>>();
    let substituted = substitute_expression(&function.expression, &replacements);
    self.active_functions.push(key);
    let mut analysis = self.analyze_expression(&substituted, depth + 1);
    self.active_functions.pop();
    analysis.cost += 1;
    analysis.nodes += 1;
    analysis
  }

  fn analyze_expression_function_call_frame(
    &mut self,
    function: &ExpressionFunction,
    args: &[AstExpression],
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
      args_analysis.nodes + body.nodes + 1,
      args_analysis.cost + body.cost + 1,
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

  fn validate_function_node(
    &mut self,
    function: &ExpressionFunction,
    permanent: &mut HashSet<FunctionKey>,
    temporary: &mut HashSet<FunctionKey>,
  ) {
    let key = function_key(function);
    if permanent.contains(&key) {
      return;
    }
    if !temporary.insert(key.clone()) {
      self.diagnostics.push(Diagnostic::new(
        format!("recursive expression function {}", function.name),
        function.expression.span,
      ));
      return;
    }

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
      self.validate_function_node(callee, permanent, temporary);
    }

    temporary.remove(&key);
    permanent.insert(key);
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
