use std::collections::BTreeMap;

use online_dsl_forge::{
  Analyzer, BinaryOp, CapabilityMeta, CompileOptions, CostModel, DynamicRegistry, EvalLimits,
  MapRuntime, RegexFlavor, RuntimeSchema, SecurityProfile, Value, compile_expression, evaluate,
  evaluate_verified, format_expression, parse_expression,
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

#[test]
fn runtime_rejects_missing_verified_method_registry_capability() {
  let ast = parse_expression("name.starts_with(\"pi\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method("starts_with", 1);
  let compiled = compile_expression(&ast, &schema, CompileOptions::default())
    .expect("expression should compile");
  let mut variables = BTreeMap::new();
  variables.insert("name".to_string(), Value::String("piquark".to_string()));
  let runtime = MapRuntime::new(variables, DynamicRegistry::new());

  let error = evaluate(&compiled, &runtime, EvalLimits::default())
    .expect_err("missing registry method should fail closed");

  assert!(
    error
      .to_string()
      .contains("runtime registry is missing verified method starts_with")
  );
}

#[test]
fn runtime_rejects_verified_function_metadata_mismatch() {
  let ast = parse_expression("len(items)").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("items")
    .add_function_capability(CapabilityMeta::function("len", 1).with_cost(CostModel::Constant(2)));
  let compiled = compile_expression(&ast, &schema, CompileOptions::default())
    .expect("expression should compile");
  let mut variables = BTreeMap::new();
  variables.insert("items".to_string(), Value::Array(Vec::new()));
  let runtime = MapRuntime::new(variables, online_dsl_forge::default_registry());

  let error = evaluate(&compiled, &runtime, EvalLimits::default())
    .expect_err("registry metadata mismatch should fail closed");

  assert!(
    error
      .to_string()
      .contains("runtime registry metadata for verified function len")
  );
}

#[test]
fn runtime_rejects_verified_operator_metadata_mismatch() {
  let ast = parse_expression("left + right").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("left")
    .add_variable("right")
    .add_binary_operator_capability(
      CapabilityMeta::binary_operator(BinaryOp::Add).with_cost(CostModel::Constant(2)),
    );
  let compiled = compile_expression(&ast, &schema, CompileOptions::default())
    .expect("expression should compile");
  let mut registry = DynamicRegistry::new();
  registry.register_binary_operator(BinaryOp::Add, |left, right| match (left, right) {
    (Value::Int(left), Value::Int(right)) => Ok(Value::Int(left + right)),
    _ => Err(online_dsl_forge::EvalError::new(
      "test add requires ints",
      online_dsl_forge::SourceSpan::default(),
    )),
  });
  let mut variables = BTreeMap::new();
  variables.insert("left".to_string(), Value::Int(1));
  variables.insert("right".to_string(), Value::Int(2));
  let runtime = MapRuntime::new(variables, registry);

  let error = evaluate(&compiled, &runtime, EvalLimits::default())
    .expect_err("operator metadata mismatch should fail closed");

  assert!(
    error
      .to_string()
      .contains("runtime registry metadata for verified binary operator +")
  );
}

#[test]
fn evaluate_verified_accepts_analyzer_output() {
  let ast = parse_expression("name.starts_with(\"pi\")").expect("expression should parse");
  let mut variables = BTreeMap::new();
  variables.insert("name".to_string(), Value::String("piquark".to_string()));
  let runtime = MapRuntime::new(variables, online_dsl_forge::default_registry());
  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &runtime.schema())
    .expect("expression should analyze");

  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("verified expression should evaluate");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn context_aware_method_uses_precompiled_regex() {
  let ast = parse_expression("name.matches(\"^pi\")").expect("expression should parse");
  let mut variables = BTreeMap::new();
  variables.insert("name".to_string(), Value::String("piquark".to_string()));
  let runtime = MapRuntime::new(variables, regex_registry());

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &runtime.schema())
    .expect("literal regex should analyze");
  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("precompiled regex should evaluate");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn context_aware_method_fails_closed_on_missing_precompiled_regex() {
  let ast = parse_expression("name.matches(pattern)").expect("expression should parse");
  let mut variables = BTreeMap::new();
  variables.insert("name".to_string(), Value::String("piquark".to_string()));
  variables.insert("pattern".to_string(), Value::String("^pi".to_string()));
  let runtime = MapRuntime::new(variables, regex_registry());

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &runtime.schema())
    .expect("dynamic regex should analyze under generic safe profile");
  let error = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect_err("missing precompiled regex should fail closed");

  assert!(
    error
      .to_string()
      .contains("precompiled default regex is missing")
  );
}

#[test]
fn context_aware_method_uses_multiple_regex_flavors() {
  let ast = parse_expression("headers.anyEntryMatches(\"content-type\", \"token\")")
    .expect("expression should parse");
  let mut headers = BTreeMap::new();
  headers.insert(
    "CONTENT-TYPE".to_string(),
    Value::String("bearer token".to_string()),
  );
  let mut variables = BTreeMap::new();
  variables.insert("headers".to_string(), Value::Object(headers));
  let runtime = MapRuntime::new(variables, regex_registry());

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &runtime.schema())
    .expect("multi-regex capability should analyze");
  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("multi-regex method should evaluate");

  assert_eq!(value, Value::Bool(true));
}

fn regex_registry() -> DynamicRegistry {
  let mut registry = DynamicRegistry::new();
  registry.register_method_capability_with_context(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
    |context, receiver, args| match (receiver, &args[0]) {
      (Value::String(receiver), Value::String(pattern)) => context
        .precompiled_regex_is_match(RegexFlavor::Default, pattern, receiver)
        .map(Value::Bool),
      (Value::String(_), other) => Err(online_dsl_forge::EvalError::new(
        format!(
          "matches requires string argument, got {}",
          other.type_name()
        ),
        context.span(),
      )),
      (other, _) => Err(online_dsl_forge::EvalError::new(
        format!(
          "matches requires string receiver, got {}",
          other.type_name()
        ),
        context.span(),
      )),
    },
  );
  registry.register_method_capability_with_context(
    CapabilityMeta::method("anyEntryMatches", 2)
      .with_regex_arg(0, RegexFlavor::HeaderName)
      .with_regex_arg(1, RegexFlavor::Default),
    |context, receiver, args| {
      let (Value::String(key_pattern), Value::String(value_pattern)) = (&args[0], &args[1]) else {
        return Err(online_dsl_forge::EvalError::new(
          "anyEntryMatches requires string regex arguments",
          context.span(),
        ));
      };
      let Value::Object(values) = receiver else {
        return Err(online_dsl_forge::EvalError::new(
          format!(
            "anyEntryMatches requires object receiver, got {}",
            receiver.type_name()
          ),
          context.span(),
        ));
      };
      let key_regex = context.require_precompiled_regex(RegexFlavor::HeaderName, key_pattern)?;
      let value_regex = context.require_precompiled_regex(RegexFlavor::Default, value_pattern)?;
      Ok(Value::Bool(values.iter().any(|(key, value)| {
        key_regex.is_match(key)
          && matches!(value, Value::String(value) if value_regex.is_match(value))
      })))
    },
  );
  registry
}
