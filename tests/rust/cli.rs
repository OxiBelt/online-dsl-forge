use std::process::Command;

fn cli() -> Command {
  Command::new(env!("CARGO_BIN_EXE_online-dsl-forgectl"))
}

#[test]
fn cli_formats_expression() {
  let output = cli()
    .args(["fmt", "score+1>=10&&name.starts_with('pi')"])
    .output()
    .expect("CLI should run");

  assert!(output.status.success(), "stderr: {}", stderr(&output));
  assert_eq!(
    String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
    "score + 1 >= 10 && name.starts_with(\"pi\")\n"
  );
}

#[test]
fn cli_evaluates_json_bindings() {
  let output = cli()
    .args([
      "eval",
      "score + 1 >= 10 && name.starts_with('pi')",
      "--bindings",
      r#"{"score":9,"name":"piquark"}"#,
    ])
    .output()
    .expect("CLI should run");

  assert!(output.status.success(), "stderr: {}", stderr(&output));
  assert_eq!(
    String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
    "true\n"
  );
}

#[test]
fn cli_reports_parse_errors() {
  let output = cli()
    .args(["check", "1 +"])
    .output()
    .expect("CLI should run");

  assert!(!output.status.success());
  assert!(stderr(&output).contains("expected expression"));
}

#[test]
fn cli_rejects_excessive_parse_depth_without_aborting() {
  let expression = format!("{}true", "!".repeat(300));
  let output = cli()
    .args(["check", expression.as_str()])
    .output()
    .expect("CLI should run");

  assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
  assert!(stderr(&output).contains("parse recursion depth limit exceeded"));
}

fn stderr(output: &std::process::Output) -> String {
  String::from_utf8(output.stderr.clone()).unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
}
