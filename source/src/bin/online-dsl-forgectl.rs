use std::env;
use std::io::{self, Read};
use std::process::ExitCode;

use online_dsl_forge::{
  CompileOptions, EvalLimits, MapRuntime, compile_expression, evaluate, format_expression,
  parse_expression,
};

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
      parse_expression(&expression).map_err(|error| error.to_string())?;
      println!("ok");
      Ok(())
    }
    "ast" => {
      let expression = expression_from_args(rest)?;
      let ast = parse_expression(&expression).map_err(|error| error.to_string())?;
      let json = serde_json::to_string_pretty(&ast).map_err(|error| error.to_string())?;
      println!("{json}");
      Ok(())
    }
    "fmt" => {
      let expression = expression_from_args(rest)?;
      let ast = parse_expression(&expression).map_err(|error| error.to_string())?;
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
  let mut bindings = "{}".to_string();
  let mut index = 0;
  while index < args.len() {
    match args[index].as_str() {
      "--bindings" => {
        index += 1;
        bindings = args
          .get(index)
          .cloned()
          .ok_or_else(|| "--bindings requires a JSON value".to_string())?;
      }
      "--bindings-file" => {
        index += 1;
        let path = args
          .get(index)
          .ok_or_else(|| "--bindings-file requires a path".to_string())?;
        bindings = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
      }
      value => expression_parts.push(value.to_string()),
    }
    index += 1;
  }

  let expression = if expression_parts.is_empty() {
    read_stdin()?
  } else {
    expression_parts.join(" ")
  };
  let bindings_json =
    serde_json::from_str::<serde_json::Value>(&bindings).map_err(|error| error.to_string())?;
  let runtime = MapRuntime::from_json_bindings(bindings_json).map_err(|error| error.to_string())?;
  let ast = parse_expression(&expression).map_err(|error| error.to_string())?;
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())
    .map_err(|error| error.to_string())?;
  let value =
    evaluate(&compiled, &runtime, EvalLimits::default()).map_err(|error| error.to_string())?;
  let json = serde_json::Value::from(value);
  println!(
    "{}",
    serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?
  );
  Ok(())
}

fn expression_from_args(args: &[String]) -> Result<String, String> {
  if args.is_empty() {
    read_stdin()
  } else {
    Ok(args.join(" "))
  }
}

fn read_stdin() -> Result<String, String> {
  let mut input = String::new();
  io::stdin()
    .read_to_string(&mut input)
    .map_err(|error| error.to_string())?;
  Ok(input)
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
