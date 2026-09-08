#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use arbitrary::{Arbitrary, Unstructured};
use online_dsl_forge::{
  Analyzer, AstExpression, AstFormatLimits, CompileOptions, Diagnostic, DiagnosticReport,
  EvalError, EvalLimits, ExprKind, ExpressionDialect, MapRuntime, MemoryFileResolver, ParseLimits,
  Phase, RulepackRenderLimits, RulepackRenderOptions, RuntimeResourceLimits, RuntimeSchema,
  SecurityProfile, SourceSpan, Value, compile_expression, evaluate_with_resource_limits,
  format_expression, format_expression_with_limits, inspect_rulepack_inputs_with_limits,
  inspect_rulepack_with_limits, parse_expression, parse_expression_with_limits,
  referenced_rulepack_files_with_limits, render_rulepack_bundle_with_limits,
  render_rulepack_for_install_with_limits, render_text_with_limits,
};
use serde::Serialize;

const BINDINGS_MARKER: &[u8] = b"\n---BINDINGS---\n";
const FILES_MARKER: &[u8] = b"\n---FILES---\n";

pub fn exercise_target(target: &str, data: &[u8]) {
  match target {
    "dsl_expression" => exercise_dsl_expression(data),
    "expression_pipeline" => exercise_expression_pipeline(data),
    "rulepack_render" => exercise_rulepack_render(data),
    other => panic!("unknown fuzz target {other}"),
  }
}

pub fn exercise_dsl_expression(data: &[u8]) {
  let source = String::from_utf8_lossy(data);
  let selected_limits = parse_limits(data);

  match online_dsl_forge::lexer::tokenize(&source) {
    Ok(tokens) => {
      assert!(!tokens.is_empty(), "successful lexing must emit EOF");
      assert!(
        matches!(
          tokens.last().map(|token| &token.kind),
          Some(online_dsl_forge::lexer::TokenKind::Eof)
        ),
        "successful lexing must end with EOF"
      );
      let mut previous_end = 0;
      for token in tokens {
        validate_span(token.span, &source);
        assert!(
          token.span.start >= previous_end,
          "token spans must be monotonically ordered"
        );
        previous_end = token.span.end;
      }
    }
    Err(diagnostics) => validate_diagnostics(&diagnostics, &source),
  }

  match online_dsl_forge::lexer::tokenize_with_limits(&source, selected_limits) {
    Ok(tokens) => {
      assert!(
        matches!(
          tokens.last().map(|token| &token.kind),
          Some(online_dsl_forge::lexer::TokenKind::Eof)
        ),
        "bounded successful lexing must end with EOF"
      );
      for token in tokens {
        validate_span(token.span, &source);
      }
    }
    Err(diagnostics) => validate_diagnostics(&diagnostics, &source),
  }

  match parse_expression_with_limits(&source, selected_limits) {
    Ok(ast) => {
      validate_ast_spans(&ast, &source);
      match format_expression_with_limits(&ast, format_limits(data)) {
        Ok(canonical) => {
          let reparsed = parse_expression(&canonical).expect("bounded canonical output must parse");
          assert_eq!(
            format_expression(&reparsed),
            canonical,
            "bounded canonical formatting must be idempotent"
          );
        }
        Err(report) => validate_diagnostics(&report.diagnostics, &source),
      }
    }
    Err(report) => validate_diagnostics(&report.diagnostics, &source),
  }

  match parse_expression(&source) {
    Ok(ast) => {
      validate_ast_spans(&ast, &source);
      let serialized = serde_json::to_vec(&ast).expect("AST serialization must succeed");
      let decoded: AstExpression =
        serde_json::from_slice(&serialized).expect("serialized AST must deserialize");
      assert_eq!(decoded, ast, "AST serde round trips must be exact");

      let canonical = format_expression(&ast);
      let reparsed = parse_expression(&canonical).expect("canonical output must parse");
      validate_ast_spans(&reparsed, &canonical);
      assert_eq!(
        format_expression(&reparsed),
        canonical,
        "canonical formatting must be idempotent"
      );
    }
    Err(report) => validate_diagnostics(&report.diagnostics, &source),
  }
}

