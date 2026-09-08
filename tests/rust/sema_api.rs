use std::collections::BTreeMap;

use online_dsl_forge::sema::{
  Analyzer, BodyAccess, BodyNeedSummary, BodyTarget, CapabilityKind, CapabilityMeta,
  CapabilityTicket, CostModel, ExpressionDialect, ExpressionFunctionLimits, ExpressionFunctionMode,
  ExpressionFunctionScope, Phase, RegexAdmissionLimits, RegexFlavor, RegexPolicy, RuntimeSchema,
  SecurityProfile, VerifiedExprKindRef,
};
use online_dsl_forge::{AstExpression, ExprKind, SourceSpan, UnaryOp, parse_expression};
use online_dsl_forge::{EvalLimits, MapRuntime, Value, default_registry, evaluate_verified};

#[test]
fn sema_compiles_to_verified_program() {
  let ast = parse_expression("score + 1 >= 10").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("score");

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  assert_eq!(verified.ast(), &ast);
  assert_eq!(verified.body_need().request, BodyAccess::None);
  assert!(
    verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(CapabilityKind::BinaryOp, "+", 2))
  );
  assert!(
    verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(CapabilityKind::BinaryOp, ">=", 2))
  );
}

#[test]
fn required_capabilities_are_exact_and_deduplicated() {
  let ast = parse_expression("[len(items), len(items), name.starts_with(\"pi\")]")
    .expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("items")
    .add_variable("name")
    .add_function("len", 1)
    .add_method("starts_with", 1);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  let expected = [
    CapabilityTicket::new(CapabilityKind::Function, "len", 1),
    CapabilityTicket::new(CapabilityKind::Method, "starts_with", 1),
  ]
  .into_iter()
  .collect();
  assert_eq!(verified.required_capabilities(), &expected);
  assert_eq!(verified.required_capability_metadata().len(), 2);
}

#[test]
fn expression_functions_inline_verified_ir_without_helper_ticket() {
  let ast = parse_expression("is_small(items)").expect("expression should parse");
  let function = parse_expression("len(items) < 3").expect("function should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("items")
    .add_function("len", 1)
    .add_expression_function("is_small", ["items"], function);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  assert!(matches!(
    verified.root().kind(),
    VerifiedExprKindRef::Binary { .. }
  ));
  assert!(
    !verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(
        CapabilityKind::Function,
        "is_small",
        1
      ))
  );
  assert!(
    verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(CapabilityKind::Function, "len", 1))
  );
}

#[test]
fn capability_phase_metadata_rejects_unavailable_calls() {
  let ast =
    parse_expression("[response_only(), name.response_only()]").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("name")
    .add_function_capability(
      CapabilityMeta::function("response_only", 0).with_phases([Phase::Response]),
    )
    .add_method_capability(
      CapabilityMeta::method("response_only", 0).with_phases([Phase::Response]),
    );

  let error = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect_err("request profile should reject response-only capabilities");
  let message = error.to_string();

  assert!(message.contains("function response_only is unavailable in Request phase"));
  assert!(message.contains("method response_only is unavailable in Request phase"));
}

#[test]
fn capability_body_access_metadata_drives_body_need() {
  let ast = parse_expression("Request.Body.inspect()").expect("expression should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_method_capability(
    CapabilityMeta::method("inspect", 0).with_body_access(BodyAccess::PrefixBytes),
  );

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
}

#[test]
fn capability_cost_metadata_contributes_to_static_cost_limit() {
  let ast = parse_expression("expensive()").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_function_capability(
    CapabilityMeta::function("expensive", 0).with_cost(CostModel::Constant(10)),
  );
  let mut profile = SecurityProfile::generic_safe();
  profile.max_cost_units = 2;

  let error = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect_err("expensive capability should exceed static cost limit");

  assert!(error.to_string().contains("static cost limit exceeded"));
}

#[test]
fn deterministic_profile_rejects_unsafe_capability_metadata() {
  let ast = parse_expression("[random(), mutate()]").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_function_capability(CapabilityMeta::function("random", 0).with_deterministic(false))
    .add_function_capability(CapabilityMeta::function("mutate", 0).with_side_effect_free(false));

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("generic safe profile should reject unsafe capability metadata");
  let message = error.to_string();

  assert!(message.contains("function random is non-deterministic"));
  assert!(message.contains("function mutate has side effects"));
}

#[test]
fn generic_transform_allows_larger_static_cost_than_generic_safe() {
  let ast = parse_expression("expensive()").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_function_capability(
    CapabilityMeta::function("expensive", 0).with_cost(CostModel::Constant(150_000)),
  );

  let safe_error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("generic safe budget should reject expensive expressions");
  assert!(
    safe_error
      .to_string()
      .contains("static cost limit exceeded")
  );

  Analyzer::new(SecurityProfile::generic_transform())
    .analyze(&ast, &schema)
    .expect("generic transform budget should allow larger expressions");
}

#[test]
fn operator_capabilities_are_verified_and_snapshot() {
  let ast = parse_expression("!enabled || score + 1 >= 10").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("enabled").add_variable("score");

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  for (name, kind, arity) in [
    ("!", CapabilityKind::UnaryOp, 1),
    ("||", CapabilityKind::BinaryOp, 2),
    ("+", CapabilityKind::BinaryOp, 2),
    (">=", CapabilityKind::BinaryOp, 2),
  ] {
    let ticket = CapabilityTicket::new(kind, name, arity);
    assert!(verified.required_capabilities().contains(&ticket));
    assert_eq!(
      verified.required_capability_metadata()[&ticket].ticket(),
      ticket
    );
  }
}

