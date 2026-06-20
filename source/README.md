# online-dsl-forge

`online-dsl-forge` is a Rust library for parsing, canonicalizing, compiling,
and evaluating a small bounded DSL expression language in memory.

The crate exists for host applications that need a canonical runtime and
operator interface for DSL expressions without adopting a general-purpose
scripting language. It uses a handwritten parser and exposes a stable AST,
deterministic formatter, dynamic runtime registry, and CLI tooling.

## Capabilities

- Handwritten lexer and recursive-descent expression parser.
- Stable span-carrying AST with `serde` serialization.
- Deterministic canonical expression formatter.
- Compile-time validation against host-provided runtime schemas.
- Bounded in-memory evaluation with a dynamic variable, function, method, and
  operator registry.
- `online-dsl-forgectl` commands for `check`, `ast`, `fmt`, and `eval`.

The language intentionally excludes loops, assignment, imports, callbacks,
external I/O, mutation, async execution, and general-purpose scripting.

## Quick Start

Add the crate to a Rust project:

```sh
cargo add online-dsl-forge
```

Parse, format, compile, and evaluate an expression:

```rust
use std::collections::BTreeMap;

use online_dsl_forge::{
  CompileOptions, EvalLimits, MapRuntime, Value, compile_expression, evaluate,
  format_expression, parse_expression,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let ast = parse_expression("score + 1 >= 10 && name.starts_with('pi')")?;
  assert_eq!(
    format_expression(&ast),
    "score + 1 >= 10 && name.starts_with(\"pi\")"
  );

  let mut variables = BTreeMap::new();
  variables.insert("score".to_string(), Value::Int(9));
  variables.insert("name".to_string(), Value::String("piquark".to_string()));

  let runtime = MapRuntime::new(variables, online_dsl_forge::default_registry());
  let compiled = compile_expression(&ast, &runtime.schema(), CompileOptions::default())?;
  let value = evaluate(&compiled, &runtime, EvalLimits::default())?;

  assert_eq!(value, Value::Bool(true));
  Ok(())
}
```

## CLI

Format an expression from the repository root:

```sh
cargo run --manifest-path source/Cargo.toml --bin online-dsl-forgectl -- \
  fmt "Request.Path.starts_with('/admin') && user_score >= 10"
```

Evaluate an expression against JSON bindings:

```sh
cargo run --manifest-path source/Cargo.toml --bin online-dsl-forgectl -- \
  eval "score + 1 >= 10 && name.starts_with('pi')" \
  --bindings '{"score":9,"name":"piquark"}'
```

## Documentation

- [Technical specification](https://github.com/OxiBelt/online-dsl-forge/blob/main/docs/Specification.md)
- [Expression reference](https://github.com/OxiBelt/online-dsl-forge/blob/main/docs/Expression.md)
- [Contributing guide](https://github.com/OxiBelt/online-dsl-forge/blob/main/CONTRIBUTING.md)
