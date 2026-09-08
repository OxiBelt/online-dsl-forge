mod body_need;
mod functions;
mod limits;
mod phase;
mod regex;
mod support;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::parser::preflight::preflight_ast;
use crate::parser::validation::validate_ast_syntax_with_scalar_limit;
use crate::parser::{
  AstExpression, BinaryOp, Diagnostic, DiagnosticReport, ExprKind, SourceSpan, UnaryOp,
};
use crate::sema::dialect::ExpressionDialect;
use crate::sema::profile::{
  BodyNeedSummary, Determinism, RegexAdmissionLimits, SecurityProfile, SecurityProfileId,
};
use crate::sema::schema::{
  CapabilityMeta, CapabilityTicket, ExpressionFunctionScope, RegexFlavor, RuntimeSchema,
  SignatureMatch,
};
use crate::sema::verified::{
  CompiledExpression, CompiledRegexCache, RegexLiteral, VerifiedExprKind, VerifiedExpression,
  VerifiedProgram, VerifiedProgramParts,
};
use support::{ArgsAnalysis, ExprAnalysis, LocalBinding, ObjectOrigin, member_origin};

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
  regex_limits: RegexAdmissionLimits,
}

impl Analyzer {
  pub fn new(profile: SecurityProfile) -> Self {
    Self {
      profile,
      options: CompileOptions::default(),
      dialect: ExpressionDialect::default(),
      expression_function_scope: ExpressionFunctionScope::Local,
      expression_function_mode: ExpressionFunctionMode::default(),
      regex_limits: RegexAdmissionLimits::default(),
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

  pub fn with_regex_admission_limits(mut self, limits: RegexAdmissionLimits) -> Self {
    self.regex_limits = limits;
    self
  }

  pub fn analyze<'a>(
    &'a self,
    expression: &'a AstExpression,
    schema: &'a RuntimeSchema,
  ) -> Result<VerifiedProgram, DiagnosticReport> {
    let schema_diagnostics = schema.validated_expression_function_diagnostics();
    if !schema_diagnostics.is_empty() {
      return Err(DiagnosticReport::new(schema_diagnostics));
    }
    let max_depth = self.profile.max_call_depth.min(128).saturating_add(1);
    let mut preflight_diagnostics = Vec::new();
    match preflight_ast(expression, self.profile.max_ast_nodes, max_depth) {
      Ok(_) => {
        if let Err(report) =
          validate_ast_syntax_with_scalar_limit(expression, limits::MAX_LOWERED_SCALAR_BYTES)
        {
          preflight_diagnostics.extend(report.diagnostics.into_iter().map(|mut diagnostic| {
            if diagnostic.message == "AST scalar byte limit exceeded" {
              diagnostic.message = "lowered scalar byte limit exceeded".to_string();
            }
            diagnostic
          }));
        }
      }
      Err(report) => preflight_diagnostics.extend(report.diagnostics),
    }
    let diagnostic_limit = schema.expression_function_limits().max_diagnostics.max(1);
    for function in schema.expression_functions() {
      if preflight_diagnostics.len() >= diagnostic_limit {
        break;
      }
      if let Err(report) =
        preflight_ast(&function.expression, self.profile.max_ast_nodes, max_depth)
      {
        let remaining = diagnostic_limit - preflight_diagnostics.len();
        preflight_diagnostics.extend(report.diagnostics.into_iter().take(remaining));
      }
    }
    if !preflight_diagnostics.is_empty() {
      return Err(DiagnosticReport::new(preflight_diagnostics));
    }

    let mut state = AnalyzeState::new(self, schema);
    self.dialect.validate(expression, &mut state.diagnostics);
    state.validate_function_graph();
    if !state.preflight_lowering(expression) {
      return Err(DiagnosticReport::new(state.diagnostics));
    }
    let mut analysis = state.analyze_expression(expression, 0);
    state.merge_body_access_for_exposed_origin(&mut analysis.body_need, analysis.origin);
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
  regex_attempts: BTreeMap<RegexFlavor, BTreeMap<String, Option<String>>>,
  regex_attempt_count: usize,
  regex_source_bytes: usize,
  regex_count_limit_reported: bool,
  regex_source_limit_reported: bool,
  required_capabilities: BTreeSet<CapabilityTicket>,
  required_capability_metadata: BTreeMap<CapabilityTicket, CapabilityMeta>,
  active_functions: Vec<(ExpressionFunctionScope, String)>,
  local_bindings: Vec<BTreeMap<String, LocalBinding>>,
  inline_bindings: Vec<(&'a [String], &'a [AstExpression])>,
  lowered_nodes: usize,
  lowered_scalar_bytes: usize,
  node_limit_reported: bool,
  depth_limit_reported: bool,
  scalar_byte_limit_reported: bool,
}

impl<'a> AnalyzeState<'a> {
  fn new(analyzer: &'a Analyzer, schema: &'a RuntimeSchema) -> Self {
    Self {
      analyzer,
      schema,
      diagnostics: Vec::new(),
      regex_literals: Vec::new(),
      regex_cache: CompiledRegexCache::default(),
      regex_attempts: BTreeMap::new(),
      regex_attempt_count: 0,
      regex_source_bytes: 0,
      regex_count_limit_reported: false,
      regex_source_limit_reported: false,
      required_capabilities: BTreeSet::new(),
      required_capability_metadata: BTreeMap::new(),
      active_functions: Vec::new(),
      local_bindings: Vec::new(),
      inline_bindings: Vec::new(),
      lowered_nodes: 0,
      lowered_scalar_bytes: 0,
      node_limit_reported: false,
      depth_limit_reported: false,
      scalar_byte_limit_reported: false,
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
    if let Some(limit) = self.analyzer.profile.body_access_limit
      && !limit.allows(analysis.body_need)
    {
      self.diagnostics.push(Diagnostic::new(
        "body access limit exceeded by profile",
        span,
      ));
    }
    if matches!(self.analyzer.profile.id, SecurityProfileId::MitigationField)
      && (analysis.mitigation_payload || analysis.body_need.reads_payload())
    {
      self.diagnostics.push(Diagnostic::new(
        "MitigationField cannot read request, response, or stream body bytes",
        span,
      ));
    }
  }

  fn analyze_expression(&mut self, expression: &'a AstExpression, depth: usize) -> ExprAnalysis {
    if depth > self.analyzer.profile.max_call_depth.min(128) {
      self.report_depth_limit(expression.span);
      return ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Null, expression.span),
        None,
      );
    }
    if let ExprKind::Identifier { name } = &expression.kind
      && let Some(binding) = self.inline_binding(name)
      && let Some(frame) = self.inline_bindings.pop()
    {
      let analysis = self.analyze_expression(binding, depth);
      self.inline_bindings.push(frame);
      return analysis;
    }
    if !self.charge_node(expression.span) {
      return ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Null, expression.span),
        None,
      );
    }
    if !self.charge_expression_scalar_bytes(expression) {
      return ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Null, expression.span),
        None,
      );
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
      let mut analysis = ExprAnalysis::leaf(
        VerifiedExpression::new(VerifiedExprKind::Identifier(name.to_string()), span),
        binding.origin,
      )
      .with_path_option(binding.path)
      .with_mitigation_payload(binding.mitigation_payload);
      self.merge_body_access_for_path(&mut analysis.body_need, analysis.path.as_deref());
      return analysis;
    }
    if !self.analyzer.options.allow_unknown_variables && !self.schema.has_variable(name) {
      self
        .diagnostics
        .push(Diagnostic::new(format!("unknown variable {name}"), span));
    }
    self.validate_variable_phase(name, span);
    let mut analysis = ExprAnalysis::leaf(
      VerifiedExpression::new(VerifiedExprKind::Identifier(name.to_string()), span),
      ObjectOrigin::root(name),
    )
    .with_path(vec![name.to_string()]);
    self.merge_body_access_for_path(&mut analysis.body_need, analysis.path.as_deref());
    analysis
  }

  fn analyze_array(
    &mut self,
    items: &'a [AstExpression],
    span: SourceSpan,
    depth: usize,
  ) -> ExprAnalysis {
    let mut body_need = BodyNeedSummary::default();
    let mut mitigation_payload = false;
    let mut nodes = 1_usize;
    let mut cost = 1_u64;
    let items = items
      .iter()
      .map(|item| {
        let analysis = self.analyze_expression(item, depth + 1);
        body_need = body_need
          .merge(self.body_need_for_consumed_analysis(analysis.body_need, analysis.origin));
        mitigation_payload |= analysis.mitigation_payload;
        nodes = nodes.saturating_add(analysis.nodes);
        cost = cost.saturating_add(analysis.cost);
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
    receiver: &'a AstExpression,
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
      receiver.nodes.saturating_add(1),
      receiver.cost.saturating_add(1),
    )
    .with_mitigation_payload(mitigation_payload)
  }

  fn analyze_function_call(
    &mut self,
    name: &str,
    args: &'a [AstExpression],
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
    if let Some(capability) = capability {
      self.validate_capability(capability, span);
      self.validate_regex_args(capability, args, span);
      self.require_capability(capability);
    }
    let args_analysis = self.analyze_args(args, depth);
    let capability_ticket = capability.map(CapabilityMeta::ticket);
    let body_need = args_analysis.consumed_body_need;
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
      body_need,
      args_analysis.nodes.saturating_add(1),
      args_analysis
        .cost
        .saturating_add(capability.map_or(1, |capability| capability.cost.static_cost())),
    )
    .with_mitigation_payload(args_analysis.mitigation_payload)
  }

  fn analyze_method_call(
    &mut self,
    receiver: &'a AstExpression,
    name: &str,
    args: &'a [AstExpression],
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
    if let Some(capability) = capability {
      self.validate_regex_args(capability, args, span);
    }
    let args_analysis = self.analyze_args(args, depth);
    let receiver_body_need =
      self.body_need_for_consumed_analysis(receiver.body_need, receiver.origin);
    let mut body_need = receiver_body_need.merge(args_analysis.consumed_body_need);
    let mitigation_payload = receiver.mitigation_payload || args_analysis.mitigation_payload;
    if let Some(capability) = capability {
      self.validate_capability(capability, span);
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
      receiver
        .nodes
        .checked_add(args_analysis.nodes)
        .and_then(|nodes| nodes.checked_add(1))
        .unwrap_or(usize::MAX),
      receiver
        .cost
        .checked_add(args_analysis.cost)
        .and_then(|cost| {
          cost.checked_add(capability.map_or(1, |capability| capability.cost.static_cost()))
        })
        .unwrap_or(u64::MAX),
    )
    .with_mitigation_payload(mitigation_payload)
  }

  fn analyze_unary(
    &mut self,
    op: UnaryOp,
    expr: &'a AstExpression,
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
    let body_need = self.body_need_for_consumed_analysis(expr.body_need, expr.origin);
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
      body_need,
      expr.nodes.saturating_add(1),
      expr.cost.saturating_add(capability.cost.static_cost()),
    )
    .with_mitigation_payload(expr.mitigation_payload)
  }

  fn analyze_binary(
    &mut self,
    left: &'a AstExpression,
    op: BinaryOp,
    right: &'a AstExpression,
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
    let left_body_need = self.body_need_for_consumed_analysis(left.body_need, left.origin);
    let right_body_need = self.body_need_for_consumed_analysis(right.body_need, right.origin);
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
      left_body_need.merge(right_body_need),
      left
        .nodes
        .checked_add(right.nodes)
        .and_then(|nodes| nodes.checked_add(1))
        .unwrap_or(usize::MAX),
      left
        .cost
        .checked_add(right.cost)
        .and_then(|cost| cost.checked_add(capability.cost.static_cost()))
        .unwrap_or(u64::MAX),
    )
    .with_mitigation_payload(left.mitigation_payload || right.mitigation_payload)
  }

  fn analyze_args(&mut self, args: &'a [AstExpression], depth: usize) -> ArgsAnalysis {
    let mut body_need = BodyNeedSummary::default();
    let mut consumed_body_need = BodyNeedSummary::default();
    let mut mitigation_payload = false;
    let mut nodes = 0_usize;
    let mut cost = 0_u64;
    let exprs = args
      .iter()
      .map(|arg| {
        let analysis = self.analyze_expression(arg, depth + 1);
        let binding = LocalBinding::from_analysis(&analysis);
        body_need = body_need.merge(analysis.body_need);
        consumed_body_need = consumed_body_need
          .merge(self.body_need_for_consumed_analysis(analysis.body_need, analysis.origin));
        mitigation_payload |= analysis.mitigation_payload;
        nodes = nodes.saturating_add(analysis.nodes);
        cost = cost.saturating_add(analysis.cost);
        (analysis.expr, binding)
      })
      .collect::<Vec<_>>();
    let (exprs, bindings) = exprs.into_iter().unzip();
    ArgsAnalysis {
      exprs,
      bindings,
      body_need,
      consumed_body_need,
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