#[test]
fn waf_request_rejects_response_access() {
  let ast = parse_expression("Response.Status == 200").expect("expression should parse");
  let error = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &RuntimeSchema::waf())
    .expect_err("request profile should reject Response");

  assert!(
    error
      .to_string()
      .contains("Response is unavailable in request phase")
  );
}

#[test]
fn waf_stream_rejects_request_body_access() {
  let ast = parse_expression("Request.Body.Size > 0").expect("expression should parse");
  let error = Analyzer::new(SecurityProfile::waf_stream())
    .analyze(&ast, &RuntimeSchema::waf())
    .expect_err("stream profile should reject Request.Body");

  assert!(
    error
      .to_string()
      .contains("Request.Body is unavailable in stream phase")
  );
}

#[test]
fn expression_functions_propagate_body_need() {
  let ast = parse_expression("has_secret(Request.Body)").expect("expression should parse");
  let function = parse_expression("body.Text.contains(\"secret\")").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("has_secret", ["body"], function);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("expression should analyze");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
  assert_eq!(verified.body_need().response, BodyAccess::None);
}

#[test]
fn strict_regex_policy_requires_literal_regex() {
  let ast =
    parse_expression("Request.Body.Text.matches(pattern)").expect("expression should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_variable("pattern");

  let error = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect_err("dynamic regex should fail");

  assert!(
    error
      .to_string()
      .contains("regex argument must be a string literal")
  );
}

#[test]
fn strict_regex_policy_precompiles_literal_regex() {
  let ast = parse_expression("Request.Body.Text.matches(\"secret|token\")")
    .expect("expression should parse");
  let schema = RuntimeSchema::waf();

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect("literal regex should compile");

  assert_eq!(verified.regex_literals().len(), 1);
  assert_eq!(verified.regex_cache().len(), 1);
}

#[test]
fn regex_forbid_policy_rejects_declared_regex_arguments() {
  let ast = parse_expression("name.matches(\"^pi\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method_capability(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
  );
  let mut profile = SecurityProfile::generic_safe();
  profile.default_regex_policy = RegexPolicy::Forbid;

  let error = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect_err("forbidden regex should fail");

  assert!(
    error
      .to_string()
      .contains("regex arguments are forbidden by profile")
  );
}

#[test]
fn generic_safe_allows_dynamic_regex_by_default() {
  let ast = parse_expression("name.matches(pattern)").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("name")
    .add_variable("pattern")
    .add_method_capability(
      CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
    );

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("generic safe should preserve dynamic regex compatibility");

  assert!(verified.regex_literals().is_empty());
  assert!(verified.regex_cache().is_empty());
}

#[test]
fn generic_safe_can_opt_into_literal_only_regex() {
  let dynamic = parse_expression("name.matches(pattern)").expect("expression should parse");
  let literal = parse_expression("name.matches(\"^pi\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("name")
    .add_variable("pattern")
    .add_method_capability(
      CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
    );
  let profile =
    SecurityProfile::generic_safe().with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled);

  let error = Analyzer::new(profile.clone())
    .analyze(&dynamic, &schema)
    .expect_err("literal-only regex policy should reject dynamic patterns");
  assert!(
    error
      .to_string()
      .contains("regex argument must be a string literal")
  );

  let verified = Analyzer::new(profile)
    .analyze(&literal, &schema)
    .expect("literal regex should precompile");
  assert_eq!(verified.regex_literals().len(), 1);
  assert_eq!(verified.regex_cache().len(), 1);
}

#[test]
fn regex_admission_limits_unique_patterns_and_total_source_bytes() {
  let expression =
    parse_expression("[name.matches('a'), name.matches('b')]").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method_capability(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
  );
  let profile =
    SecurityProfile::generic_safe().with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled);
  let error = Analyzer::new(profile.clone())
    .with_regex_admission_limits(RegexAdmissionLimits {
      max_unique_regexes: 1,
      ..RegexAdmissionLimits::default()
    })
    .analyze(&expression, &schema)
    .expect_err("the second unique regex must exceed admission");
  assert!(
    error
      .to_string()
      .contains("unique regex literal limit exceeded")
  );

  let one = parse_expression("name.matches('aa')").expect("expression should parse");
  let error = Analyzer::new(profile)
    .with_regex_admission_limits(RegexAdmissionLimits {
      max_total_regex_source_bytes: 1,
      ..RegexAdmissionLimits::default()
    })
    .analyze(&one, &schema)
    .expect_err("regex source bytes must be charged before compilation");
  assert!(
    error
      .to_string()
      .contains("total regex source byte limit exceeded")
  );
}

#[test]
fn regex_builder_compiled_size_limit_fails_closed() {
  let expression = parse_expression("name.matches('a{1000}')").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method_capability(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
  );
  let profile =
    SecurityProfile::generic_safe().with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled);
  let error = Analyzer::new(profile)
    .with_regex_admission_limits(RegexAdmissionLimits {
      max_compiled_regex_bytes: 1,
      ..RegexAdmissionLimits::default()
    })
    .analyze(&expression, &schema)
    .expect_err("the compiled-regex size limit must reject the pattern");
  assert!(error.to_string().contains("invalid regex pattern"));
}

#[test]
fn inline_function_preserves_literal_regex_argument_substitution() {
  let root = parse_expression("matches_name(\"^pi\")").expect("root should parse");
  let body = parse_expression("name.matches(pattern)").expect("body should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("name")
    .add_method_capability(
      CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
    )
    .add_expression_function("matches_name", ["pattern"], body);
  let profile =
    SecurityProfile::generic_safe().with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled);
  let verified = Analyzer::new(profile)
    .analyze(&root, &schema)
    .expect("substituted literal regex should precompile");
  assert_eq!(verified.regex_literals().len(), 1);
  assert_eq!(verified.regex_cache().len(), 1);
}

