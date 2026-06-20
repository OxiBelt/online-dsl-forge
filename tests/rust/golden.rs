use online_dsl_forge::{
  Analyzer, BodyAccess, CompileOptions, EvalLimits, MapRuntime, RuntimeSchema, SecurityProfile,
  compile_expression, evaluate, format_expression, parse_expression,
};
use serde_json::{Value as JsonValue, json};

const PARSING: &str = include_str!("../golden/parsing.json");
const FORMATTING: &str = include_str!("../golden/formatting.json");
const PARSE_DIAGNOSTICS: &str = include_str!("../golden/parse-diagnostics.json");
const COMPILE_DIAGNOSTICS: &str = include_str!("../golden/compile-diagnostics.json");
const SEMA_DIAGNOSTICS: &str = include_str!("../golden/sema-diagnostics.json");
const EVALUATION: &str = include_str!("../golden/evaluation.json");
const RULEPACK_RENDER_DEFERRED: &str = include_str!("../golden/rulepack-render/deferred.json");
const BODY_NEED_DEFERRED: &str = include_str!("../golden/body-need/deferred.json");

#[test]
fn parsing_ast_json_matches_golden_fixtures() {
  for case in fixture_cases(PARSING) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let expected = required_value(&case, "expected_ast");

    let ast = parse_expression(input).unwrap_or_else(|error| {
      panic!("{id}: expected expression to parse, got {error}");
    });
    let actual = serde_json::to_value(&ast).expect("AST should serialize");

    assert_json_eq(id, &actual, expected);
  }
}

#[test]
fn formatting_matches_golden_fixtures_and_is_idempotent() {
  for case in fixture_cases(FORMATTING) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let expected = required_str(&case, "expected");

    let ast = parse_expression(input).unwrap_or_else(|error| {
      panic!("{id}: expected expression to parse, got {error}");
    });
    let actual = format_expression(&ast);
    assert_eq!(actual, expected, "{id}: formatted expression changed");

    let reparsed = parse_expression(&actual).unwrap_or_else(|error| {
      panic!("{id}: formatted expression should reparse, got {error}");
    });
    assert_eq!(
      format_expression(&reparsed),
      actual,
      "{id}: formatted expression is not idempotent"
    );
  }
}

#[test]
fn parse_diagnostics_match_golden_fixtures() {
  for case in fixture_cases(PARSE_DIAGNOSTICS) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let expected = required_value(&case, "expected");

    let error = parse_expression(input).expect_err("fixture should fail to parse");
    let actual = serde_json::to_value(&error).expect("diagnostics should serialize");

    assert_json_eq(id, &actual, expected);
  }
}

#[test]
fn compile_diagnostics_match_golden_fixtures() {
  for case in fixture_cases(COMPILE_DIAGNOSTICS) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let expected = required_value(&case, "expected");
    let ast = parse_expression(input).unwrap_or_else(|error| {
      panic!("{id}: expected expression to parse, got {error}");
    });
    let schema = runtime_schema(case.get("schema"));
    let options = compile_options(case.get("options"));

    let actual = match compile_expression(&ast, &schema, options) {
      Ok(_) => json!({ "diagnostics": [] }),
      Err(error) => serde_json::to_value(&error).expect("diagnostics should serialize"),
    };

    assert_json_eq(id, &actual, expected);
  }
}

#[test]
fn sema_diagnostics_match_golden_fixtures() {
  for case in fixture_cases(SEMA_DIAGNOSTICS) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let ast = parse_expression(input).unwrap_or_else(|error| {
      panic!("{id}: expected expression to parse, got {error}");
    });
    let schema = sema_schema(required_value(&case, "schema"));
    let profile = sema_profile(required_str(&case, "profile"));

    let actual = match Analyzer::new(profile).analyze(&ast, &schema) {
      Ok(_) => json!({ "diagnostics": [] }),
      Err(error) => serde_json::to_value(&error).expect("diagnostics should serialize"),
    };

    assert_json_eq(id, &actual, required_value(&case, "expected"));
  }
}

#[test]
fn evaluation_matches_golden_fixtures() {
  for case in fixture_cases(EVALUATION) {
    let id = case_id(&case);
    let input = required_str(&case, "input");
    let bindings = case.get("bindings").cloned().unwrap_or_else(|| json!({}));
    let runtime = MapRuntime::from_json_bindings(bindings).unwrap_or_else(|error| {
      panic!("{id}: bindings should create a runtime, got {error}");
    });
    let ast = parse_expression(input).unwrap_or_else(|error| {
      panic!("{id}: expected expression to parse, got {error}");
    });
    let compiled = compile_expression(
      &ast,
      &runtime.schema(),
      compile_options(case.get("compile_options")),
    )
    .unwrap_or_else(|error| {
      panic!("{id}: expected expression to compile, got {error}");
    });

    let actual = match evaluate(&compiled, &runtime, eval_limits(case.get("limits"))) {
      Ok(value) => {
        let value: JsonValue = value.into();
        json!({ "ok": value })
      }
      Err(error) => json!({ "error": error.to_string() }),
    };

    assert_json_eq(id, &actual, required_value(&case, "expected"));
  }
}

