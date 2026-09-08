use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use online_dsl_forge::runtime::{
  RuntimeContext, RuntimeResourceLimits, evaluate_with_resource_limits,
};
use online_dsl_forge::{
  Analyzer, BinaryOp, CapabilityMeta, CompileOptions, CostModel, DynamicRegistry, EvalLimits,
  ExpressionDialect, ExpressionFunctionMode, MapRuntime, RegexFlavor, RuntimePatternSetConfig,
  RuntimePatternSets, RuntimeSchema, SecurityProfile, Value, compile_expression, evaluate,
  evaluate_verified, format_expression, oxirule_pattern_set_registry, parse_expression,
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
fn json_values_reject_unsigned_integer_precision_collisions() {
  assert_eq!(
    Value::try_from(serde_json::json!(9223372036854775807_i64)),
    Ok(Value::Int(i64::MAX))
  );
  for input in [
    "9223372036854775808",
    "18446744073709551615",
    "18446744073709551616",
    "18446744073709551617",
    "-9223372036854775809",
  ] {
    let json: serde_json::Value = serde_json::from_str(input).expect("JSON integer should parse");
    let error = Value::try_from(json).expect_err("out-of-range JSON integer must fail");
    assert!(
      error
        .to_string()
        .contains("outside the supported i64 range")
    );
  }
  assert_eq!(
    Value::try_from(serde_json::json!(-9223372036854775808_i64)),
    Ok(Value::Int(i64::MIN))
  );
  assert!(matches!(
    Value::try_from(serde_json::json!(1.5)).expect("fraction should convert"),
    Value::Float(_)
  ));
  let exponent: serde_json::Value =
    serde_json::from_str("1e20").expect("exponent JSON should parse");
  assert!(matches!(
    Value::try_from(exponent).expect("exponent should convert"),
    Value::Float(_)
  ));
  let decimal: serde_json::Value =
    serde_json::from_str("18446744073709551616.0").expect("decimal JSON should parse");
  assert!(matches!(
    Value::try_from(decimal).expect("decimal should convert"),
    Value::Float(_)
  ));

  let float = Value::Float(f64::from_bits(0x3fd5_5555_5555_5555));
  let encoded = serde_json::to_vec(&float).expect("runtime float should serialize");
  let decoded: Value = serde_json::from_slice(&encoded).expect("runtime float should deserialize");
  assert_eq!(
    decoded, float,
    "runtime float JSON must preserve exact bits"
  );
}

#[test]
fn public_json_value_conversion_enforces_a_safe_depth_ceiling() {
  let mut nested = serde_json::Value::Null;
  for _ in 0..16_384 {
    nested = serde_json::Value::Array(vec![nested]);
  }
  let json =
    serde_json::Value::Object(serde_json::Map::from_iter([("nested".to_string(), nested)]));
  let error = Value::try_from(json).expect_err("deep public conversion must be bounded");
  assert!(
    error
      .to_string()
      .contains("value graph depth limit exceeded")
  );
}

#[test]
fn public_json_value_conversion_drains_deep_work_on_rejection() {
  let mut nested = serde_json::Value::Number(serde_json::Number::from(u64::MAX));
  for _ in 0..100 {
    nested = serde_json::Value::Array(vec![nested]);
  }
  let error = Value::try_from(nested).expect_err("deep unsupported integer must fail");
  assert!(
    error
      .to_string()
      .contains("outside the supported i64 range")
  );
}

#[test]
fn runtime_rejects_values_above_the_hard_depth_ceiling_and_drains_them() {
  let mut registry = DynamicRegistry::new();
  registry.register_function("host", 0, |_| {
    let mut nested = Value::Null;
    for _ in 0..16_384 {
      nested = Value::Array(vec![nested]);
    }
    Ok(nested)
  });
  let runtime = MapRuntime::new(BTreeMap::new(), registry);
  let ast = parse_expression("host()").expect("expression should parse");
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_depth: usize::MAX,
      ..RuntimeResourceLimits::default()
    },
  )
  .expect_err("the hard depth ceiling must apply to custom limits");

  assert!(
    error
      .to_string()
      .contains("value graph depth limit exceeded")
  );
}

#[test]
fn runtime_drains_owned_values_rejected_by_the_cumulative_budget() {
  let mut registry = DynamicRegistry::new();
  registry.register_function("host", 0, |_| {
    let mut nested = Value::Null;
    for _ in 0..100 {
      nested = Value::Array(vec![nested]);
    }
    Ok(nested)
  });
  let runtime = MapRuntime::new(BTreeMap::new(), registry);
  let ast = parse_expression("host()").expect("expression should parse");
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_total_value_bytes: 0,
      ..RuntimeResourceLimits::default()
    },
  )
  .expect_err("the cumulative byte budget must reject the owned result");

  assert!(
    error
      .to_string()
      .contains("runtime cumulative value byte limit exceeded")
  );
}

