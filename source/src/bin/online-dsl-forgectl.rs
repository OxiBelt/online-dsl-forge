use std::borrow::Cow;
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::process::ExitCode;

use online_dsl_forge::{
  CompileOptions, EvalLimits, MapRuntime, ParseLimits, RuntimeResourceLimits, compile_expression,
  evaluate_with_resource_limits, format_expression, parse_expression_with_limits,
};

const MAX_BINDINGS_JSON_BYTES: usize = 64 * 1024 * 1024;

fn main() -> ExitCode {
  match run(env::args().skip(1).collect()) {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => {
      eprintln!("error: {error}");
      ExitCode::from(1)
    }
  }
}

fn run(args: Vec<String>) -> Result<(), String> {
  let Some(command) = args.first().map(String::as_str) else {
    return Err(usage());
  };
  let rest = &args[1..];
  match command {
    "check" => {
      let expression = expression_from_args(rest)?;
      parse_expression_with_limits(&expression, ParseLimits::default())
        .map_err(|error| error.to_string())?;
      println!("ok");
      Ok(())
    }
    "ast" => {
      let expression = expression_from_args(rest)?;
      let ast = parse_expression_with_limits(&expression, ParseLimits::default())
        .map_err(|error| error.to_string())?;
      let json = serde_json::to_string_pretty(&ast).map_err(|error| error.to_string())?;
      println!("{json}");
      Ok(())
    }
    "fmt" => {
      let expression = expression_from_args(rest)?;
      let ast = parse_expression_with_limits(&expression, ParseLimits::default())
        .map_err(|error| error.to_string())?;
      println!("{}", format_expression(&ast));
      Ok(())
    }
    "eval" => eval_command(rest),
    "help" | "--help" | "-h" => {
      println!("{}", usage());
      Ok(())
    }
    other => Err(format!("unknown command {other}\n\n{}", usage())),
  }
}

fn eval_command(args: &[String]) -> Result<(), String> {
  let mut expression_parts = Vec::new();
  let mut bindings = Cow::Borrowed("{}");
  let mut index = 0;
  while index < args.len() {
    match args[index].as_str() {
      "--bindings" => {
        index += 1;
        let value = args
          .get(index)
          .ok_or_else(|| "--bindings requires a JSON value".to_string())?;
        check_input_bytes(value, MAX_BINDINGS_JSON_BYTES, "bindings JSON")?;
        bindings = Cow::Borrowed(value);
      }
      "--bindings-file" => {
        index += 1;
        let path = args
          .get(index)
          .ok_or_else(|| "--bindings-file requires a path".to_string())?;
        let file = File::open(path).map_err(|error| error.to_string())?;
        bindings = Cow::Owned(read_utf8_limited(
          file,
          MAX_BINDINGS_JSON_BYTES,
          "bindings JSON",
        )?);
      }
      value => expression_parts.push(value),
    }
    index += 1;
  }

  let expression = if expression_parts.is_empty() {
    read_stdin(ParseLimits::default().max_source_bytes, "expression")?
  } else {
    join_limited(
      &expression_parts,
      ParseLimits::default().max_source_bytes,
      "expression",
    )?
  };
  let bindings_json =
    serde_json::from_str::<serde_json::Value>(&bindings).map_err(|error| error.to_string())?;
  let resource_limits = RuntimeResourceLimits::default();
  let runtime = MapRuntime::from_json_bindings_with_limits(bindings_json, resource_limits)
    .map_err(|error| error.to_string())?;
  let ast = parse_expression_with_limits(&expression, ParseLimits::default())
    .map_err(|error| error.to_string())?;
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .map_err(|error| error.to_string())?;
  let value =
    evaluate_with_resource_limits(&compiled, &runtime, EvalLimits::default(), resource_limits)
      .map_err(|error| error.to_string())?;
  let json = value.try_into_json().map_err(|error| error.to_string())?;
  println!(
    "{}",
    serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?
  );
  Ok(())
}

fn expression_from_args(args: &[String]) -> Result<String, String> {
  if args.is_empty() {
    read_stdin(ParseLimits::default().max_source_bytes, "expression")
  } else {
    join_limited(
      &args.iter().map(String::as_str).collect::<Vec<_>>(),
      ParseLimits::default().max_source_bytes,
      "expression",
    )
  }
}

fn join_limited(parts: &[&str], max_bytes: usize, label: &str) -> Result<String, String> {
  let bytes = parts
    .iter()
    .enumerate()
    .try_fold(0usize, |total, (index, part)| {
      total
        .checked_add(usize::from(index > 0))
        .and_then(|total| total.checked_add(part.len()))
    });
  if bytes.is_none_or(|bytes| bytes > max_bytes) {
    return Err(format!("{label} exceeds input byte limit of {max_bytes}"));
  }
  Ok(parts.join(" "))
}

fn read_stdin(max_bytes: usize, label: &str) -> Result<String, String> {
  read_utf8_limited(io::stdin().lock(), max_bytes, label)
}

fn read_utf8_limited(reader: impl Read, max_bytes: usize, label: &str) -> Result<String, String> {
  let mut input = String::new();
  reader
    .take(
      u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1),
    )
    .read_to_string(&mut input)
    .map_err(|error| error.to_string())?;
  check_input_bytes(&input, max_bytes, label)?;
  Ok(input)
}

fn check_input_bytes(value: &str, max_bytes: usize, label: &str) -> Result<(), String> {
  if value.len() > max_bytes {
    Err(format!("{label} exceeds input byte limit of {max_bytes}"))
  } else {
    Ok(())
  }
}

fn usage() -> String {
  "usage:
  online-dsl-forgectl check EXPR
  online-dsl-forgectl ast EXPR
  online-dsl-forgectl fmt EXPR
  online-dsl-forgectl eval EXPR --bindings JSON
  online-dsl-forgectl eval EXPR --bindings-file PATH"
    .to_string()
}