#[test]
fn deferred_rulepack_render_fixtures_are_well_formed() {
  for case in fixture_cases(RULEPACK_RENDER_DEFERRED) {
    let id = case_id(&case);
    let origin = required_str(&case, "origin");
    let source = required_str(&case, "source");
    let input = required_str(&case, "input");
    let expected = required_value(&case, "expected");

    assert!(
      origin.starts_with("/references/OxiBelt/"),
      "{id}: origin should point to the OxiBelt reference tree"
    );
    assert!(!source.is_empty(), "{id}: source should not be empty");
    assert!(
      input.contains("[rulepack]"),
      "{id}: deferred rulepack fixture should include a manifest"
    );

    let must_contain = required_array(expected, "must_contain");
    assert!(
      !must_contain.is_empty(),
      "{id}: rulepack fixture should lock at least one rendered invariant"
    );
    assert_all_strings(id, must_contain, "must_contain");

    if let Some(must_not_contain) = expected.get("must_not_contain") {
      assert_all_strings(
        id,
        must_not_contain
          .as_array()
          .unwrap_or_else(|| panic!("{id}: must_not_contain must be an array")),
        "must_not_contain",
      );
    }
  }
}

#[test]
fn deferred_body_need_fixtures_match_sema_analysis() {
  for case in fixture_cases(BODY_NEED_DEFERRED) {
    let id = case_id(&case);
    let origin = required_str(&case, "origin");
    let expression = required_str(&case, "expression");

    assert!(
      origin.starts_with("/references/OxiBelt/"),
      "{id}: origin should point to the OxiBelt reference tree"
    );
    let ast = parse_expression(expression).unwrap_or_else(|error| {
      panic!("{id}: body-need expression should parse, got {error}");
    });
    let mut schema = RuntimeSchema::waf();

    for function in required_array(&case, "functions") {
      add_fixture_expression_function(&mut schema, id, function);
    }

    let expected = required_value(&case, "expected");
    assert_body_need(id, required_str(expected, "request_body"));
    assert_body_need(id, required_str(expected, "response_body"));
    let expected_stream_body = expected
      .get("stream_body")
      .and_then(JsonValue::as_str)
      .unwrap_or("none");
    assert_body_need(id, expected_stream_body);

    let profile = case
      .get("profile")
      .and_then(JsonValue::as_str)
      .map(sema_profile)
      .unwrap_or_else(SecurityProfile::generic_safe);
    let verified = Analyzer::new(profile)
      .analyze(&ast, &schema)
      .unwrap_or_else(|error| panic!("{id}: body-need expression should analyze, got {error}"));
    let actual = verified.body_need();
    assert_eq!(
      actual.request,
      body_access(required_str(expected, "request_body")),
      "{id}: request body need changed"
    );
    assert_eq!(
      actual.response,
      body_access(required_str(expected, "response_body")),
      "{id}: response body need changed"
    );
    assert_eq!(
      actual.stream,
      body_access(expected_stream_body),
      "{id}: stream body need changed"
    );
  }
}

fn fixture_cases(raw: &str) -> Vec<JsonValue> {
  serde_json::from_str(raw).expect("fixture file should be valid JSON")
}

fn case_id(case: &JsonValue) -> &str {
  required_str(case, "id")
}

fn required_value<'a>(value: &'a JsonValue, field: &str) -> &'a JsonValue {
  value
    .get(field)
    .unwrap_or_else(|| panic!("fixture is missing field {field}"))
}

fn required_str<'a>(value: &'a JsonValue, field: &str) -> &'a str {
  required_value(value, field)
    .as_str()
    .unwrap_or_else(|| panic!("fixture field {field} must be a string"))
}

fn required_array<'a>(value: &'a JsonValue, field: &str) -> &'a Vec<JsonValue> {
  required_value(value, field)
    .as_array()
    .unwrap_or_else(|| panic!("fixture field {field} must be an array"))
}

fn assert_all_strings(id: &str, values: &[JsonValue], field: &str) {
  for value in values {
    let item = value
      .as_str()
      .unwrap_or_else(|| panic!("{id}: {field} entries must be strings"));
    assert!(!item.is_empty(), "{id}: {field} entries must not be empty");
  }
}

fn assert_body_need(id: &str, need: &str) {
  assert!(
    matches!(need, "none" | "size_only" | "prefix_bytes"),
    "{id}: invalid body need {need}"
  );
}