#[test]
fn runtime_rejects_wide_values_before_scheduling_their_children() {
  let wide = Value::Array(vec![Value::Null; 65_536]);
  let error = MapRuntime::try_new_with_limits(
    BTreeMap::from([("wide".to_string(), wide)]),
    DynamicRegistry::new(),
    RuntimeResourceLimits {
      max_value_nodes: 1,
      max_value_items: usize::MAX,
      max_value_bytes: usize::MAX,
      ..RuntimeResourceLimits::default()
    },
  )
  .err()
  .expect("a wide graph must fail its projected node count");

  assert!(
    error
      .to_string()
      .contains("value graph node limit exceeded")
  );
}

#[test]
fn map_runtime_drop_drains_legacy_deep_values_iteratively() {
  let mut nested = Value::Null;
  for _ in 0..4_096 {
    nested = Value::Array(vec![nested]);
  }
  let runtime = MapRuntime::new(
    BTreeMap::from([("nested".to_string(), nested)]),
    DynamicRegistry::new(),
  );
  drop(runtime);
}

#[test]
fn runtime_resource_limits_are_inclusive_and_bound_graph_shape() {
  let mut variables = BTreeMap::new();
  variables.insert("value".to_string(), Value::String("four".to_string()));
  let runtime = MapRuntime::new(variables, DynamicRegistry::new());
  let ast = parse_expression("value").expect("expression should parse");
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let exact = RuntimeResourceLimits {
    max_value_depth: 1,
    max_value_nodes: 1,
    max_value_items: 0,
    max_value_bytes: 5,
    max_total_value_bytes: 5,
  };
  assert_eq!(
    evaluate_with_resource_limits(&compiled, &runtime, EvalLimits::default(), exact),
    Ok(Value::String("four".to_string()))
  );
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_bytes: 4,
      ..exact
    },
  )
  .expect_err("the next byte must fail");
  assert!(
    error
      .to_string()
      .contains("value graph byte limit exceeded")
  );
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_nodes: 0,
      ..exact
    },
  )
  .expect_err("a zero graph node limit rejects even a scalar root");
  assert!(
    error
      .to_string()
      .contains("value graph node limit exceeded")
  );

  let wide = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
  let admitted = MapRuntime::try_new_with_limits(
    BTreeMap::from([("wide".to_string(), wide.clone())]),
    DynamicRegistry::new(),
    RuntimeResourceLimits {
      max_value_depth: 2,
      max_value_nodes: 4,
      max_value_items: 3,
      max_value_bytes: 4,
      max_total_value_bytes: 0,
    },
  );
  assert!(admitted.is_ok(), "exact graph limits should succeed");
  let error = MapRuntime::try_new_with_limits(
    BTreeMap::from([("wide".to_string(), wide)]),
    DynamicRegistry::new(),
    RuntimeResourceLimits {
      max_value_items: 2,
      ..RuntimeResourceLimits::default()
    },
  )
  .err()
  .expect("the next item must fail");
  assert!(
    error
      .to_string()
      .contains("value graph item limit exceeded")
  );

  let nested = Value::Array(vec![Value::Array(vec![Value::Int(1)])]);
  let error = MapRuntime::try_new_with_limits(
    BTreeMap::from([("nested".to_string(), nested)]),
    DynamicRegistry::new(),
    RuntimeResourceLimits {
      max_value_depth: 2,
      ..RuntimeResourceLimits::default()
    },
  )
  .err()
  .expect("the next depth must fail");
  assert!(
    error
      .to_string()
      .contains("value graph depth limit exceeded")
  );

  let error = MapRuntime::from_json_bindings_with_limits(
    serde_json::json!({ "binding": "five" }),
    RuntimeResourceLimits {
      max_value_bytes: 4,
      ..RuntimeResourceLimits::default()
    },
  )
  .err()
  .expect("JSON bindings must use the same admission limits");
  assert!(
    error
      .to_string()
      .contains("value graph byte limit exceeded")
  );

  let exact_json = RuntimeResourceLimits {
    max_value_depth: 2,
    max_value_nodes: 2,
    max_value_items: 1,
    max_value_bytes: 7,
    max_total_value_bytes: 0,
  };
  assert!(
    MapRuntime::from_json_bindings_with_limits(serde_json::json!({ "x": "four" }), exact_json,)
      .is_ok(),
    "JSON conversion must accept an exactly bounded graph"
  );
  let error = MapRuntime::from_json_bindings_with_limits(
    serde_json::json!({ "x": "four" }),
    RuntimeResourceLimits {
      max_value_bytes: 6,
      ..exact_json
    },
  )
  .err()
  .expect("the next JSON graph byte must fail during conversion");
  assert!(
    error
      .to_string()
      .contains("value graph byte limit exceeded")
  );
}

