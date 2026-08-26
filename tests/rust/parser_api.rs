use online_dsl_forge::{
  AstExpression, DiagnosticReport, ExprKind, SourceSpan, format_expression, parse_expression,
};
use serde_json::json;

const LONG_DECIMAL_FLOAT: &str = "0.0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001";

fn assert_default_json_round_trip(input: &str) {
  let ast = parse_expression(input).expect("boundary expression should parse");
  let serialized = serde_json::to_vec(&ast).expect("AST should serialize");
  let decoded: AstExpression =
    serde_json::from_slice(&serialized).expect("serialized AST should deserialize");
  assert_eq!(ast, decoded, "AST JSON round trip should be exact");
}

#[test]
fn parser_api_round_trips_long_decimal_float_exactly() {
  let ast = parse_expression(LONG_DECIMAL_FLOAT).expect("long decimal float should parse");
  let ExprKind::Float { value } = ast.kind else {
    panic!("long decimal float should produce a float AST node");
  };
  assert_eq!(
    value.to_bits(),
    (1e-97_f64).to_bits(),
    "DSL float parsing should select the nearest f64 value"
  );
  assert_default_json_round_trip(LONG_DECIMAL_FLOAT);
}

fn assert_ast_depth_error(input: &str) {
  let error = parse_expression(input).expect_err("over-depth expression should fail");
  assert_eq!(error.diagnostics.len(), 1);
  assert_eq!(error.diagnostics[0].message, "AST depth limit exceeded");
  assert_eq!(error.diagnostics[0].span, SourceSpan::new(0, input.len()));
}

#[test]
fn parser_api_parses_formats_and_serializes_ast() {
  let ast =
    parse_expression("score + 1 >= 10 && name.starts_with('pi')").expect("expression should parse");

  assert_eq!(
    format_expression(&ast),
    "score + 1 >= 10 && name.starts_with(\"pi\")"
  );
  assert!(matches!(ast.kind, ExprKind::Binary { .. }));

  let actual = serde_json::to_value(&ast).expect("AST should serialize");
  assert_eq!(
    actual["kind"]["kind"],
    json!("binary"),
    "top-level AST JSON shape should stay stable"
  );
}

#[test]
fn parser_api_reports_diagnostics() {
  let error = parse_expression("1 +").expect_err("invalid expression should fail");
  let report: DiagnosticReport = error;

  assert_eq!(report.diagnostics.len(), 1);
  assert_eq!(report.diagnostics[0].message, "expected expression");
}

#[test]
fn parser_api_rejects_excessive_recursive_nesting() {
  let cases = [
    ("unary operators", format!("{}true", "!".repeat(300))),
    (
      "parentheses",
      format!("{}true{}", "(".repeat(300), ")".repeat(300)),
    ),
    (
      "arrays",
      format!("{}true{}", "[".repeat(300), "]".repeat(300)),
    ),
    (
      "call arguments",
      format!("{}true{}", "len(".repeat(300), ")".repeat(300)),
    ),
  ];

  for (name, input) in cases {
    let error = parse_expression(&input).expect_err(name);
    assert!(
      error
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message == "parse recursion depth limit exceeded"),
      "{name} should report parse recursion depth limit exceeded, got {error}"
    );
  }
}

#[test]
fn parser_api_bounds_serialized_ast_depth() {
  let accepted = [
    format!("{}a", "-".repeat(62)),
    vec!["a"; 63].join("-"),
    format!("a{}", ".x".repeat(62)),
    format!("{}a{}", "[".repeat(41), "]".repeat(41)),
    format!("{}a{}", "f(".repeat(41), ")".repeat(41)),
    format!("a{}", ".f()".repeat(62)),
    format!("{}a{}", "a.f(".repeat(41), ")".repeat(41)),
    format!("[{}]", vec!["a"; 512].join(",")),
    format!("f({})", vec!["a"; 512].join(",")),
  ];

  for input in accepted {
    assert_default_json_round_trip(&input);
  }

  let rejected = [
    format!("{}a", "-".repeat(63)),
    vec!["a"; 64].join("-"),
    format!("a{}", ".x".repeat(63)),
    format!("{}a{}", "[".repeat(42), "]".repeat(42)),
    format!("{}a{}", "f(".repeat(42), ")".repeat(42)),
    format!("a{}", ".f()".repeat(63)),
    format!("{}a{}", "a.f(".repeat(42), ")".repeat(42)),
    format!("{}R-ua", "-".repeat(62)),
  ];

  for input in rejected {
    assert_ast_depth_error(&input);
  }
}