#[test]
fn generic_safe_can_deny_host_declared_body_access() {
  let ast =
    parse_expression("event.payload.contains(\"secret\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("event")
    .add_body_path(
      ["event", "payload"],
      BodyTarget::Request,
      BodyAccess::PrefixBytes,
    )
    .add_method("contains", 1);
  let profile = SecurityProfile::generic_safe().deny_body_access();

  let error = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect_err("body access should exceed the denied generic profile limit");

  assert!(
    error
      .to_string()
      .contains("body access limit exceeded by profile")
  );
}

#[test]
fn generic_safe_can_deny_root_host_declared_body_access() {
  let ast = parse_expression("payload.contains(\"secret\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("payload")
    .add_body_path(["payload"], BodyTarget::Request, BodyAccess::PrefixBytes)
    .add_method("contains", 1);
  let profile = SecurityProfile::generic_safe().deny_body_access();

  let error = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect_err("root body access should exceed the denied generic profile limit");

  assert!(
    error
      .to_string()
      .contains("body access limit exceeded by profile")
  );
}

#[test]
fn generic_safe_reports_root_host_declared_body_need() {
  let ast = parse_expression("payload.contains(\"secret\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("payload")
    .add_body_path(["payload"], BodyTarget::Request, BodyAccess::PrefixBytes)
    .add_method("contains", 1);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("root body access should analyze when the profile allows it");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
}

#[test]
fn generic_safe_can_allow_host_declared_body_access_after_denying_it() {
  let ast =
    parse_expression("event.payload.contains(\"secret\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("event")
    .add_body_path(
      ["event", "payload"],
      BodyTarget::Request,
      BodyAccess::PrefixBytes,
    )
    .add_method("contains", 1);
  let profile = SecurityProfile::generic_safe()
    .deny_body_access()
    .allow_body_access();

  let verified = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect("body access should be allowed after clearing the limit");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
}

#[test]
fn generic_safe_can_limit_body_access_by_target() {
  let ast = parse_expression("event.payload_size > 0").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("event").add_body_path(
    ["event", "payload_size"],
    BodyTarget::Request,
    BodyAccess::SizeOnly,
  );
  let profile = SecurityProfile::generic_safe().with_body_access_limit(Some(BodyNeedSummary {
    request: BodyAccess::SizeOnly,
    response: BodyAccess::None,
    stream: BodyAccess::None,
  }));

  let verified = Analyzer::new(profile)
    .analyze(&ast, &schema)
    .expect("size-only body access should fit the profile limit");

  assert_eq!(verified.body_need().request, BodyAccess::SizeOnly);
}

#[test]
fn generic_safe_preserves_size_only_body_member_access() {
  let ast = parse_expression("Request.Body.Size > 0").expect("expression should parse");
  let profile = SecurityProfile::generic_safe().with_body_access_limit(Some(BodyNeedSummary {
    request: BodyAccess::SizeOnly,
    response: BodyAccess::None,
    stream: BodyAccess::None,
  }));

  let verified = Analyzer::new(profile)
    .analyze(&ast, &RuntimeSchema::waf())
    .expect("size-only body member access should fit the profile limit");

  assert_eq!(verified.body_need().request, BodyAccess::SizeOnly);
}

#[test]
fn generic_profile_compiles_and_evaluates_non_waf_host_data() {
  let ast =
    parse_expression("items.contains(\"pi\") && user.name.starts_with(\"pi\") && len(items) == 2")
      .expect("expression should parse");
  let mut user = BTreeMap::new();
  user.insert("name".to_string(), Value::String("piquark".to_string()));
  let mut variables = BTreeMap::new();
  variables.insert(
    "items".to_string(),
    Value::Array(vec![
      Value::String("pi".to_string()),
      Value::String("tau".to_string()),
    ]),
  );
  variables.insert("user".to_string(), Value::Object(user));
  let runtime = MapRuntime::new(variables, default_registry());
  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &runtime.schema())
    .expect("generic profile should analyze non-WAF host data");

  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("generic profile expression should evaluate");

  assert_eq!(value, Value::Bool(true));
}

#[test]
fn invalid_literal_regex_fails_during_analysis() {
  let ast = parse_expression("Request.Body.Text.matches(\"[\")").expect("expression should parse");
  let schema = RuntimeSchema::waf();

  let error = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect_err("invalid regex should fail");

  assert!(error.to_string().contains("invalid regex pattern"));
}

#[test]
fn duplicate_regex_literals_share_one_compiled_cache_entry() {
  let ast = parse_expression("name.matches(\"^pi\") || name.matches(\"^pi\")")
    .expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method_capability(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
  );

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect("duplicate literals should analyze");

  assert_eq!(verified.regex_literals().len(), 2);
  assert_eq!(verified.regex_cache().len(), 1);
}

#[test]
fn header_name_regex_flavor_precompiles_case_insensitive_regex() {
  let ast =
    parse_expression("headers.anyNameMatches(\"content-type\")").expect("expression should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_variable("headers");

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect("header-name regex should analyze");

  assert_eq!(
    verified
      .regex_cache()
      .is_match(RegexFlavor::HeaderName, "content-type", "CONTENT-TYPE"),
    Some(true)
  );
}

#[test]
fn custom_regex_capability_uses_declared_regex_argument() {
  let ast = parse_expression("name.matches(\"^pi\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("name").add_method_capability(
    CapabilityMeta::method("matches", 1).with_regex_arg(0, RegexFlavor::Default),
  );

  let verified = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect("declared regex capability should analyze");

  assert_eq!(verified.regex_literals()[0].pattern, "^pi");
}

#[test]
fn local_expression_function_overrides_global_function() {
  let ast = parse_expression("has_secret(Request.Body)").expect("expression should parse");
  let global = parse_expression("body.Size > 0").expect("function should parse");
  let local = parse_expression("body.Text.contains(\"secret\")").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("has_secret", ["body"], global);
  schema.add_local_expression_function("has_secret", ["body"], local);

  let route_verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("local function should analyze");
  let global_verified = Analyzer::new(SecurityProfile::generic_safe())
    .with_expression_function_scope(ExpressionFunctionScope::Global)
    .analyze(&ast, &schema)
    .expect("global function should analyze");

  assert_eq!(route_verified.body_need().request, BodyAccess::PrefixBytes);
  assert_eq!(global_verified.body_need().request, BodyAccess::SizeOnly);
}

#[test]
fn global_function_body_does_not_see_local_override() {
  let ast = parse_expression("outer(Request.Body)").expect("expression should parse");
  let global_inner = parse_expression("body.Size > 0").expect("function should parse");
  let global_outer = parse_expression("inner(body)").expect("function should parse");
  let local_inner =
    parse_expression("body.Text.contains(\"secret\")").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("inner", ["body"], global_inner);
  schema.add_expression_function("outer", ["body"], global_outer);
  schema.add_local_expression_function("inner", ["body"], local_inner);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("global function should analyze against global callees");

  assert_eq!(verified.body_need().request, BodyAccess::SizeOnly);
}

#[test]
fn local_function_body_uses_local_override() {
  let ast = parse_expression("outer(Request.Body)").expect("expression should parse");
  let global_inner = parse_expression("body.Size > 0").expect("function should parse");
  let local_inner =
    parse_expression("body.Text.contains(\"secret\")").expect("function should parse");
  let local_outer = parse_expression("inner(body)").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("inner", ["body"], global_inner);
  schema.add_local_expression_function("inner", ["body"], local_inner);
  schema.add_local_expression_function("outer", ["body"], local_outer);

  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("local function should analyze against local callees");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
}

#[test]
fn mitigation_rejects_body_object_without_content_member() {
  let ast = parse_expression("Request.Body").expect("expression should parse");
  let error = Analyzer::new(SecurityProfile::mitigation_field(Phase::Request))
    .analyze(&ast, &RuntimeSchema::waf())
    .expect_err("mitigation should reject body object access");

  assert!(
    error
      .to_string()
      .contains("MitigationField cannot read request, response, or stream body bytes")
  );
}

#[test]
fn mitigation_rejects_body_object_passed_through_function() {
  let ast = parse_expression("identity(Request.Body)").expect("expression should parse");
  let identity = parse_expression("body").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("identity", ["body"], identity);

  let error = Analyzer::new(SecurityProfile::mitigation_field(Phase::Request))
    .analyze(&ast, &schema)
    .expect_err("mitigation should reject function-mediated body access");

  assert!(
    error
      .to_string()
      .contains("MitigationField cannot read request, response, or stream body bytes")
  );
}

#[test]
fn generic_safe_denies_waf_body_object_boundaries() {
  for expression in ["Request.Body", "Response.Body"] {
    let ast = parse_expression(expression).expect("expression should parse");
    let error = Analyzer::new(SecurityProfile::generic_safe().deny_body_access())
      .analyze(&ast, &RuntimeSchema::waf())
      .expect_err("body object access should exceed the denied generic profile limit");

    assert!(
      error
        .to_string()
        .contains("body access limit exceeded by profile")
    );
  }
}

#[test]
fn generic_safe_denies_body_object_passed_through_expression_function() {
  let ast = parse_expression("identity(Request.Body)").expect("expression should parse");
  let identity = parse_expression("body").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("identity", ["body"], identity);

  let error = Analyzer::new(SecurityProfile::generic_safe().deny_body_access())
    .analyze(&ast, &schema)
    .expect_err("function-mediated body object access should exceed the denied limit");

  assert!(
    error
      .to_string()
      .contains("body access limit exceeded by profile")
  );
}

#[test]
fn generic_safe_denies_body_object_passed_to_host_calls() {
  let function_ast = parse_expression("inspect(Request.Body)").expect("expression should parse");
  let method_ast = parse_expression("Request.Body.inspect()").expect("expression should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_function("inspect", 1).add_method("inspect", 0);

  for ast in [&function_ast, &method_ast] {
    let error = Analyzer::new(SecurityProfile::generic_safe().deny_body_access())
      .analyze(ast, &schema)
      .expect_err("host-call body object access should exceed the denied generic profile limit");

    assert!(
      error
        .to_string()
        .contains("body access limit exceeded by profile")
    );
  }
}

#[test]
fn mitigation_rejects_host_declared_payload_body_path() {
  let ast =
    parse_expression("Context.Body.Text.contains(\"secret\")").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("Context")
    .add_body_path(
      ["Context", "Body", "Text"],
      BodyTarget::Request,
      BodyAccess::PrefixBytes,
    )
    .add_method("contains", 1);

  let error = Analyzer::new(SecurityProfile::mitigation_field(Phase::Request))
    .analyze(&ast, &schema)
    .expect_err("mitigation should reject host-declared payload body access");

  assert!(
    error
      .to_string()
      .contains("MitigationField cannot read request, response, or stream body bytes")
  );
}

#[test]
fn mitigation_allows_host_declared_size_only_body_path() {
  let ast = parse_expression("Context.Body.Size > 0").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("Context").add_body_path(
    ["Context", "Body", "Size"],
    BodyTarget::Request,
    BodyAccess::SizeOnly,
  );

  let verified = Analyzer::new(SecurityProfile::mitigation_field(Phase::Request))
    .analyze(&ast, &schema)
    .expect("mitigation should allow host-declared size-only body access");

  assert_eq!(verified.body_need().request, BodyAccess::SizeOnly);
}

#[test]
fn stream_payload_need_is_tracked_separately() {
  let ast =
    parse_expression("Stream.Payload.Text.contains(\"secret\")").expect("expression should parse");
  let verified = Analyzer::new(SecurityProfile::waf_stream())
    .analyze(&ast, &RuntimeSchema::waf())
    .expect("stream payload access should analyze");

  assert_eq!(verified.body_need().request, BodyAccess::None);
  assert_eq!(verified.body_need().response, BodyAccess::None);
  assert_eq!(verified.body_need().stream, BodyAccess::PrefixBytes);
}

#[test]
fn expression_function_phase_validation_rejects_response_in_request() {
  let ast = parse_expression("uses_response()").expect("expression should parse");
  let function = parse_expression("Response.Status == 200").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("uses_response", std::iter::empty::<&str>(), function);

  let error = Analyzer::new(SecurityProfile::waf_request())
    .analyze(&ast, &schema)
    .expect_err("request profile should reject function body Response access");

  assert!(
    error
      .to_string()
      .contains("Response is unavailable in request phase")
  );
}

#[test]
fn expression_function_params_are_validated() {
  let ast = parse_expression("true").expect("expression should parse");
  let function = parse_expression("body").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("bad", ["body", "body"], function);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("duplicate parameters should be rejected");

  assert!(
    error
      .to_string()
      .contains("function bad contains duplicate parameter body")
  );
}

#[test]
fn expression_function_schema_limits_apply_before_retention() {
  let limits = ExpressionFunctionLimits {
    max_functions: 1,
    ..ExpressionFunctionLimits::default()
  };
  let mut schema = RuntimeSchema::new().with_expression_function_limits(limits);
  schema.add_expression_function(
    "first",
    std::iter::empty::<&str>(),
    parse_expression("true").expect("body should parse"),
  );
  schema.add_expression_function(
    "second",
    std::iter::empty::<&str>(),
    parse_expression("true").expect("body should parse"),
  );
  assert_eq!(schema.expression_functions().count(), 1);

  let root = parse_expression("true").expect("root should parse");
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&root, &schema)
    .expect_err("the rejected function must leave a schema diagnostic");
  assert!(
    error
      .to_string()
      .contains("expression function count limit exceeded")
  );
}

#[test]
fn expression_function_scalar_limit_precedes_name_scanning() {
  let span = SourceSpan::new(0, 4);
  let mut schema = RuntimeSchema::new().with_expression_function_limits(ExpressionFunctionLimits {
    max_total_body_scalar_bytes: 3,
    ..ExpressionFunctionLimits::default()
  });
  schema.add_expression_function(
    "bounded",
    std::iter::empty::<&str>(),
    AstExpression::new(
      ExprKind::Identifier {
        name: "four".to_string(),
      },
      span,
    ),
  );
  assert_eq!(schema.expression_functions().count(), 0);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(
      &parse_expression("true").expect("root should parse"),
      &schema,
    )
    .expect_err("the body scalar limit must reject before retention");
  assert!(
    error
      .to_string()
      .contains("expression function body scalar byte limit exceeded")
  );
}

#[test]
fn expression_function_parameters_are_admitted_before_conversion() {
  use std::cell::Cell;

  struct CountedParam<'a>(&'a Cell<usize>);

  impl From<CountedParam<'_>> for String {
    fn from(value: CountedParam<'_>) -> Self {
      value.0.set(value.0.get() + 1);
      "parameter".to_string()
    }
  }

  let converted = Cell::new(0);
  let params = std::iter::repeat_with(|| CountedParam(&converted)).take(64);
  let mut schema = RuntimeSchema::new().with_expression_function_limits(ExpressionFunctionLimits {
    max_total_parameters: 1,
    ..ExpressionFunctionLimits::default()
  });
  schema.add_expression_function(
    "bounded",
    params,
    parse_expression("true").expect("body should parse"),
  );

  assert_eq!(converted.get(), 1);
  assert_eq!(schema.expression_functions().count(), 0);
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(
      &parse_expression("true").expect("root should parse"),
      &schema,
    )
    .expect_err("the second parameter must exceed admission before conversion");
  assert!(
    error
      .to_string()
      .contains("expression function parameter limit exceeded")
  );
}

#[test]
fn expression_function_diagnostics_are_bounded_before_retention_and_cloning() {
  let long_name = "f".repeat(4096);
  let limits = ExpressionFunctionLimits {
    max_total_name_bytes: 8192,
    ..ExpressionFunctionLimits::default()
  };
  let mut schema = RuntimeSchema::new().with_expression_function_limits(limits);
  for _ in 0..2 {
    schema.add_expression_function(
      long_name.clone(),
      std::iter::empty::<&str>(),
      parse_expression("true").expect("body should parse"),
    );
  }
  let diagnostic = &schema.expression_function_diagnostics()[0];
  assert!(diagnostic.message.len() <= 1024);
  assert!(diagnostic.diagnostic().message.len() <= 1024);

  let limits = ExpressionFunctionLimits {
    max_total_name_bytes: 8192,
    max_total_diagnostic_bytes: 64,
    ..ExpressionFunctionLimits::default()
  };
  let mut schema = RuntimeSchema::new().with_expression_function_limits(limits);
  for _ in 0..2 {
    schema.add_expression_function(
      long_name.clone(),
      std::iter::empty::<&str>(),
      parse_expression("true").expect("body should parse"),
    );
  }
  let retained_bytes = schema
    .expression_function_diagnostics()
    .iter()
    .map(|diagnostic| diagnostic.message.len())
    .sum::<usize>();
  assert!(retained_bytes <= 64);
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(
      &parse_expression("true").expect("root should parse"),
      &schema,
    )
    .expect_err("suppressed diagnostics must still fail closed");
  assert!(
    error
      .to_string()
      .contains("expression function diagnostic limit exceeded")
  );
}

#[test]
fn sema_rejects_invalid_names_in_public_asts_and_function_bodies() {
  let span = SourceSpan::new(0, 1);
  let root = AstExpression::new(
    ExprKind::FunctionCall {
      name: "false || privileged".to_string(),
      args: Vec::new(),
    },
    span,
  );
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&root, &RuntimeSchema::new())
    .expect_err("arbitrary root AST names must be validated before analysis");
  assert!(
    error
      .to_string()
      .contains("function name must follow identifier syntax")
  );

  for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
    let root = AstExpression::new(ExprKind::Float { value }, span);
    let error = Analyzer::new(SecurityProfile::generic_safe())
      .analyze(&root, &RuntimeSchema::new())
      .expect_err("non-finite public AST floats must fail before analysis");
    assert!(error.to_string().contains("float value must be finite"));
  }

  let mut schema = RuntimeSchema::new();
  schema.add_expression_function(
    "helper",
    ["bad-name"],
    AstExpression::new(ExprKind::Bool { value: true }, span),
  );
  assert_eq!(schema.expression_functions().count(), 0);
  let valid_root = parse_expression("true").expect("root should parse");
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&valid_root, &schema)
    .expect_err("invalid function parameters must leave a schema diagnostic");
  assert!(error.to_string().contains("must be a valid identifier"));
}

