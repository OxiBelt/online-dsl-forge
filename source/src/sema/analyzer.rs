mod body_need;
mod functions;
mod phase;
mod support;

use std::collections::{BTreeMap, BTreeSet};

use crate::parser::{
  AstExpression, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, SourceSpan, UnaryOp,
};
use serde::{Deserialize, Serialize};

use crate::sema::dialect::ExpressionDialect;
use crate::sema::profile::{
  BodyNeedSummary, Determinism, RegexPolicy, SecurityProfile, SecurityProfileId,
};
use crate::sema::schema::{
  CapabilityMeta, CapabilityTicket, ExpressionFunctionScope, RuntimeSchema, SignatureMatch,
};
use crate::sema::verified::{
  CompiledExpression, CompiledRegexCache, RegexLiteral, VerifiedExprKind, VerifiedExpression,
  VerifiedProgram, VerifiedProgramParts,
};
use support::{
  ArgsAnalysis, ExprAnalysis, LocalBinding, ObjectOrigin, member_origin, string_literal,
};

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompileOptions {
  pub allow_unknown_variables: bool,
  pub allow_unknown_functions: bool,
  pub allow_unknown_methods: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExpressionFunctionMode {
  #[default]
  Inline,
  CallFrame,
}

#[derive(Debug, Clone)]
pub struct Analyzer {
  profile: SecurityProfile,
  options: CompileOptions,
  dialect: ExpressionDialect,
  expression_function_scope: ExpressionFunctionScope,
  expression_function_mode: ExpressionFunctionMode,
}

impl Analyzer {
  pub fn new(profile: SecurityProfile) -> Self {
    Self {
      profile,
      options: CompileOptions::default(),
      dialect: ExpressionDialect::default(),
      expression_function_scope: ExpressionFunctionScope::Local,
      expression_function_mode: ExpressionFunctionMode::default(),
    }
  }

  pub fn with_options(mut self, options: CompileOptions) -> Self {
    self.options = options;
    self
  }

  pub fn with_dialect(mut self, dialect: ExpressionDialect) -> Self {
    self.dialect = dialect;
    self
  }

  pub fn with_expression_function_scope(mut self, scope: ExpressionFunctionScope) -> Self {
    self.expression_function_scope = scope;
    self
  }

  pub fn with_expression_function_mode(mut self, mode: ExpressionFunctionMode) -> Self {
    self.expression_function_mode = mode;
    self
  }

  pub fn analyze(
    &self,
    expression: &AstExpression,
    schema: &RuntimeSchema,
  ) -> Result<VerifiedProgram, DiagnosticReport> {
    let mut state = AnalyzeState::new(self, schema);
    self.dialect.validate(expression, &mut state.diagnostics);
    state.validate_function_graph();
    let analysis = state.analyze_expression(expression, 0);
    state.validate_program_bounds(&analysis, expression.span);

    if state.diagnostics.is_empty() {
      Ok(VerifiedProgram::new(VerifiedProgramParts {
        ast: expression.clone(),
        root: analysis.expr,
        profile: self.profile.clone(),
        body_need: analysis.body_need,
        static_cost_upper_bound: analysis.cost,
        regex_literals: state.regex_literals,
        regex_cache: state.regex_cache,
        required_capabilities: state.required_capabilities,
        required_capability_metadata: state.required_capability_metadata,
      }))
    } else {
      Err(DiagnosticReport::new(state.diagnostics))
    }
  }
}

pub fn compile_expression(
  expression: &AstExpression,
  schema: &RuntimeSchema,
  options: CompileOptions,
) -> Result<CompiledExpression, DiagnosticReport> {
  Analyzer::new(SecurityProfile::generic_safe())
    .with_options(options)
    .analyze(expression, schema)
    .map(CompiledExpression::new)
}

struct AnalyzeState<'a> {
  analyzer: &'a Analyzer,
  schema: &'a RuntimeSchema,
  diagnostics: Vec<Diagnostic>,
  regex_literals: Vec<RegexLiteral>,
  regex_cache: CompiledRegexCache,
  required_capabilities: BTreeSet<CapabilityTicket>,
  required_capability_metadata: BTreeMap<CapabilityTicket, CapabilityMeta>,
  active_functions: Vec<(ExpressionFunctionScope, String)>,
  local_bindings: Vec<BTreeMap<String, LocalBinding>>,
}

impl<'a> AnalyzeState<'a> {
  fn new(analyzer: &'a Analyzer, schema: &'a RuntimeSchema) -> Self {
    Self {
      analyzer,
      schema,
      diagnostics: Vec::new(),
      regex_literals: Vec::new(),
      regex_cache: CompiledRegexCache::default(),
      required_capabilities: BTreeSet::new(),
      required_capability_metadata: BTreeMap::new(),
      active_functions: Vec::new(),
      local_bindings: Vec::new(),
    }
  }

