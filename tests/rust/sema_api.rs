use online_dsl_forge_parser::parse_expression;
use online_dsl_forge_sema::{
  Analyzer, BodyAccess, CapabilityMeta, RegexFlavor, RuntimeSchema, SecurityProfile,
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
  assert!(verified.required_capabilities().is_empty());
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