#[test]
fn expression_function_graph_rejects_recursion() {
  let ast = parse_expression("true").expect("expression should parse");
  let first = parse_expression("second()").expect("function should parse");
  let second = parse_expression("first()").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("first", std::iter::empty::<&str>(), first);
  schema.add_expression_function("second", std::iter::empty::<&str>(), second);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("recursive expression functions should be rejected");

  assert!(
    error
      .to_string()
      .contains("recursive expression function first")
      || error
        .to_string()
        .contains("recursive expression function second")
  );
}

#[test]
fn expression_function_graph_rejects_unknown_calls() {
  let ast = parse_expression("true").expect("expression should parse");
  let function = parse_expression("missing()").expect("function should parse");
  let mut schema = RuntimeSchema::waf();
  schema.add_expression_function("bad", std::iter::empty::<&str>(), function);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("unknown function calls in expression functions should be rejected");

  assert!(error.to_string().contains("unknown function missing"));
}

#[test]
fn oxirule_dialect_rejects_syntax_outside_oxirule_v1() {
  let ast = parse_expression("[1.5, -score, score * 2]").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_variable("score");

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &schema)
    .expect_err("OxiRule dialect should reject broader generic syntax");
  let message = error.to_string();

  assert!(message.contains("OxiRule V1 does not support array literals"));
  assert!(message.contains("OxiRule V1 does not support float literals"));
  assert!(message.contains("OxiRule V1 does not support unary numeric negation"));
  assert!(message.contains("OxiRule V1 does not support operator *"));
}

