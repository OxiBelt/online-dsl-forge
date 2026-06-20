# online-dsl-forge Technical Specification

Status: Draft
Target project: `online-dsl-forge` Rust DSL parser and runtime library

This document is the compact behavior specification for `online-dsl-forge`.
Expression syntax is covered in [Expression.md](Expression.md).

## Scope

`online-dsl-forge` parses, canonicalizes, validates, and evaluates bounded DSL
expressions in memory. Host applications provide runtime bindings, functions,
methods, and optional operator overrides through Rust APIs.

The implementation is optimized for:

- embedding in larger Rust applications
- deterministic canonical AST and formatting
- strict validation of untrusted input
- bounded evaluation without external I/O
- minimal dependencies
- OxiBelt-like repository layout and contribution workflow

## Language Model

The v1 language is expression-only. It supports:

- literals: `null`, booleans, integers, floats, strings, and arrays
- identifiers resolved from a host runtime context
- member access with `object.field`
- function calls with `name(arg1, arg2)`
- method calls with `receiver.method(arg1, arg2)`
- unary operators `!` and `-`
- binary arithmetic, comparison, equality, and boolean operators
- parentheses for grouping

The language intentionally does not support loops, imports, assignment,
mutation, callbacks, async execution, external I/O, or arbitrary scripting.

## Parser and AST

The parser is handwritten and recursive-descent. It produces a public AST where
every node carries a byte span into the original source text.

The public AST is serializable with `serde`. Serialized AST shape is part of the
public compatibility surface and should change only with documentation and
tests.

Parser diagnostics should include a stable message and source span. Diagnostics
must not panic on malformed input.

## Canonical Formatting

Canonical formatting emits a deterministic normalized expression string from an
AST. Formatting must be idempotent:

```text
parse(format(parse(input))) == parse(input)
format(parse(format(parse(input)))) == format(parse(input))
```

Whitespace is normalized around binary operators and after commas. String
literals are emitted with deterministic escaping.

## Compile Validation

Compilation validates parsed AST against a host-provided runtime schema.
Validation covers:

- unknown variables unless explicitly allowed by `CompileOptions`
- unknown functions
- function arity
- unknown methods unless explicitly allowed by `CompileOptions`
- method arity when a method signature is registered

Compilation does not execute user code. It returns a `CompiledExpression` that
can be evaluated repeatedly against compatible runtime contexts.

## Runtime Evaluation

Runtime evaluation receives a compiled expression, a `RuntimeContext`, and
`EvalLimits`.

The runtime:

- resolves identifiers from the context
- dispatches functions, methods, and optional operator overrides through a
  dynamic registry
- enforces step and recursion-depth limits
- fails closed on unknown names, type errors, arity errors, arithmetic
  overflow, division by zero, budget exhaustion, and missing object members
- short-circuits `&&` and `||`

Runtime values are JSON-compatible: null, booleans, integers, floats, strings,
arrays, and objects.

## CLI

`online-dsl-forgectl` provides:

- `check EXPR`: parse an expression and report success or diagnostics
- `ast EXPR`: print AST JSON
- `fmt EXPR`: print canonical expression text
- `eval EXPR --bindings JSON`: evaluate with JSON object bindings

CLI output should be deterministic and suitable for repository tests.