pub fn exercise_expression_pipeline(data: &[u8]) {
  let (selectors, payload) = selectors(data, 7);
  let (source_bytes, bindings_bytes) = split_once(payload, BINDINGS_MARKER);
  let source = String::from_utf8_lossy(source_bytes);
  let bindings = bindings_bytes
    .and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
    .unwrap_or_else(|| serde_json::json!({}));
  let resource_limits = runtime_resource_limits(&selectors[2..7]);

  if let Err(error) = MapRuntime::from_json_bindings_with_limits(bindings.clone(), resource_limits)
  {
    validate_span(error.span, &source);
  }

  let ast = match parse_expression(&source) {
    Ok(ast) => ast,
    Err(report) => {
      validate_diagnostics(&report.diagnostics, &source);
      return;
    }
  };
  validate_ast_spans(&ast, &source);

  exercise_selected_analysis(&ast, &source, &selectors);

  let runtime = match MapRuntime::from_json_bindings(bindings) {
    Ok(runtime) => runtime,
    Err(error) => {
      validate_span(error.span, &source);
      return;
    }
  };
  let options = compile_options(selectors[1]);
  let schema = runtime.schema();
  let first = compile_expression(&ast, &schema, options);
  let second = compile_expression(&ast, &schema, options);
  compare_compile_results(
    first,
    second,
    &runtime,
    eval_limits(&selectors[2..7]),
    resource_limits,
    &source,
  );

  if compile_expression(&ast, &schema, CompileOptions::default()).is_ok() {
    assert!(
      compile_expression(
        &ast,
        &schema,
        CompileOptions {
          allow_unknown_variables: true,
          allow_unknown_functions: true,
          allow_unknown_methods: true,
        }
      )
      .is_ok(),
      "relaxing unknown-name checks must not reject a strict success"
    );
  }
}

pub fn exercise_rulepack_render(data: &[u8]) {
  let (selectors, payload) = selectors(data, 2);
  let (manifest_bytes, files_bytes) = split_once(payload, FILES_MARKER);
  let manifest = String::from_utf8_lossy(manifest_bytes);
  let structured = files_bytes
    .and_then(|raw| serde_json::from_slice::<RulepackStructuredInput>(raw).ok())
    .unwrap_or_default();

  let variables = structured
    .variables
    .into_iter()
    .take(16)
    .map(|(name, value)| (bounded_string(name, 128), bounded_string(value, 4096)))
    .collect::<BTreeMap<_, _>>();
  let options = RulepackRenderOptions {
    variables: variables.clone(),
    source_commit: structured
      .source_commit
      .map(|value| bounded_string(value, 128)),
    pin_variables: selectors[0] & 1 == 1,
    ..RulepackRenderOptions::default()
  };
  let mut resolver = MemoryFileResolver::new();
  for (path, content) in structured.files.into_iter().take(8) {
    resolver.insert(bounded_string(path, 512), bounded_string(content, 8192));
  }
  let source = if selectors[1] & 1 == 0 {
    "fuzz rulepack"
  } else {
    "fuzz rulepack unicode ☃"
  };
  let limits = rulepack_limits(&selectors);

  assert_deterministic(
    inspect_rulepack_inputs_with_limits(&manifest, source, limits),
    inspect_rulepack_inputs_with_limits(&manifest, source, limits),
  );
  assert_deterministic(
    inspect_rulepack_with_limits(&manifest, source, options.clone(), limits),
    inspect_rulepack_with_limits(&manifest, source, options.clone(), limits),
  );
  assert_deterministic(
    referenced_rulepack_files_with_limits(&manifest, source, options.clone(), limits),
    referenced_rulepack_files_with_limits(&manifest, source, options.clone(), limits),
  );
  assert_deterministic(
    render_rulepack_for_install_with_limits(&manifest, source, options.clone(), limits),
    render_rulepack_for_install_with_limits(&manifest, source, options.clone(), limits),
  );
  assert_deterministic(
    render_rulepack_bundle_with_limits(&manifest, source, options.clone(), &resolver, limits),
    render_rulepack_bundle_with_limits(&manifest, source, options, &resolver, limits),
  );
  assert_deterministic(
    render_text_with_limits(&manifest, &variables, limits),
    render_text_with_limits(&manifest, &variables, limits),
  );
}