fn runtime_schema(value: Option<&JsonValue>) -> RuntimeSchema {
  let mut schema = RuntimeSchema::new();
  let Some(value) = value else {
    return schema;
  };

  if let Some(variables) = value.get("variables").and_then(JsonValue::as_array) {
    for variable in variables {
      schema.add_variable(
        variable
          .as_str()
          .expect("schema variables entries must be strings"),
      );
    }
  }

  if let Some(functions) = value.get("functions").and_then(JsonValue::as_object) {
    for (name, arities) in functions {
      for arity in arities
        .as_array()
        .expect("schema function arities must be arrays")
      {
        schema.add_function(name, json_usize(arity));
      }
    }
  }

  if let Some(methods) = value.get("methods").and_then(JsonValue::as_object) {
    for (name, arities) in methods {
      for arity in arities
        .as_array()
        .expect("schema method arities must be arrays")
      {
        schema.add_method(name, json_usize(arity));
      }
    }
  }

  schema
}

fn sema_schema(value: &JsonValue) -> RuntimeSchema {
  let mut schema = if value.get("preset").and_then(JsonValue::as_str) == Some("waf") {
    RuntimeSchema::waf()
  } else {
    RuntimeSchema::new()
  };

  if let Some(variables) = value.get("variables").and_then(JsonValue::as_array) {
    for variable in variables {
      schema.add_variable(
        variable
          .as_str()
          .expect("schema variable entries must be strings"),
      );
    }
  }

  if let Some(functions) = value
    .get("expression_functions")
    .and_then(JsonValue::as_array)
  {
    for function in functions {
      add_fixture_expression_function(&mut schema, "sema fixture", function);
    }
  }

  schema
}

fn add_fixture_expression_function(schema: &mut RuntimeSchema, id: &str, function: &JsonValue) {
  let function_name = required_str(function, "name");
  let function_expression =
    parse_expression(required_str(function, "expression")).unwrap_or_else(|error| {
      panic!("{id}: function {function_name} expression should parse, got {error}");
    });
  let params = required_array(function, "params")
    .iter()
    .map(|param| param.as_str().expect("function params must be strings"));
  match function
    .get("scope")
    .and_then(JsonValue::as_str)
    .unwrap_or("global")
  {
    "global" => {
      schema.add_expression_function(function_name, params, function_expression);
    }
    "local" => {
      schema.add_local_expression_function(function_name, params, function_expression);
    }
    other => panic!("{id}: unknown function scope {other}"),
  }
}

fn sema_profile(value: &str) -> SecurityProfile {
  match value {
    "generic_safe" => SecurityProfile::generic_safe(),
    "waf_request" => SecurityProfile::waf_request(),
    "waf_response" => SecurityProfile::waf_response(),
    "waf_stream" => SecurityProfile::waf_stream(),
    "mitigation_request" => SecurityProfile::mitigation_field(online_dsl_forge::Phase::Request),
    "mitigation_response" => SecurityProfile::mitigation_field(online_dsl_forge::Phase::Response),
    "mitigation_stream" => SecurityProfile::mitigation_field(online_dsl_forge::Phase::Stream),
    other => panic!("unknown sema profile {other}"),
  }
}

fn body_access(value: &str) -> BodyAccess {
  match value {
    "none" => BodyAccess::None,
    "size_only" => BodyAccess::SizeOnly,
    "prefix_bytes" => BodyAccess::PrefixBytes,
    other => panic!("unknown body access {other}"),
  }
}

fn compile_options(value: Option<&JsonValue>) -> CompileOptions {
  let mut options = CompileOptions::default();
  let Some(value) = value else {
    return options;
  };
  options.allow_unknown_variables = json_bool(value, "allow_unknown_variables");
  options.allow_unknown_functions = json_bool(value, "allow_unknown_functions");
  options.allow_unknown_methods = json_bool(value, "allow_unknown_methods");
  options
}

fn eval_limits(value: Option<&JsonValue>) -> EvalLimits {
  let mut limits = EvalLimits::default();
  let Some(value) = value else {
    return limits;
  };
  if let Some(max_steps) = value.get("max_steps") {
    limits.max_steps = json_usize(max_steps);
  }
  if let Some(max_depth) = value.get("max_depth") {
    limits.max_depth = json_usize(max_depth);
  }
  if let Some(max_string_bytes) = value.get("max_string_bytes") {
    limits.max_string_bytes = json_usize(max_string_bytes);
  }
  if let Some(max_array_items) = value.get("max_array_items") {
    limits.max_array_items = json_usize(max_array_items);
  }
  limits
}

fn json_bool(value: &JsonValue, field: &str) -> bool {
  value
    .get(field)
    .and_then(JsonValue::as_bool)
    .unwrap_or(false)
}

fn json_usize(value: &JsonValue) -> usize {
  value
    .as_u64()
    .and_then(|value| usize::try_from(value).ok())
    .expect("fixture value must fit in usize")
}

fn assert_json_eq(id: &str, actual: &JsonValue, expected: &JsonValue) {
  assert_eq!(
    actual,
    expected,
    "{id}: golden JSON changed\nactual:\n{}\nexpected:\n{}",
    serde_json::to_string_pretty(actual).expect("actual should serialize"),
    serde_json::to_string_pretty(expected).expect("expected should serialize")
  );
}
