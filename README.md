# online-dsl-forge

`online-dsl-forge` is a Rust library for parsing, canonicalizing, compiling,
and evaluating a small bounded DSL expression language in memory.

The project exists for host applications that need a canonical runtime and
operator interface for DSL expressions without adopting a general-purpose
scripting language. It uses a handwritten parser and exposes a stable AST,
deterministic formatter, dynamic runtime registry, and CLI tooling.

## Capabilities

- Publishable `online-dsl-forge-parser` crate with a handwritten lexer,
  recursive-descent expression parser, AST, diagnostics, spans, and formatter.
- Stable span-carrying AST with `serde` serialization.
- Deterministic canonical expression formatter.
- Compile-time validation against host-provided runtime schemas.
- Bounded in-memory evaluation with a dynamic variable, function, method, and
  operator registry.
- `online-dsl-forgectl` commands for `check`, `ast`, `fmt`, and `eval`.

The language intentionally excludes loops, assignment, imports, callbacks,
external I/O, mutation, async execution, and general-purpose scripting.

## Quick Start

From the repository root:

```sh
cargo test --all-features
```

Format an expression:

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

- [Technical specification](docs/Specification.md)
- [Expression reference](docs/Expression.md)
- [Contributing guide](CONTRIBUTING.md)

## Project Layout

```text
parser/                       Parser, AST, diagnostics, spans, and formatter crate
source/                       Umbrella Rust library and CLI crate
parser/src/                   Syntax-only parser library source
source/src/                   Compiler, runtime, values, re-exports, and CLI support
tests/rust/                   Repository-level Rust integration tests
tests/scripts/                Local and CI check scripts
docs/                         Specification and expression reference
.github/workflows/            GitHub Actions workflows
```

Root-level documentation uses root-relative paths. If a command must run from
`source/`, the command block says so explicitly.