#[test]
fn oxirule_dialect_rejects_broad_syntax_in_expression_functions() {
  let ast = parse_expression("helper(score)").expect("expression should parse");
  let function = parse_expression("[score]").expect("function should parse");
  let mut schema = RuntimeSchema::new();
  schema
    .add_variable("score")
    .add_expression_function("helper", ["score"], function);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &schema)
    .expect_err("OxiRule dialect should validate function bodies");

  assert!(
    error
      .to_string()
      .contains("OxiRule V1 does not support array literals")
  );
}

#[test]
fn oxirule_compat_profile_allows_dynamic_regex_arguments() {
  let ast =
    parse_expression("Request.Http.Path.matches(pattern)").expect("expression should parse");
  let mut schema = RuntimeSchema::oxirule_waf();
  schema.add_variable("pattern");

  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &schema)
    .expect("compat profile should preserve current dynamic regex behavior");

  assert!(verified.regex_literals().is_empty());
  assert!(verified.regex_cache().is_empty());
}

#[test]
fn oxirule_schema_accepts_contains_any_pattern_set_method() {
  let ast = parse_expression("Request.Http.Path.containsAny('blocked-paths')")
    .expect("expression should parse");

  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &RuntimeSchema::oxirule_waf())
    .expect("OxiRule pattern-set method should analyze");

  assert!(
    verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(
        CapabilityKind::Method,
        "containsAny",
        1
      ))
  );
  assert_eq!(verified.body_need().request, BodyAccess::None);
}