#[derive(Debug, Default, serde::Deserialize)]
struct RulepackStructuredInput {
  #[serde(default)]
  variables: BTreeMap<String, String>,
  #[serde(default)]
  files: BTreeMap<String, String>,
  #[serde(default)]
  source_commit: Option<String>,
}

fn selectors(data: &[u8], count: usize) -> (Vec<u8>, &[u8]) {
  let mut raw = Unstructured::new(data);
  let values = (0..count)
    .map(|_| u8::arbitrary(&mut raw).unwrap_or_default())
    .collect();
  (values, raw.take_rest())
}

fn split_once<'a>(data: &'a [u8], marker: &[u8]) -> (&'a [u8], Option<&'a [u8]>) {
  match data
    .windows(marker.len())
    .position(|window| window == marker)
  {
    Some(index) => (&data[..index], Some(&data[index + marker.len()..])),
    None => (data, None),
  }
}

fn bounded_string(mut value: String, max_bytes: usize) -> String {
  if value.len() <= max_bytes {
    return value;
  }
  let mut end = max_bytes;
  while !value.is_char_boundary(end) {
    end -= 1;
  }
  value.truncate(end);
  value
}

fn compile_options(selector: u8) -> CompileOptions {
  CompileOptions {
    allow_unknown_variables: selector & 1 != 0,
    allow_unknown_functions: selector & 2 != 0,
    allow_unknown_methods: selector & 4 != 0,
  }
}

fn eval_limits(selectors: &[u8]) -> EvalLimits {
  EvalLimits {
    max_steps: 1 + usize::from(selectors[0]) * 8,
    max_depth: 1 + usize::from(selectors[1] % 64),
    max_string_bytes: 1 + usize::from(selectors[2]) * 64,
    max_array_items: 1 + usize::from(selectors[3]),
  }
}

fn parse_limits(data: &[u8]) -> ParseLimits {
  let select = |index: usize| usize::from(data.get(index).copied().unwrap_or_default());
  ParseLimits {
    max_source_bytes: select(0) * 32,
    max_decoded_scalar_bytes: select(1) * 32,
    max_tokens: select(2) * 8,
    max_diagnostics: select(3) * 4,
    max_ast_nodes: select(4) * 8,
    max_collection_items: select(5) * 4,
  }
}

fn format_limits(data: &[u8]) -> AstFormatLimits {
  let select = |index: usize| usize::from(data.get(index).copied().unwrap_or_default());
  AstFormatLimits {
    max_depth: select(6) % 128,
    max_nodes: select(7) * 8,
    max_output_bytes: select(8) * 32,
  }
}

fn runtime_resource_limits(selectors: &[u8]) -> RuntimeResourceLimits {
  RuntimeResourceLimits {
    max_value_depth: 1 + usize::from(selectors[0] % 32),
    max_value_nodes: 1 + usize::from(selectors[1]) * 16,
    max_value_items: usize::from(selectors[2]) * 16,
    max_value_bytes: 1 + usize::from(selectors[3]) * 64,
    max_total_value_bytes: 1 + usize::from(selectors[4]) * 256,
  }
}

fn rulepack_limits(selectors: &[u8]) -> RulepackRenderLimits {
  let input = 1 + usize::from(selectors[0]) * 256;
  let output = 1 + usize::from(selectors[1]) * 512;
  RulepackRenderLimits {
    max_manifest_bytes: input,
    max_referenced_file_bytes: input,
    max_variable_value_bytes: 1 + usize::from(selectors[0]) * 32,
    max_total_variable_bytes: input,
    max_variables: 1 + usize::from(selectors[1] % 32),
    max_rulepack_files: 1 + usize::from(selectors[0] % 32),
    max_placeholders: usize::from(selectors[1]) * 16,
    max_total_input_bytes: input * 2,
    max_total_output_bytes: output,
  }
}

