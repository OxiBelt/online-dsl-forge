use std::collections::BTreeMap;

use online_dsl_forge::{
  CompileOptions, DynamicRegistry, EvalLimits, MapRuntime, RuntimeSchema, Value,
  compile_expression, evaluate, format_expression, parse_expression,
};

#[test]
fn public_api_parses_formats_compiles_and_evaluates() {
  let ast =
    parse_expression("score + 1 >= 10 && name.starts_with('pi')").expect("expression should parse");
  assert_eq!(
    format_expression(&ast),
    "score + 1 >= 10 && name.starts_with(\"pi\")"
  );

  let mut variables = BTreeMap::new();
  variables.insert("score".to_string(), Value::Int(9));
  variables.insert("name".to_string(), Value::String("piquark".to_string()));
  let runtime = MapRuntime::new(variables, online_dsl_forge::default_registry());
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let value = evaluate(&compiled, &runtime, EvalLimits::default()).expect("expression should eval");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn canonical_formatting_is_idempotent() {
  let ast = parse_expression(" user . name . starts_with( 'pi' ) && (score+1)>=10 ")
    .expect("expression should parse");
  let once = format_expression(&ast);
  let reparsed = parse_expression(&once).expect("canonical expression should parse");
  let twice = format_expression(&reparsed);

  assert_eq!(once, twice);
}

#[test]
fn compile_validation_reports_all_direct_unknowns() {
  let ast = parse_expression("left + right").expect("expression should parse");
  let error = compile_expression(
    &ast,
    &online_dsl_forge::RuntimeSchema::new(),
    CompileOptions::default(),
  )
  .expect_err("unknown variables should fail");
  let message = error.to_string();

  assert!(message.contains("unknown variable left"));
  assert!(message.contains("unknown variable right"));
}

#[test]
fn runtime_short_circuits_boolean_and() {
  let ast = parse_expression("false && missing").expect("expression should parse");
  let compiled = compile_expression(
    &ast,
    &RuntimeSchema::new(),
    CompileOptions {
      allow_unknown_variables: true,
      allow_unknown_functions: false,
      allow_unknown_methods: false,
    },
  )
  .expect("expression should compile");
  let runtime = MapRuntime::new(BTreeMap::new(), online_dsl_forge::default_registry());

  let value = evaluate(&compiled, &runtime, EvalLimits::default()).expect("eval should pass");

  assert_eq!(value, Value::Bool(false));
}

#[test]
fn runtime_rejects_missing_verified_registry_capability() {
  let ast = parse_expression("len(items)").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("items").add_function("len", 1);
  let compiled = compile_expression(&ast, &schema, CompileOptions::default())
    .expect("expression should compile");
  let mut variables = BTreeMap::new();
  variables.insert("items".to_string(), Value::Array(Vec::new()));
  let runtime = MapRuntime::new(variables, DynamicRegistry::new());

  let error = evaluate(&compiled, &runtime, EvalLimits::default())
    .expect_err("missing registry capability should fail closed");

  assert!(
    error
      .to_string()
      .contains("runtime registry is missing verified function len")
  );
}
