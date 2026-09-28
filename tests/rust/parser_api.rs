use online_dsl_forge::parser::{
  AstFormatLimits, ParseLimits, UnaryOp, format_expression_with_limits,
  parse_expression_with_limits,
};
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

#[test]
fn parser_api_accepts_only_finite_float_literals() {
  let finite = format!("1{}.0", "0".repeat(308));
  let ast = parse_expression(&finite).expect("1e308 should remain a finite float");
  let ExprKind::Float { value } = ast.kind else {
    panic!("finite boundary should produce a float AST node");
  };
  assert!(value.is_finite(), "accepted float literals must be finite");
  assert_eq!(value.to_bits(), (1e308_f64).to_bits());
  assert_default_json_round_trip(&finite);

  let overflow = format!("2{}.0", "0".repeat(308));
  let lex_error =
    online_dsl_forge::lexer::tokenize(&overflow).expect_err("2e308 should fail lexing");
  assert_eq!(lex_error.len(), 1);
  assert_eq!(lex_error[0].message, "invalid float literal");
  assert_eq!(lex_error[0].span, SourceSpan::new(0, overflow.len()));

  let parse_error = parse_expression(&overflow).expect_err("2e308 should fail parsing");
  assert_eq!(parse_error.diagnostics.len(), 1);
  assert_eq!(parse_error.diagnostics[0].message, "invalid float literal");
  assert_eq!(
    parse_error.diagnostics[0].span,
    SourceSpan::new(0, overflow.len())
  );
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
fn parser_api_rejects_unsupported_string_escapes_at_their_source_spans() {
  let input = r#""\.é\q""#;
  let error = parse_expression(input).expect_err("unsupported escapes must fail closed");
  assert_eq!(error.diagnostics.len(), 2);
  assert_eq!(error.diagnostics[0].message, "unsupported string escape");
  assert_eq!(error.diagnostics[0].span, SourceSpan::new(1, 3));
  assert_eq!(error.diagnostics[1].message, "unsupported string escape");
  assert_eq!(error.diagnostics[1].span, SourceSpan::new(5, 7));
}

#[test]
fn parser_api_keeps_the_six_documented_string_escapes() {
  let cases = [
    (r#""\\""#, "\\"),
    (r#""\"""#, "\""),
    (r#""\'""#, "'"),
    (r#""\n""#, "\n"),
    (r#""\r""#, "\r"),
    (r#""\t""#, "\t"),
  ];
  for (source, expected) in cases {
    let ast = parse_expression(source).expect("documented escape should parse");
    let ExprKind::String { value } = ast.kind else {
      panic!("expected string literal");
    };
    assert_eq!(value, expected);
  }
}

#[test]
fn parser_api_preserves_doubled_backslashes_in_regex_literals_and_formatting() {
  let input = r#"Request.Http.Path.matches("\\.\\.")"#;
  let ast = parse_expression(input).expect("doubled backslashes should parse");
  let ExprKind::MethodCall { args, .. } = &ast.kind else {
    panic!("expected regex method call");
  };
  let ExprKind::String { value } = &args[0].kind else {
    panic!("expected string regex argument");
  };
  assert_eq!(value, r"\.\.");
  let formatted = format_expression(&ast);
  assert_eq!(formatted, input);
  assert_eq!(
    format_expression(&parse_expression(&formatted).unwrap()),
    input
  );
}

#[test]
fn parser_api_bounds_unsupported_escape_diagnostics() {
  let input = r#""\a\b\c""#;
  let error = parse_expression_with_limits(
    input,
    ParseLimits {
      max_diagnostics: 2,
      ..ParseLimits::default()
    },
  )
  .expect_err("diagnostics should stop at the configured limit");
  assert_eq!(error.diagnostics.len(), 1);
  assert_eq!(error.diagnostics[0].message, "diagnostic limit exceeded");
  assert_eq!(error.diagnostics[0].span, SourceSpan::new(5, 7));
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

#[test]
fn parser_limits_accept_exact_boundaries_and_reject_the_next_unit() {
  let exact_source = ParseLimits {
    max_source_bytes: 4,
    ..ParseLimits::default()
  };
  parse_expression_with_limits("true", exact_source).expect("exact source limit should succeed");
  let error = parse_expression_with_limits("false", exact_source)
    .expect_err("source beyond limit should fail");
  assert_eq!(error.diagnostics[0].message, "source byte limit exceeded");

  let exact_tokens = ParseLimits {
    max_tokens: 2,
    ..ParseLimits::default()
  };
  parse_expression_with_limits("true", exact_tokens).expect("token plus EOF should fit exactly");
  let error = parse_expression_with_limits(
    "!true",
    ParseLimits {
      max_tokens: 2,
      ..ParseLimits::default()
    },
  )
  .expect_err("one token beyond limit should fail");
  assert_eq!(error.diagnostics[0].message, "token limit exceeded");

  parse_expression_with_limits(
    "abc",
    ParseLimits {
      max_decoded_scalar_bytes: 3,
      ..ParseLimits::default()
    },
  )
  .expect("decoded identifier should fit exactly");
  let error = parse_expression_with_limits(
    "abc",
    ParseLimits {
      max_decoded_scalar_bytes: 2,
      ..ParseLimits::default()
    },
  )
  .expect_err("decoded identifier beyond limit should fail");
  assert_eq!(
    error.diagnostics[0].message,
    "decoded scalar byte limit exceeded"
  );

  parse_expression_with_limits(
    "1 + 2",
    ParseLimits {
      max_ast_nodes: 3,
      ..ParseLimits::default()
    },
  )
  .expect("three AST nodes should fit exactly");
  let error = parse_expression_with_limits(
    "1 + 2",
    ParseLimits {
      max_ast_nodes: 2,
      ..ParseLimits::default()
    },
  )
  .expect_err("third AST node should fail");
  assert_eq!(error.diagnostics[0].message, "AST node limit exceeded");
}

#[test]
fn parser_limits_bound_collections_diagnostics_and_zero_configuration() {
  for input in ["[a, b]", "f(a, b)"] {
    parse_expression_with_limits(
      input,
      ParseLimits {
        max_collection_items: 2,
        ..ParseLimits::default()
      },
    )
    .expect("two collection items should fit exactly");
    let error = parse_expression_with_limits(
      input,
      ParseLimits {
        max_collection_items: 1,
        ..ParseLimits::default()
      },
    )
    .expect_err("second collection item should fail");
    assert_eq!(
      error.diagnostics[0].message,
      "collection item limit exceeded"
    );
  }

  let diagnostics = online_dsl_forge::lexer::tokenize_with_limits(
    "@@",
    ParseLimits {
      max_diagnostics: 2,
      ..ParseLimits::default()
    },
  )
  .expect_err("invalid input should report diagnostics");
  assert_eq!(diagnostics.len(), 2);
  let diagnostics = online_dsl_forge::lexer::tokenize_with_limits(
    "@@",
    ParseLimits {
      max_diagnostics: 1,
      ..ParseLimits::default()
    },
  )
  .expect_err("diagnostic overflow should fail terminally");
  assert_eq!(diagnostics.len(), 1);
  assert_eq!(diagnostics[0].message, "diagnostic limit exceeded");

  let error = parse_expression_with_limits(
    "",
    ParseLimits {
      max_source_bytes: 0,
      max_decoded_scalar_bytes: 0,
      max_tokens: 0,
      max_diagnostics: 0,
      max_ast_nodes: 0,
      max_collection_items: 0,
    },
  )
  .expect_err("zero limits should fail deterministically");
  assert_eq!(error.diagnostics[0].message, "token limit exceeded");
}

#[test]
fn bounded_formatter_is_iterative_and_checks_before_output_growth() {
  let mut deep = AstExpression::new(ExprKind::Bool { value: true }, SourceSpan::new(0, 1));
  for _ in 0..128 {
    deep = AstExpression::new(
      ExprKind::Unary {
        op: UnaryOp::Not,
        expr: Box::new(deep),
      },
      SourceSpan::new(0, 1),
    );
  }
  let error = format_expression_with_limits(&deep, AstFormatLimits::default())
    .expect_err("manual over-depth AST should be rejected without recursion");
  assert_eq!(
    error.diagnostics[0].message,
    "AST format depth limit exceeded"
  );
  assert_eq!(
    format_expression(&deep),
    "",
    "the infallible compatibility formatter must fail closed at its default limit"
  );

  let string = AstExpression::new(
    ExprKind::String {
      value: "a\n".to_string(),
    },
    SourceSpan::new(0, 1),
  );
  assert_eq!(
    format_expression_with_limits(
      &string,
      AstFormatLimits {
        max_output_bytes: 5,
        ..AstFormatLimits::default()
      }
    )
    .expect("escaped output should fit exactly"),
    "\"a\\n\""
  );
  let error = format_expression_with_limits(
    &string,
    AstFormatLimits {
      max_output_bytes: 4,
      ..AstFormatLimits::default()
    },
  )
  .expect_err("escaped output beyond limit should fail");
  assert_eq!(
    error.diagnostics[0].message,
    "AST format output byte limit exceeded"
  );
}

#[test]
fn bounded_formatter_rejects_scalar_input_before_identifier_scanning() {
  let expression = AstExpression::new(
    ExprKind::Identifier {
      name: "a".repeat(9),
    },
    SourceSpan::new(0, 9),
  );
  let error = format_expression_with_limits(
    &expression,
    AstFormatLimits {
      max_output_bytes: 8,
      ..AstFormatLimits::default()
    },
  )
  .expect_err("identifier input beyond the output budget must fail before scanning");
  assert_eq!(
    error.diagnostics[0].message,
    "AST scalar byte limit exceeded"
  );
}

#[test]
fn arbitrary_ast_formatter_rejects_invalid_syntactic_names() {
  let span = SourceSpan::new(0, 1);
  let receiver = || AstExpression::new(ExprKind::Bool { value: true }, span);
  let invalid = [
    AstExpression::new(
      ExprKind::Identifier {
        name: "false || privileged()".to_string(),
      },
      span,
    ),
    AstExpression::new(
      ExprKind::Member {
        receiver: Box::new(receiver()),
        name: "bad-name".to_string(),
      },
      span,
    ),
    AstExpression::new(
      ExprKind::FunctionCall {
        name: "true".to_string(),
        args: Vec::new(),
      },
      span,
    ),
    AstExpression::new(
      ExprKind::MethodCall {
        receiver: Box::new(receiver()),
        name: "call()".to_string(),
        args: Vec::new(),
      },
      span,
    ),
  ];

  for expression in invalid {
    let error = format_expression_with_limits(&expression, AstFormatLimits::default())
      .expect_err("invalid names must fail before formatter output");
    assert!(
      error
        .to_string()
        .contains("name must follow identifier syntax and must not be reserved")
    );
    assert_eq!(format_expression(&expression), "");
  }

  let valid = AstExpression::new(
    ExprKind::Identifier {
      name: "_safe9".to_string(),
    },
    span,
  );
  assert_eq!(format_expression(&valid), "_safe9");
}

#[test]
fn arbitrary_ast_formatter_rejects_non_finite_floats() {
  let span = SourceSpan::new(0, 1);
  for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
    let expression = AstExpression::new(ExprKind::Float { value }, span);
    let error = format_expression_with_limits(&expression, AstFormatLimits::default())
      .expect_err("non-finite public AST floats must fail before formatter output");
    assert_eq!(error.diagnostics[0].message, "float value must be finite");
    assert_eq!(format_expression(&expression), "");
  }
}