#[test]
fn json_bindings_conversion_rejects_deep_graphs_iteratively() {
  let mut nested = serde_json::Value::Null;
  for _ in 0..256 {
    nested = serde_json::Value::Array(vec![nested]);
  }
  let bindings =
    serde_json::Value::Object(serde_json::Map::from_iter([("nested".to_string(), nested)]));
  let error = MapRuntime::from_json_bindings_with_limits(
    bindings,
    RuntimeResourceLimits {
      max_value_depth: 8,
      ..RuntimeResourceLimits::default()
    },
  )
  .err()
  .expect("deep manually constructed JSON must fail before conversion recurses");
  assert!(
    error
      .to_string()
      .contains("value graph depth limit exceeded")
  );
}

#[test]
fn runtime_resource_limits_meter_repeated_legacy_context_access() {
  struct LegacyContext {
    value: Value,
    registry: DynamicRegistry,
  }
  impl RuntimeContext for LegacyContext {
    fn get_variable(&self, name: &str) -> Option<Value> {
      (name == "value").then(|| self.value.clone())
    }

    fn registry(&self) -> &DynamicRegistry {
      &self.registry
    }
  }

  let context = LegacyContext {
    value: Value::String("four".to_string()),
    registry: DynamicRegistry::new(),
  };
  let ast = parse_expression("value == value").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("value");
  let compiled = compile_expression(&ast, &schema, CompileOptions::default())
    .expect("expression should compile");
  let error = evaluate_with_resource_limits(
    &compiled,
    &context,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_depth: 1,
      max_value_nodes: 1,
      max_value_items: 0,
      max_value_bytes: 5,
      max_total_value_bytes: 9,
    },
  )
  .expect_err("two accesses must consume the cumulative budget");
  assert!(
    error
      .to_string()
      .contains("runtime cumulative value byte limit exceeded")
  );
}

#[test]
fn runtime_resource_limits_reject_handler_results_before_reuse() {
  let mut registry = DynamicRegistry::new();
  registry.register_function("host", 0, |_| Ok(Value::String("oversized".to_string())));
  let runtime = MapRuntime::new(BTreeMap::new(), registry);
  let ast = parse_expression("host()").expect("expression should parse");
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_bytes: 9,
      ..RuntimeResourceLimits::default()
    },
  )
  .expect_err("handler output must be admitted before it escapes");
  assert!(
    error
      .to_string()
      .contains("value graph byte limit exceeded")
  );
}

