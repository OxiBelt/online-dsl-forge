use std::io::Write;
use std::process::{Command, Stdio};

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

#[test]
fn cli_rejects_oversized_stdin_before_parsing() {
  let mut child = cli()
    .arg("check")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("CLI should start");
  child
    .stdin
    .take()
    .expect("stdin should be piped")
    .write_all(&vec![b'a'; 1024 * 1024 + 1])
    .expect("oversized expression should be written");
  let output = child.wait_with_output().expect("CLI should finish");

  assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
  assert!(stderr(&output).contains("expression exceeds input byte limit of 1048576"));
}

#[test]
fn cli_rejects_unsigned_json_integers_outside_i64() {
  for integer in ["18446744073709551615", "18446744073709551616"] {
    let bindings = format!(r#"{{"identifier":{integer}}}"#);
    let output = cli()
      .args(["eval", "true", "--bindings", bindings.as_str()])
      .output()
      .expect("CLI should run");

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("outside the supported i64 range"));
  }
}

fn stderr(output: &std::process::Output) -> String {
  String::from_utf8(output.stderr.clone()).unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
}
