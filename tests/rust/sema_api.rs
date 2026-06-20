use online_dsl_forge::parse_expression;
use online_dsl_forge::sema::{
  Analyzer, BodyAccess, CapabilityKind, CapabilityMeta, CapabilityTicket, CostModel,
  ExpressionFunctionScope, Phase, RegexFlavor, RegexPolicy, RuntimeSchema, SecurityProfile,
  VerifiedExprKindRef,
};

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
