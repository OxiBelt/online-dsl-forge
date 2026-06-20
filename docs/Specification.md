# online-dsl-forge Technical Specification

Status: Draft
Target project: `online-dsl-forge` Rust DSL parser, semantic analyzer, and
runtime library

This document is the compact behavior specification for `online-dsl-forge`.
Expression syntax is covered in [Expression.md](Expression.md).

## Scope

The single `online-dsl-forge` crate parses and canonicalizes bounded DSL
expressions, validates parsed ASTs against runtime schemas and security
profiles, emits verified programs, and provides bounded runtime evaluation.
Host applications provide runtime bindings, functions, methods, and optional
operator overrides through Rust APIs.

The implementation is optimized for:

- embedding in larger Rust applications
- deterministic canonical AST and formatting
- strict semantic validation of untrusted input
- explicit security profiles and capability metadata
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

The parser module contains the handwritten recursive-descent parser. It
produces a public AST where every node carries a byte span into the original
source text.

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

## Semantic Validation

Semantic analysis validates parsed AST against a host-provided runtime schema
and security profile. Validation covers:

- unknown variables unless explicitly allowed by `CompileOptions`
- unknown functions
- function arity
- expression-function graph validation, including invalid parameters,
  recursion, and scoped local-over-global resolution
- unknown methods unless explicitly allowed by `CompileOptions`
- method arity when a method signature is registered
- capability phase availability
- request, response, and stream body-access inference
- WAF profile restrictions such as request-phase rejection of `Response` and
  stream-phase rejection of `Request.Body`
- mitigation-field restrictions that reject `Request.Body`, `Response.Body`,
  or `Stream.Payload` access, including through expression functions
- regex admission policy and literal regex precompilation for strict profiles
- static AST node, call-depth, and cost limits

Expression functions are sema-only helpers. The default analysis scope admits
local functions first and then global functions, matching route-local override
behavior in OxiBelt-like hosts. Global function bodies resolve nested calls
against global functions only; local function bodies resolve nested calls
against local functions first and then global functions.

The compatibility `compile_expression` API uses the generic safe profile and
returns a `CompiledExpression` backed by sema's verified program.

Semantic analysis does not execute user code. It returns a verified program
that can be evaluated repeatedly against compatible runtime contexts.

## Runtime Evaluation

Runtime evaluation receives a compiled expression or verified program, a
`RuntimeContext`, and `EvalLimits`.

The runtime:

- evaluates sema-verified IR rather than arbitrary parser AST
- rejects runtime registries missing verified function or method capabilities
- resolves identifiers from the context
- dispatches functions, methods, and optional operator overrides through a
  dynamic registry
- enforces step and recursion-depth limits
- fails closed on unknown names, type errors, arity errors, arithmetic
  overflow, division by zero, budget exhaustion, and missing object members
- short-circuits `&&` and `||`

Runtime values are JSON-compatible: null, booleans, integers, floats, strings,
arrays, and objects.

Compiled expressions are validation artifacts. Host applications should parse
and analyze expressions through `online-dsl-forge`; they should not treat
serialized ASTs or external data as already verified runtime input.

## CLI

`online-dsl-forgectl` provides:

- `check EXPR`: parse an expression and report success or diagnostics
- `ast EXPR`: print AST JSON
- `fmt EXPR`: print canonical expression text
- `eval EXPR --bindings JSON`: evaluate with JSON object bindings

CLI output should be deterministic and suitable for repository tests.
