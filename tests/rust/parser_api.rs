use online_dsl_forge::{DiagnosticReport, ExprKind, format_expression, parse_expression};
use serde_json::json;

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