fn exercise_selected_analysis(ast: &AstExpression, source: &str, selectors: &[u8]) {
  let (profile, schema, dialect) = match selectors[0] % 9 {
    0 => (
      SecurityProfile::generic_safe(),
      RuntimeSchema::new(),
      ExpressionDialect::Generic,
    ),
    1 => (
      SecurityProfile::generic_transform(),
      RuntimeSchema::new(),
      ExpressionDialect::Generic,
    ),
    2 => (
      SecurityProfile::waf_request(),
      RuntimeSchema::waf(),
      ExpressionDialect::Generic,
    ),
    3 => (
      SecurityProfile::waf_response(),
      RuntimeSchema::waf(),
      ExpressionDialect::Generic,
    ),
    4 => (
      SecurityProfile::waf_stream(),
      RuntimeSchema::waf(),
      ExpressionDialect::Generic,
    ),
    5 => (
      SecurityProfile::mitigation_field(Phase::Request),
      RuntimeSchema::waf(),
      ExpressionDialect::Generic,
    ),
    6 => (
      SecurityProfile::oxirule_waf_request(),
      RuntimeSchema::oxirule_waf(),
      ExpressionDialect::OxiRuleV1,
    ),
    7 => (
      SecurityProfile::oxirule_waf_response(),
      RuntimeSchema::oxirule_waf(),
      ExpressionDialect::OxiRuleV1,
    ),
    _ => (
      SecurityProfile::oxirule_waf_stream(),
      RuntimeSchema::oxirule_waf(),
      ExpressionDialect::OxiRuleV1,
    ),
  };
  let options = compile_options(selectors[1]);
  let first = Analyzer::new(profile.clone())
    .with_options(options)
    .with_dialect(dialect)
    .analyze(ast, &schema);
  let second = Analyzer::new(profile.clone())
    .with_options(options)
    .with_dialect(dialect)
    .analyze(ast, &schema);

  match (first, second) {
    (Err(left), Err(right)) => {
      validate_diagnostics(&left.diagnostics, source);
      validate_diagnostics(&right.diagnostics, source);
      assert_eq!(left, right, "semantic diagnostics must be deterministic");
    }
    (Ok(left), Ok(right)) => {
      assert_eq!(left.ast(), ast, "analysis must retain the parsed AST");
      assert_eq!(right.ast(), ast, "analysis must retain the parsed AST");
      assert_eq!(left.profile(), right.profile());
      assert_eq!(left.body_need(), right.body_need());
      assert_eq!(
        left.static_cost_upper_bound(),
        right.static_cost_upper_bound()
      );
      assert_eq!(left.regex_literals(), right.regex_literals());
      assert_eq!(left.required_capabilities(), right.required_capabilities());
      assert_eq!(
        left.required_capability_metadata(),
        right.required_capability_metadata()
      );
      assert!(
        left.static_cost_upper_bound() <= profile.max_cost_units,
        "successful analysis must respect the profile cost limit"
      );
      if let Some(limit) = profile.body_access_limit {
        assert!(
          limit.allows(left.body_need()),
          "successful analysis must respect the profile body limit"
        );
      }
      let metadata = left
        .required_capability_metadata()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
      assert_eq!(
        &metadata,
        left.required_capabilities(),
        "required capabilities and metadata must stay aligned"
      );
      let regex_keys = left
        .regex_literals()
        .iter()
        .map(|literal| (literal.flavor, literal.pattern.as_str()))
        .collect::<BTreeSet<_>>();
      assert_eq!(
        left.regex_cache().len(),
        regex_keys.len(),
        "regex cache entries must match unique flavor and pattern pairs"
      );
      for literal in left.regex_literals() {
        assert!(
          left
            .regex_cache()
            .get(literal.flavor, &literal.pattern)
            .is_some(),
          "every admitted regex literal must be precompiled"
        );
      }
    }
    _ => panic!("semantic analysis must return the same result class twice"),
  }
}