#[test]
fn oxirule_schema_rejects_stale_pattern_sets_contains_call() {
  let ast = parse_expression("PatternSets.contains('blocked-paths', Request.Http.Path)")
    .expect("expression should parse");

  let error = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &RuntimeSchema::oxirule_waf())
    .expect_err("stale PatternSets helper should be rejected");
  let message = error.to_string();

  assert!(message.contains("unknown variable PatternSets"));
  assert!(message.contains("method contains does not accept 2 arguments"));
}

#[test]
fn oxirule_body_methods_preserve_body_access_metadata() {
  for expression in [
    "Request.Http.Body.contains(\"secret\")",
    "Request.Http.Body.matches(\"secret\")",
  ] {
    let ast = parse_expression(expression).expect("expression should parse");

    let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
      .with_dialect(ExpressionDialect::OxiRuleV1)
      .analyze(&ast, &RuntimeSchema::oxirule_waf())
      .expect("OxiRule body methods should analyze");

    assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
  }
}

#[test]
fn oxirule_schema_tracks_http_body_aliases_and_camel_case_methods() {
  let ast = parse_expression(
    "Request.Http.Body.Text.contains(\"secret\") && Request.Http.Path.startsWith(\"/admin\")",
  )
  .expect("expression should parse");

  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .analyze(&ast, &RuntimeSchema::oxirule_waf())
    .expect("OxiRule schema should accept OxiBelt method names");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
  assert!(
    verified
      .required_capabilities()
      .contains(&CapabilityTicket::new(
        CapabilityKind::Method,
        "startsWith",
        1
      ))
  );
}