  fn validate_program_bounds(&mut self, analysis: &ExprAnalysis, span: SourceSpan) {
    if analysis.nodes > self.analyzer.profile.max_ast_nodes {
      self
        .diagnostics
        .push(Diagnostic::new("AST node limit exceeded", span));
    }
    if analysis.cost > self.analyzer.profile.max_cost_units {
      self
        .diagnostics
        .push(Diagnostic::new("static cost limit exceeded", span));
    }
    if matches!(self.analyzer.profile.id, SecurityProfileId::MitigationField)
      && analysis.mitigation_payload
    {
      self.diagnostics.push(Diagnostic::new(
        "MitigationField cannot read request, response, or stream body bytes",
        span,
      ));
    }
  }

  fn analyze_expression(&mut self, expression: &AstExpression, depth: usize) -> ExprAnalysis {
    if depth > self.analyzer.profile.max_call_depth {
      self.diagnostics.push(Diagnostic::new(
        "semantic call depth limit exceeded",
        expression.span,
      ));
    }

    match &expression.kind {
      ExprKind::Null => ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Null, expression.span),
        None,
      ),
      ExprKind::Bool { value } => ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Bool(*value), expression.span),
        None,
      ),
      ExprKind::Int { value } => ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Int(*value), expression.span),
        None,
      ),
      ExprKind::Float { value } => ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Float(*value), expression.span),
        None,
      ),
      ExprKind::String { value } => ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::String(value.clone()), expression.span),
        None,
      ),
      ExprKind::Identifier { name } => self.analyze_identifier(name, expression.span),
      ExprKind::Array { items } => self.analyze_array(items, expression.span, depth),
      ExprKind::Member { receiver, name } => {
        self.analyze_member(receiver, name, expression.span, depth)
      }
      ExprKind::FunctionCall { name, args } => {
        self.analyze_function_call(name, args, expression.span, depth)
      }
      ExprKind::MethodCall {
        receiver,
        name,
        args,
      } => self.analyze_method_call(receiver, name, args, expression.span, depth),
      ExprKind::Unary { op, expr } => self.analyze_unary(*op, expr, expression.span, depth),
      ExprKind::Binary { left, op, right } => {
        self.analyze_binary(left, *op, right, expression.span, depth)
      }
    }
  }

  fn analyze_identifier(&mut self, name: &str, span: SourceSpan) -> ExprAnalysis {
    if let Some(binding) = self.local_binding(name).cloned() {
      return ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Identifier(name.to_string()), span),
        binding.origin,
      )
      .with_path_option(binding.path)
      .with_mitigation_payload(binding.mitigation_payload);
    }
    if !self.analyzer.options.allow_unknown_variables && !self.schema.has_variable(name) {
      self
        .diagnostics
        .push(Diagnostic::new(format!("unknown variable {name}"), span));
    }
    self.validate_variable_phase(name, span);
    ExprAnalysis::leaf(
      VerifiedExpression::new(VerifiedExprKind::Identifier(name.to_string()), span),
      ObjectOrigin::root(name),
    )
    .with_path(vec![name.to_string()])
  }

  fn analyze_array(
    &mut self,
    items: &[AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let mut body_need = BodyNeedSummary::default();
    let mut mitigation_payload = false;
    let mut nodes = 1;
    let mut cost = 1;
    let items = items
      .iter()
      .map(|item| {
        let analysis = self.analyze_expression(item, depth + 1);
        body_need = body_need.merge(analysis.body_need);
        mitigation_payload |= analysis.mitigation_payload;
        nodes += analysis.nodes;
        cost += analysis.cost;
        analysis.expr
      })
      .collect();
    ExprAnalysis::new(
      VerifiedExpression::new(VerifiedExprKind::Array(items), span),
      None,
      None,
      body_need,
      nodes,
      cost,
    )
    .with_mitigation_payload(mitigation_payload)
  }

  fn analyze_member(
    &mut self,
    receiver: &AstExpression,
    name: &str,
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let receiver = self.analyze_expression(receiver, depth + 1);
    let path = receiver.path.as_ref().map(|path| {
      let mut path = path.clone();
      path.push(name.to_string());
      path
    });
    let origin = receiver
      .origin
      .and_then(|origin| member_origin(origin, name));
    let mut body_need = receiver.body_need;
    self.merge_body_access_for_origin(&mut body_need, receiver.origin, name, span);
    if let Some(path) = &path
      && let Some((target, access)) = self.schema.body_access_for_path(path)
    {
      body_need.merge_target(target, access);
    }
    self.validate_origin_phase(origin, span);
    let mitigation_payload = receiver.mitigation_payload
      || origin.is_some_and(ObjectOrigin::is_mitigation_payload_boundary);

    ExprAnalysis::new(
      VerifiedExpression::new(
        VerifiedExprKind::Member {
          receiver: Box::new(receiver.expr),
          name: name.to_string(),
        },
        span,
      ),
      origin,
      path,
      body_need,
      receiver.nodes + 1,
      receiver.cost + 1,
    )
    .with_mitigation_payload(mitigation_payload)
  }

  fn analyze_function_call(
    &mut self,
    name: &str,
    args: &[AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    if let Some(function) = self
      .schema
      .expression_function_for_scope(name, self.current_function_scope())
    {
      return self.analyze_expression_function(function, args, span, depth);
    }

    let capability = self.validate_call(
      "function",
      name,
      args.len(),
      self.schema.function_accepts(name, args.len()),
      self.analyzer.options.allow_unknown_functions,
      span,
    );
    let args_analysis = self.analyze_args(args, depth);
    if let Some(capability) = capability {
      self.validate_capability(capability, span);
      self.validate_regex_args(capability, args, span);
      self.require_capability(capability);
    }
    let capability_ticket = capability.map(CapabilityMeta::ticket);
    ExprAnalysis::new(
      verified_with_capability(
        VerifiedExpression::new(
          VerifiedExprKind::FunctionCall {
            name: name.to_string(),
            args: args_analysis.exprs,
          },
          span,
        ),
        capability_ticket,
      ),
      None,
      None,
      args_analysis.body_need,
      args_analysis.nodes + 1,
      args_analysis.cost + capability.map_or(1, |capability| capability.cost.static_cost()),
    )
    .with_mitigation_payload(args_analysis.mitigation_payload)
  }

  fn analyze_method_call(
    &mut self,
    receiver: &AstExpression,
    name: &str,
    args: &[AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let receiver = self.analyze_expression(receiver, depth + 1);
    let capability = self.validate_call(
      "method",
      name,
      args.len(),
      self.schema.method_accepts(name, args.len()),
      self.analyzer.options.allow_unknown_methods,
      span,
    );
    let args_analysis = self.analyze_args(args, depth);
    let mut body_need = receiver.body_need.merge(args_analysis.body_need);
    let mitigation_payload = receiver.mitigation_payload || args_analysis.mitigation_payload;
    if let Some(capability) = capability {
      self.validate_capability(capability, span);
      self.validate_regex_args(capability, args, span);
      self.merge_body_access_for_method(&mut body_need, receiver.origin, capability);
      self.require_capability(capability);
    }
    let capability_ticket = capability.map(CapabilityMeta::ticket);

    ExprAnalysis::new(
      verified_with_capability(
        VerifiedExpression::new(
          VerifiedExprKind::MethodCall {
            receiver: Box::new(receiver.expr),
            name: name.to_string(),
            args: args_analysis.exprs,
          },
          span,
        ),
        capability_ticket,
      ),
      None,
      None,
      body_need,
      receiver.nodes + args_analysis.nodes + 1,
      receiver.cost
        + args_analysis.cost
        + capability.map_or(1, |capability| capability.cost.static_cost()),
    )
    .with_mitigation_payload(mitigation_payload)
  }

  fn analyze_unary(
    &mut self,
    op: UnaryOp,
    expr: &AstExpression,
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let expr = self.analyze_expression(expr, depth + 1);
    let capability = self
      .schema
      .unary_operator_capability(op)
      .cloned()
      .unwrap_or_else(|| CapabilityMeta::unary_operator(op));
    self.validate_capability(&capability, span);
    self.require_capability(&capability);
    let ticket = capability.ticket();
    ExprAnalysis::new(
      VerifiedExpression::new(
        VerifiedExprKind::Unary {
          op,
          expr: Box::new(expr.expr),
        },
        span,
      )
      .with_capability_ticket(ticket),
      None,
      None,
      expr.body_need,
      expr.nodes + 1,
      expr.cost + capability.cost.static_cost(),
    )
    .with_mitigation_payload(expr.mitigation_payload)
  }

  fn analyze_binary(
    &mut self,
    left: &AstExpression,
    op: BinaryOp,
    right: &AstExpression,
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let left = self.analyze_expression(left, depth + 1);
    let right = self.analyze_expression(right, depth + 1);
    let capability = self
      .schema
      .binary_operator_capability(op)
      .cloned()
      .unwrap_or_else(|| CapabilityMeta::binary_operator(op));
    self.validate_capability(&capability, span);
    self.require_capability(&capability);
    let ticket = capability.ticket();
    ExprAnalysis::new(
      VerifiedExpression::new(
        VerifiedExprKind::Binary {
          left: Box::new(left.expr),
          op,
          right: Box::new(right.expr),
        },
        span,
      )
      .with_capability_ticket(ticket),
      None,
      None,
      left.body_need.merge(right.body_need),
      left.nodes + right.nodes + 1,
      left.cost + right.cost + capability.cost.static_cost(),
    )
    .with_mitigation_payload(left.mitigation_payload || right.mitigation_payload)
  }

  fn analyze_args(&mut self, args: &[AstExpression], depth: usize) -> ArgsAnalysis {
    let mut body_need = BodyNeedSummary::default();
    let mut mitigation_payload = false;
    let mut nodes = 0;
    let mut cost = 0;
    let exprs = args
      .iter()
      .map(|arg| {
        let analysis = self.analyze_expression(arg, depth + 1);
        let binding = LocalBinding::from_analysis(&analysis);
        body_need = body_need.merge(analysis.body_need);
        mitigation_payload |= analysis.mitigation_payload;
        nodes += analysis.nodes;
        cost += analysis.cost;
        (analysis.expr, binding)
      })
      .collect::<Vec<_>>();
    let (exprs, bindings) = exprs.into_iter().unzip();
    ArgsAnalysis {
      exprs,
      bindings,
      body_need,
      mitigation_payload,
      nodes,
      cost,
    }
  }

  fn local_binding(&self, name: &str) -> Option<&LocalBinding> {
    self
      .local_bindings
      .iter()
      .rev()
      .find_map(|bindings| bindings.get(name))
  }

  fn validate_call(
    &mut self,
    kind: &'static str,
    name: &str,
    arity: usize,
    result: SignatureMatch,
    allow_unknown: bool,
    span: SourceSpan,
  ) -> Option<&'a CapabilityMeta> {
    match result {
      SignatureMatch::Matches => {
        if kind == "function" {
          self.schema.function_capability(name, arity)
        } else {
          self.schema.method_capability(name, arity)
        }
      }
      SignatureMatch::Unknown if allow_unknown => None,
      SignatureMatch::Unknown => {
        self
          .diagnostics
          .push(Diagnostic::new(format!("unknown {kind} {name}"), span));
        None
      }
      SignatureMatch::ArityMismatch => {
        self.diagnostics.push(Diagnostic::new(
          format!("{kind} {name} does not accept {arity} arguments"),
          span,
        ));
        None
      }
    }
  }

  fn validate_regex_args(
    &mut self,
    capability: &CapabilityMeta,
    args: &[AstExpression],
    span: SourceSpan,
  ) {
    for regex_arg in &capability.regex_args {
      let Some(arg) = args.get(regex_arg.index) else {
        continue;
      };
      match self.analyzer.profile.default_regex_policy {
        RegexPolicy::Forbid => self.diagnostics.push(Diagnostic::new(
          "regex arguments are forbidden by profile",
          span,
        )),
        RegexPolicy::LiteralOnlyPrecompiled => {
          let Some(pattern) = string_literal(arg) else {
            self.diagnostics.push(Diagnostic::new(
              "regex argument must be a string literal",
              arg.span,
            ));
            continue;
          };
          let literal = RegexLiteral {
            pattern,
            flavor: regex_arg.flavor,
            span: arg.span,
          };
          if let Err(error) = self.regex_cache.insert(&literal) {
            self.diagnostics.push(Diagnostic::new(
              format!("invalid regex pattern: {error}"),
              arg.span,
            ));
          } else {
            self.regex_literals.push(literal);
          }
        }
        RegexPolicy::DynamicWithBudget => {
          if let Some(pattern) = string_literal(arg) {
            let literal = RegexLiteral {
              pattern,
              flavor: regex_arg.flavor,
              span: arg.span,
            };
            if self.regex_cache.insert(&literal).is_ok() {
              self.regex_literals.push(literal);
            }
          }
        }
      }
    }
  }

  fn validate_capability(&mut self, capability: &CapabilityMeta, span: SourceSpan) {
    self.validate_capability_phase(capability, span);
    if matches!(self.analyzer.profile.determinism, Determinism::Required) {
      if !capability.deterministic {
        self.diagnostics.push(Diagnostic::new(
          format!(
            "{} {} is non-deterministic but profile requires determinism",
            support::capability_kind_label(capability.kind),
            capability.name
          ),
          span,
        ));
      }
      if !capability.side_effect_free {
        self.diagnostics.push(Diagnostic::new(
          format!(
            "{} {} has side effects but profile requires side-effect-free capabilities",
            support::capability_kind_label(capability.kind),
            capability.name
          ),
          span,
        ));
      }
    }
  }

  fn require_capability(&mut self, capability: &CapabilityMeta) {
    let ticket = capability.ticket();
    self.required_capabilities.insert(ticket.clone());
    self
      .required_capability_metadata
      .insert(ticket, capability.clone());
  }
}

fn verified_with_capability(
  expression: VerifiedExpression,
  ticket: Option<CapabilityTicket>,
) -> VerifiedExpression {
  if let Some(ticket) = ticket {
    expression.with_capability_ticket(ticket)
  } else {
    expression
  }
}