#[test]
fn runtime_resource_limits_meter_object_member_clones_before_equality() {
  let mut object = BTreeMap::new();
  object.insert("name".to_string(), Value::String("four".to_string()));
  let runtime = MapRuntime::new(
    BTreeMap::from([("item".to_string(), Value::Object(object))]),
    DynamicRegistry::new(),
  );
  let ast = parse_expression("item.name == item.name").expect("expression should parse");
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .expect("expression should compile");
  let error = evaluate_with_resource_limits(
    &compiled,
    &runtime,
    EvalLimits::default(),
    RuntimeResourceLimits {
      max_value_depth: 2,
      max_value_nodes: 2,
      max_value_items: 1,
      max_value_bytes: 10,
      max_total_value_bytes: 29,
    },
  )
  .expect_err("member clones and both equality operands must be metered");
  assert!(
    error
      .to_string()
      .contains("runtime cumulative value byte limit exceeded")
  );
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
fn expression_function_call_frame_evaluates_arguments_once() {
  let ast = parse_expression("same(next())").expect("expression should parse");
  let same = parse_expression("value == value").expect("function should parse");
  let counter = Arc::new(Mutex::new(0_i64));
  let counter_for_handler = Arc::clone(&counter);
  let mut registry = DynamicRegistry::new();
  registry.register_function("next", 0, move |_| {
    let mut value = counter_for_handler
      .lock()
      .expect("counter mutex should not be poisoned");
    *value += 1;
    Ok(Value::Int(*value))
  });
  let runtime = MapRuntime::new(BTreeMap::new(), registry);
  let mut schema = runtime.schema();
  schema.add_expression_function("same", ["value"], same);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .with_expression_function_mode(ExpressionFunctionMode::CallFrame)
    .analyze(&ast, &schema)
    .expect("call-frame expression function should analyze");
  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("call-frame expression function should evaluate");

  assert_eq!(value, Value::Bool(true));
  assert_eq!(
    *counter
      .lock()
      .expect("counter mutex should not be poisoned"),
    1
  );
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

#[test]
fn oxirule_pattern_set_registry_evaluates_contains_any() {
  let pattern_sets = RuntimePatternSets::compile([RuntimePatternSetConfig::contains(
    "blocked-paths",
    ["/admin", "/blocked"],
  )])
  .expect("pattern set should compile");

  let matched = evaluate_oxirule_path(
    "/admin/settings",
    "Request.Http.Path.containsAny('blocked-paths')",
    pattern_sets.clone(),
  )
  .expect("containsAny should evaluate");
  let missed = evaluate_oxirule_path(
    "/public",
    "Request.Http.Path.containsAny('blocked-paths')",
    pattern_sets,
  )
  .expect("containsAny should evaluate");

  assert_eq!(matched, Value::Bool(true));
  assert_eq!(missed, Value::Bool(false));
}

#[test]
fn oxirule_pattern_set_registry_evaluates_matches_any() {
  let pattern_sets = RuntimePatternSets::compile([RuntimePatternSetConfig::regex(
    "admin-paths",
    [r"^/admin(/|$)"],
  )])
  .expect("regex pattern set should compile");

  let value = evaluate_oxirule_path(
    "/admin/settings",
    "Request.Http.Path.matchesAny('admin-paths')",
    pattern_sets,
  )
  .expect("matchesAny should evaluate");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn oxirule_pattern_set_registry_fails_closed_for_unknown_set() {
  let pattern_sets =
    RuntimePatternSets::compile(Vec::new()).expect("empty pattern sets should compile");
  let error = evaluate_oxirule_path(
    "/admin",
    "Request.Http.Path.containsAny('missing')",
    pattern_sets,
  )
  .expect_err("unknown pattern set should fail closed");

  assert!(
    error
      .to_string()
      .contains("unknown runtime pattern set missing")
  );
}

#[test]
fn runtime_pattern_sets_reject_invalid_regex() {
  let error = RuntimePatternSets::compile([RuntimePatternSetConfig::regex("bad", ["["])])
    .expect_err("invalid regex pattern should fail to compile");

  assert!(
    error
      .to_string()
      .contains("runtime pattern set bad contains invalid regex pattern")
  );
}

#[test]
fn pattern_set_methods_match_string_array_receivers() {
  let pattern_sets = RuntimePatternSets::compile([RuntimePatternSetConfig::contains(
    "blocked-values",
    ["secret"],
  )])
  .expect("pattern set should compile");
  let registry = oxirule_pattern_set_registry(pattern_sets);
  let mut variables = BTreeMap::new();
  variables.insert(
    "items".to_string(),
    Value::Array(vec![
      Value::String("public".to_string()),
      Value::String("secret-token".to_string()),
    ]),
  );
  let runtime = MapRuntime::new(variables, registry);
  let ast =
    parse_expression("items.containsAny('blocked-values')").expect("expression should parse");
  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &runtime.schema())
    .expect("array receiver should analyze");

  let value =
    evaluate_verified(&verified, &runtime, EvalLimits::default()).expect("array should evaluate");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn pattern_set_methods_reject_non_string_array_items() {
  let pattern_sets = RuntimePatternSets::compile([RuntimePatternSetConfig::contains(
    "blocked-values",
    ["secret"],
  )])
  .expect("pattern set should compile");
  let registry = oxirule_pattern_set_registry(pattern_sets);
  let mut variables = BTreeMap::new();
  variables.insert(
    "items".to_string(),
    Value::Array(vec![
      Value::String("secret-token".to_string()),
      Value::Int(7),
    ]),
  );
  let runtime = MapRuntime::new(variables, registry);
  let ast =
    parse_expression("items.containsAny('blocked-values')").expect("expression should parse");
  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &runtime.schema())
    .expect("array receiver should analyze");

  let error = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect_err("non-string array item should fail closed");

  assert!(
    error
      .to_string()
      .contains("pattern-set methods require string array items")
  );
}

fn evaluate_oxirule_path(
  path: &str,
  expression: &str,
  pattern_sets: RuntimePatternSets,
) -> Result<Value, online_dsl_forge::EvalError> {
  let ast = parse_expression(expression).expect("expression should parse");
  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &RuntimeSchema::oxirule_waf())
    .expect("OxiRule expression should analyze");
  let runtime = request_path_runtime(path, oxirule_pattern_set_registry(pattern_sets));
  evaluate_verified(&verified, &runtime, EvalLimits::default())
}

fn request_path_runtime(path: &str, registry: DynamicRegistry) -> MapRuntime {
  let mut http = BTreeMap::new();
  http.insert("Path".to_string(), Value::String(path.to_string()));
  let mut request = BTreeMap::new();
  request.insert("Http".to_string(), Value::Object(http));
  let mut variables = BTreeMap::new();
  variables.insert("Request".to_string(), Value::Object(request));
  MapRuntime::new(variables, registry)
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