#[test]
fn oxirule_call_frame_functions_preserve_returned_body_origin() {
  let ast = parse_expression("identity(Request.Http.Body).Text.contains(\"secret\")")
    .expect("expression should parse");
  let function = parse_expression("body").expect("function should parse");
  let mut schema = RuntimeSchema::oxirule_waf();
  schema.add_expression_function("identity", ["body"], function);

  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .with_expression_function_mode(ExpressionFunctionMode::CallFrame)
    .analyze(&ast, &schema)
    .expect("call-frame mode should preserve body origin for outer member access");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
}

#[test]
fn oxirule_call_frame_functions_propagate_bound_body_origin() {
  let ast = parse_expression("has_secret(Request.Http.Body)").expect("expression should parse");
  let function = parse_expression("body.Text.contains(\"secret\")").expect("function should parse");
  let mut schema = RuntimeSchema::oxirule_waf();
  schema.add_expression_function("has_secret", ["body"], function);

  let verified = Analyzer::new(SecurityProfile::oxirule_waf_request())
    .with_dialect(ExpressionDialect::OxiRuleV1)
    .with_expression_function_mode(ExpressionFunctionMode::CallFrame)
    .analyze(&ast, &schema)
    .expect("call-frame mode should analyze OxiRule function calls");

  assert_eq!(verified.body_need().request, BodyAccess::PrefixBytes);
  assert!(matches!(
    verified.root().kind(),
    VerifiedExprKindRef::ExpressionFunctionCall { .. }
  ));
}

fn deep_manual_ast(nodes: usize) -> AstExpression {
  let mut expression = AstExpression::new(ExprKind::Bool { value: true }, SourceSpan::new(0, 1));
  for _ in 1..nodes {
    expression = AstExpression::new(
      ExprKind::Unary {
        op: UnaryOp::Not,
        expr: Box::new(expression),
      },
      SourceSpan::new(0, 1),
    );
  }
  expression
}

#[test]
fn analyzer_preflights_manual_root_and_unreachable_function_bodies() {
  let deep_root = deep_manual_ast(131);
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&deep_root, &RuntimeSchema::new())
    .expect_err("manual over-depth root should fail before recursive analysis");
  assert!(
    error
      .to_string()
      .contains("semantic call depth limit exceeded")
  );

  let root = parse_expression("true").expect("root should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_expression_function(
    "unreachable",
    std::iter::empty::<&str>(),
    deep_manual_ast(131),
  );
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&root, &schema)
    .expect_err("unreachable over-depth function body should be rejected");
  assert!(
    error
      .to_string()
      .contains("semantic call depth limit exceeded")
  );
}

#[test]
fn schema_drains_rejected_deep_function_bodies_iteratively() {
  let mut schema = RuntimeSchema::new().with_expression_function_limits(ExpressionFunctionLimits {
    max_body_depth: usize::MAX,
    ..ExpressionFunctionLimits::default()
  });
  schema.add_expression_function(
    "too_deep",
    std::iter::empty::<&str>(),
    deep_manual_ast(16_384),
  );
  assert_eq!(schema.expression_functions().count(), 0);

  let root = parse_expression("true").expect("root should parse");
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&root, &schema)
    .expect_err("rejected deep function must retain a bounded diagnostic");
  assert!(
    error
      .to_string()
      .contains("semantic call depth limit exceeded")
  );
}

#[test]
fn schema_drains_rejected_wide_function_bodies_without_a_second_wide_buffer() {
  let span = SourceSpan::new(0, 1);
  let items = (0..65_536)
    .map(|_| AstExpression::new(ExprKind::Bool { value: true }, span))
    .collect();
  let wide = AstExpression::new(ExprKind::Array { items }, span);
  let mut schema = RuntimeSchema::new().with_expression_function_limits(ExpressionFunctionLimits {
    max_total_body_nodes: 1,
    ..ExpressionFunctionLimits::default()
  });
  schema.add_expression_function("too_wide", std::iter::empty::<&str>(), wide);
  assert_eq!(schema.expression_functions().count(), 0);

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(
      &parse_expression("true").expect("root should parse"),
      &schema,
    )
    .expect_err("rejected wide function must retain a bounded diagnostic");
  assert!(error.to_string().contains("AST node limit exceeded"));
}