fn compare_compile_results(
  first: Result<online_dsl_forge::CompiledExpression, DiagnosticReport>,
  second: Result<online_dsl_forge::CompiledExpression, DiagnosticReport>,
  runtime: &MapRuntime,
  limits: EvalLimits,
  resource_limits: RuntimeResourceLimits,
  source: &str,
) {
  match (first, second) {
    (Err(left), Err(right)) => {
      validate_diagnostics(&left.diagnostics, source);
      validate_diagnostics(&right.diagnostics, source);
      assert_eq!(left, right, "compile diagnostics must be deterministic");
    }
    (Ok(left), Ok(right)) => {
      let first_result = evaluate_with_resource_limits(&left, runtime, limits, resource_limits);
      let second_result = evaluate_with_resource_limits(&right, runtime, limits, resource_limits);
      assert_eq!(
        evaluation_fingerprint(&first_result),
        evaluation_fingerprint(&second_result),
        "evaluation must be deterministic"
      );

      let canonical = format_expression(left.ast());
      let canonical_ast =
        parse_expression(&canonical).expect("compiled canonical output must parse");
      let canonical_compiled = compile_expression(
        &canonical_ast,
        &runtime.schema(),
        CompileOptions {
          allow_unknown_variables: true,
          allow_unknown_functions: true,
          allow_unknown_methods: true,
        },
      );
      if let Ok(canonical_compiled) = canonical_compiled {
        let canonical_result =
          evaluate_with_resource_limits(&canonical_compiled, runtime, limits, resource_limits);
        assert_eq!(
          evaluation_fingerprint(&first_result),
          evaluation_fingerprint(&canonical_result),
          "canonical formatting must preserve evaluation outcomes"
        );
      }
    }
    _ => panic!("compilation must return the same result class twice"),
  }
}

fn evaluation_fingerprint(result: &Result<Value, EvalError>) -> String {
  match result {
    Ok(value) => format!("ok:{}", value_fingerprint(value)),
    Err(error) => format!("error:{}", error.message),
  }
}

fn value_fingerprint(value: &Value) -> String {
  match value {
    Value::Null => "null".to_string(),
    Value::Bool(value) => format!("bool:{value}"),
    Value::Int(value) => format!("int:{value}"),
    Value::Float(value) => format!("float:{:016x}", value.to_bits()),
    Value::String(value) => format!("string:{value:?}"),
    Value::Array(values) => format!(
      "array:[{}]",
      values
        .iter()
        .map(value_fingerprint)
        .collect::<Vec<_>>()
        .join(",")
    ),
    Value::Object(values) => format!(
      "object:{{{}}}",
      values
        .iter()
        .map(|(name, value)| format!("{name:?}:{}", value_fingerprint(value)))
        .collect::<Vec<_>>()
        .join(",")
    ),
  }
}

fn assert_deterministic<T, E>(first: Result<T, E>, second: Result<T, E>)
where
  T: Serialize,
  E: ToString,
{
  let fingerprint = |result: Result<T, E>| match result {
    Ok(value) => format!(
      "ok:{}",
      serde_json::to_string(&value).expect("public rulepack values must serialize")
    ),
    Err(error) => format!("error:{}", error.to_string()),
  };
  assert_eq!(
    fingerprint(first),
    fingerprint(second),
    "rulepack operations must be deterministic"
  );
}

fn validate_diagnostics(diagnostics: &[Diagnostic], source: &str) {
  assert!(!diagnostics.is_empty(), "error reports must not be empty");
  for diagnostic in diagnostics {
    validate_span(diagnostic.span, source);
  }
}

fn validate_span(span: SourceSpan, source: &str) {
  assert!(span.start <= span.end, "span start must not exceed its end");
  assert!(
    span.end <= source.len(),
    "span must remain within the source"
  );
  assert!(
    source.is_char_boundary(span.start) && source.is_char_boundary(span.end),
    "span endpoints must be UTF-8 character boundaries"
  );
}

fn validate_ast_spans(expression: &AstExpression, source: &str) {
  validate_span(expression.span, source);
  match &expression.kind {
    ExprKind::Array { items } => {
      for item in items {
        validate_ast_spans(item, source);
      }
    }
    ExprKind::Member { receiver, .. } => validate_ast_spans(receiver, source),
    ExprKind::FunctionCall { args, .. } => {
      for arg in args {
        validate_ast_spans(arg, source);
      }
    }
    ExprKind::MethodCall { receiver, args, .. } => {
      validate_ast_spans(receiver, source);
      for arg in args {
        validate_ast_spans(arg, source);
      }
    }
    ExprKind::Unary { expr, .. } => validate_ast_spans(expr, source),
    ExprKind::Binary { left, right, .. } => {
      validate_ast_spans(left, source);
      validate_ast_spans(right, source);
    }
    ExprKind::Null
    | ExprKind::Bool { .. }
    | ExprKind::Int { .. }
    | ExprKind::Float { .. }
    | ExprKind::String { .. }
    | ExprKind::Identifier { .. } => {}
  }
}