#[test]
fn analyzer_depth_limit_accepts_exact_boundary_and_rejects_next_node() {
  let mut profile = SecurityProfile::generic_safe();
  profile.max_call_depth = 2;
  Analyzer::new(profile.clone())
    .analyze(&deep_manual_ast(3), &RuntimeSchema::new())
    .expect("root depth zero plus two child edges should fit exactly");
  let error = Analyzer::new(profile)
    .analyze(&deep_manual_ast(4), &RuntimeSchema::new())
    .expect_err("third child edge should exceed the semantic depth limit");
  assert!(
    error
      .to_string()
      .contains("semantic call depth limit exceeded")
  );
}

#[test]
fn analyzer_stops_at_depth_limit_in_long_acyclic_function_chain() {
  let mut schema = RuntimeSchema::new();
  schema.add_expression_function(
    "chain0",
    std::iter::empty::<&str>(),
    parse_expression("true").expect("leaf should parse"),
  );
  for level in 1..=512 {
    let body =
      parse_expression(&format!("chain{}()", level - 1)).expect("chain function should parse");
    schema.add_expression_function(format!("chain{level}"), std::iter::empty::<&str>(), body);
  }
  let root = parse_expression("chain512()").expect("root should parse");
  let mut profile = SecurityProfile::generic_safe();
  profile.max_call_depth = usize::MAX;
  let error = Analyzer::new(profile)
    .analyze(&root, &schema)
    .expect_err("large custom depth must retain the stack-safe hard cap");
  assert!(
    error
      .to_string()
      .contains("semantic call depth limit exceeded")
  );
}

fn doubling_schema(levels: usize) -> RuntimeSchema {
  let mut schema = RuntimeSchema::new();
  schema.add_expression_function(
    "double0",
    ["value"],
    parse_expression("value + value").expect("base function should parse"),
  );
  for level in 1..=levels {
    let previous = level - 1;
    let body = parse_expression(&format!(
      "double{previous}(value) + double{previous}(value)"
    ))
    .expect("doubling function should parse");
    schema.add_expression_function(format!("double{level}"), ["value"], body);
  }
  schema
}

#[test]
fn runtime_schema_rejects_forged_capability_identity() {
  let mut schema = RuntimeSchema::new();
  schema.add_function("safe", 0);
  let mut encoded = serde_json::to_value(schema).expect("schema should serialize");
  let functions = encoded
    .get_mut("functions")
    .and_then(serde_json::Value::as_object_mut)
    .expect("functions should serialize as an object");
  let safe = functions
    .remove("safe")
    .expect("safe capability should be present");
  functions.insert("privileged".to_string(), safe);

  let error = serde_json::from_value::<RuntimeSchema>(encoded)
    .expect_err("an outer dispatch name may not differ from embedded metadata");
  assert!(
    error
      .to_string()
      .contains("runtime schema function identity mismatch")
  );
}

#[test]
fn analyzer_rejects_programmatic_capability_kind_mismatch() {
  let ast = parse_expression("privileged()").expect("expression should parse");
  let mut schema = RuntimeSchema::new();
  schema.add_function_capability(CapabilityMeta::method("privileged", 0));

  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("programmatic schemas must receive the same integrity check");
  assert!(
    error
      .to_string()
      .contains("runtime schema function identity mismatch")
  );
}

#[test]
fn deserialized_schema_uses_out_of_band_expression_function_limits() {
  let schema = RuntimeSchema::new().with_expression_function_limits(ExpressionFunctionLimits {
    max_functions: usize::MAX,
    ..ExpressionFunctionLimits::default()
  });
  let mut encoded = serde_json::to_value(schema).expect("schema should serialize");
  assert!(
    encoded.get("expression_function_limits").is_none(),
    "host policy must not be serialized in-band"
  );
  encoded
    .as_object_mut()
    .expect("schema should serialize as an object")
    .insert(
      "expression_function_limits".to_string(),
      serde_json::json!({ "max_functions": usize::MAX }),
    );

  let decoded: RuntimeSchema =
    serde_json::from_value(encoded).expect("legacy in-band limits should be ignored");
  assert_eq!(
    decoded.expression_function_limits(),
    ExpressionFunctionLimits::default()
  );
}

#[test]
fn inline_function_expansion_is_metered_before_exponential_allocation() {
  let schema = doubling_schema(13);
  let ast = parse_expression("double13(1)").expect("root should parse");
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("expanded function should exceed the profile node budget");
  assert!(error.to_string().contains("AST node limit exceeded"));
}

#[test]
fn inline_function_expansion_meters_cloned_scalar_bytes_before_lowering() {
  let schema = doubling_schema(9);
  let payload = "x".repeat(128 * 1024);
  let ast = parse_expression(&format!("double9({payload:?})")).expect("root should parse");
  let error = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect_err("expanded string payloads must exceed the hard scalar-byte budget");
  assert!(
    error
      .to_string()
      .contains("lowered scalar byte limit exceeded")
  );

  let small = "x".repeat(1024);
  let ast = parse_expression(&format!("double9({small:?})")).expect("small root should parse");
  Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("a small duplicated payload should preserve inline compatibility");
}

#[test]
fn smaller_inline_expansion_retains_duplicated_argument_semantics() {
  let schema = doubling_schema(3);
  let ast = parse_expression("double3(1)").expect("root should parse");
  let verified = Analyzer::new(SecurityProfile::generic_safe())
    .analyze(&ast, &schema)
    .expect("small inline function should analyze");
  let runtime = MapRuntime::new(BTreeMap::new(), default_registry());
  let value = evaluate_verified(&verified, &runtime, EvalLimits::default())
    .expect("small inline function should evaluate");
  assert_eq!(value, Value::Int(16));
}
