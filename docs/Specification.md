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

The parser accepts float literals only when they resolve to finite `f64`
values; out-of-range decimal literals fail closed with an
`invalid float literal` diagnostic. Accepted floating-point values in
serialized ASTs round-trip through `serde_json` without changing their `f64`
bit representation. This guarantees binary floating-point fidelity, not
arbitrary-precision JSON numbers.

Parser diagnostics should include a stable message and source span. Diagnostics
must not panic on malformed input.

`parse_expression` and `lexer::tokenize` apply `ParseLimits::default()`.
`parse_expression_with_limits` and `lexer::tokenize_with_limits` let an
embedding select explicit limits. Checks occur before retaining the next token,
decoded scalar, AST node, diagnostic, or array/call item. Limits are inclusive.

| `ParseLimits` field | Default |
| --- | ---: |
| `max_source_bytes` | 1 MiB |
| `max_decoded_scalar_bytes` | 1 MiB across decoded strings and identifiers |
| `max_tokens` | 262,144, including EOF |
| `max_diagnostics` | 1,024 |
| `max_ast_nodes` | 65,536 |
| `max_collection_items` | 65,536 per array or argument list |

Parsing also enforces an internal recursion-depth budget for nested
expressions and reports a diagnostic instead of exhausting the host stack.

Parser-produced ASTs also stay below the default `serde_json` container-depth
limit so every successful parse can be serialized and deserialized with the
default configuration. Expressions whose AST would exceed that budget fail
closed with an `AST depth limit exceeded` diagnostic. This parser guarantee
does not constrain AST values constructed directly through the public Rust
types. Semantic analysis performs an iterative structural preflight over the
root and every registered expression-function body before recursive work. It
enforces the selected profile's node limit and caps recursive semantic
traversal at 128 levels even if a custom profile asks for more.

## Canonical Formatting

Canonical formatting emits a deterministic normalized expression string from an
AST. Formatting must be idempotent:

```text
parse(format(parse(input))) == parse(input)
format(parse(format(parse(input)))) == format(parse(input))
```

Whitespace is normalized around binary operators and after commas. String
literals are emitted with deterministic escaping.

`format_expression_with_limits` is the fallible boundary for arbitrary public
AST values. Its defaults allow depth 127, 65,536 nodes, and 4 MiB of output;
the traversal is iterative and checks the complete shape before formatting.
`format_expression` remains an infallible compatibility convenience, applies
the same defaults, and returns an empty string if a directly constructed AST
exceeds them. Use the fallible entry point when the caller needs a diagnostic.
Both formatter entry points also validate every identifier, member, function,
and method name against the lexer grammar and reserved-word list before emitting
source, and reject non-finite floats in directly constructed ASTs. The formatter
charges scalar input against its output-byte limit before scanning name syntax,
so an oversized public AST field is rejected before linear validation work.

## Semantic Validation

Semantic analysis validates parsed AST against a host-provided runtime schema
and security profile. Validation covers:

- unknown variables unless explicitly allowed by `CompileOptions`
- optional dialect restrictions, including the OxiRule V1 compatibility dialect
- unknown functions unless explicitly retained as unresolved by `CompileOptions`
- function arity
- expression-function graph validation, including invalid parameters,
  recursion, and scoped local-over-global resolution
- unknown methods unless explicitly retained as unresolved by `CompileOptions`
- method arity when a method signature is registered
- capability phase availability
- request, response, and stream body-access inference
- WAF profile restrictions such as request-phase rejection of `Response` and
  stream-phase rejection of `Request.Body`
- mitigation-field restrictions that reject `Request.Body`, `Response.Body`,
  `Stream.Payload`, or schema-declared payload body access, including through
  expression functions
- regex admission policy and literal regex precompilation for strict profiles
- aggregate limits on unique regexes and regex source bytes, plus a per-regex
  compiled-automaton size limit
- optional profile-level body-access limits
- static AST node, call-depth, and cost limits

`ExpressionDialect::OxiRuleV1` is an embedding compatibility gate for
OxiBelt-style OxiRule expressions. It accepts the shared parser output but
rejects generic syntax that OxiRule does not currently expose: array literals,
float literals, unary numeric negation, and the `-`, `*`, `/`, and `%`
operators. This keeps OxiBelt migration checks behavior-preserving without
forking the parser.

### Capability Metadata

Host functions, methods, and operator overrides are declared through capability
metadata. Metadata includes the capability kind, name, arity, optional
receiver/argument/result type classes, allowed phases, body-access need, regex
argument positions, determinism, side-effect freedom, and static cost model.

The semantic analyzer treats this metadata as part of validation, not as
advisory documentation. Security profiles can reject capabilities that are not
available in the active phase, require forbidden or dynamic regex arguments,
exceed static cost budgets, read disallowed body content, are non-deterministic,
or are not side-effect free.

The current type-class fields are compatibility metadata for host/runtime
contracts. They are preserved in verified programs and checked against runtime
registry metadata, but they are not yet a full static type checker for arbitrary
host object graphs.

Expression functions are sema-only helpers. The default analysis scope admits
local functions first and then global functions, matching route-local override
behavior in OxiBelt-like hosts. Global function bodies resolve nested calls
against global functions only; local function bodies resolve nested calls
against local functions first and then global functions.

Expression functions can be lowered in two modes. The default inline mode keeps
the historical substitution semantics while resolving parameters through
borrowed bindings instead of cloning replacement trees. A dry-run meter rejects
an acyclic expansion before lowering when it would exceed the profile's AST
node or effective depth limit, or the hard 64 MiB limit for owned scalar bytes
in the lowered program. `ExpressionFunctionMode::CallFrame` preserves a verified
expression-function call node, evaluates arguments exactly once at runtime, and
evaluates the verified body with parameter locals. This mode is intended for
OxiRule compatibility, where function arguments are evaluated once and body
origin inference still has to propagate through parameters such as
`has_secret(Request.Body)`.

Expression-function registration validates each function name, parameter list,
and body before retaining or replacing it. `ExpressionFunctionLimits` bounds
the complete schema with these defaults:

| Field | Default |
| --- | ---: |
| `max_functions` | 1,024 |
| `max_total_parameters` | 8,192 |
| `max_total_name_bytes` | 1 MiB |
| `max_total_body_nodes` | 65,536 |
| `max_total_body_scalar_bytes` | 16 MiB |
| `max_total_call_edges` | 65,536 |
| `max_body_depth` | 128 |
| `max_diagnostics` | 1,024 |
| `max_total_diagnostic_bytes` | 1 MiB |

Hosts can install explicit bounds with
`RuntimeSchema::with_expression_function_limits`. Semantic analysis revalidates
the aggregate limits and syntax so schemas received through serialization do
not bypass registration checks. Body depth remains hard-capped at 128, each
diagnostic message at 1 KiB, and retained or cloned expression-function
diagnostics at 1,024 messages and 1 MiB even when a schema requests higher
settings.

`RuntimeSchema::oxirule_waf()` and the `SecurityProfile::oxirule_waf_*`
constructors provide the behavior-preserving OxiRule WAF migration surface. The
compatibility profiles use the WAF phase/body/cost limits but keep
`RegexPolicy::DynamicWithBudget` so existing OxiBelt rules with dynamic regex
arguments continue to validate. The stricter `waf_*` profiles remain
literal-regex-only.

OxiRule pattern-set evaluation is host-supplied runtime behavior. Hosts can
register compiled pattern sets with the runtime registry helpers and use
receiver-method expressions such as `Request.Http.Path.containsAny("set")`.
`RuntimeSchema::oxirule_waf()` does not register the stale
`PatternSets.contains(name, value)` helper alias.

`RuntimePatternSetLimits::default()` admits at most 256 sets, 1,024 patterns
per set, 4 KiB per pattern, and 64 MiB of aggregate pattern source. Regex
patterns compile through `RegexBuilder` with a 256 KiB approximate compiled
size limit per pattern and a 64 MiB aggregate projection across regex patterns.
Hosts can install explicit bounds with `RuntimePatternSets::compile_with_limits`.

`SecurityProfile::generic_safe()` is the default non-WAF embedding profile. It
keeps `RegexPolicy::DynamicWithBudget` for compatibility, requires
deterministic and side-effect-free capabilities, fails closed, and uses modest
AST, call-depth, and static-cost budgets. Hosts that need stricter admission can
derive from it with `with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled)`,
`deny_body_access()`, or `with_body_access_limit(...)`.

`RegexAdmissionLimits::default()` admits at most 256 unique `(flavor, pattern)`
pairs and 1 MiB of aggregate pattern source. Each compilation uses a 256 KiB
`RegexBuilder` approximate size limit before cache retention, giving the default
cache an approximate 64 MiB compiled-size ceiling. Hosts can select explicit bounds
with `Analyzer::with_regex_admission_limits`. Literal source bytes are admitted
before semantic lowering or cache-key retention copies the pattern.

`SecurityProfile::generic_transform()` is for non-WAF transformation and
normalization workloads. It keeps the same deterministic and fail-closed
baseline as `generic_safe`, but raises AST, call-depth, and static-cost budgets
for expressions that operate over larger host-provided objects or collections.

The compatibility `compile_expression` API uses the generic safe profile and
returns a `CompiledExpression` backed by sema's verified program.

Semantic analysis does not execute user code. It returns a verified program
that can be evaluated repeatedly against compatible runtime contexts.

### Verified IR Contract

Verified programs carry the original AST, a semantically verified expression
tree, the security profile used for analysis, static body need, static cost
upper bound, admitted regex literals, precompiled regex cache, and the exact
capability metadata snapshot required by the program. The regex cache is scoped
to the verified program; host runtime bindings do not own or mutate it.

Function and method calls plus unary and binary operators carry capability
tickets in the verified tree. Expression functions are expanded during semantic
analysis in inline mode and become verified expression-function call-frame
nodes in call-frame mode; neither form becomes a runtime host capability. A
runtime context must provide registry metadata compatible with every verified
capability ticket before evaluation begins.

`CompileOptions` can retain an unknown function or method as an unresolved call
for compatibility inspection. Unresolved calls cannot execute against a later
runtime registry; callers must analyze the AST again against the concrete
schema so capability metadata, regex policy, and body access are verified.

## Runtime Evaluation

Runtime evaluation receives a compiled expression or verified program, a
`RuntimeContext`, `EvalLimits`, and a value-graph resource policy. `evaluate`
and `evaluate_verified` use `RuntimeResourceLimits::default()`;
`evaluate_with_resource_limits` and
`evaluate_verified_with_resource_limits` accept an explicit policy.

The runtime:

- evaluates sema-verified IR rather than arbitrary parser AST
- rejects runtime registries missing verified function, method, or operator
  capabilities
- rejects runtime registry metadata that does not match the verified capability
  metadata snapshot
- resolves identifiers from the context
- dispatches functions, methods, and optional operator overrides through a
  dynamic registry
- passes a `RuntimeCallContext` to context-aware function and method handlers so
  they can inspect the active security profile and use verified precompiled
  regex literals, preflight projected string results, or charge input-dependent
  handler work before performing it
- enforces step and recursion-depth limits
- validates every input, local, member copy, handler result, operator result,
  intermediate value, and returned value before further use
- applies `EvalLimits::max_string_bytes` to every string in those admitted
  graphs, checks string literals and concatenation before allocation, and
  charges built-in ASCII case conversion work before copying
- meters value depth, nodes, collection items, logical bytes, and cumulative
  logical bytes and crate-owned handler work processed by one evaluation
- fails closed on unknown names, type errors, arity errors, arithmetic
  overflow or non-finite results, division by zero, budget exhaustion, and
  missing object members
- short-circuits `&&` and `||`

Handlers registered with `register_function_with_context`,
`register_function_capability_with_context`, `register_method_with_context`, or
`register_method_capability_with_context` can call
`RuntimeCallContext::require_precompiled_regex` or
`RuntimeCallContext::precompiled_regex_is_match`. These helpers only succeed
for literals admitted and compiled during semantic analysis. If a handler
requires a precompiled regex for a dynamic pattern or for a literal that was not
part of the verified program, evaluation fails closed with an `EvalError`.
`RegexFlavor::HeaderName` regexes are compiled case-insensitively.
`precompiled_regex_is_match` charges candidate bytes multiplied by the admitted
pattern's source-byte complexity before matching.
The built-in OxiRule `containsAny` and `matchesAny` handlers project and charge
the complete workload before scanning, including every string item in an array
receiver. The projection multiplies the candidate bytes by the sum of each
pattern's source bytes plus one, so both candidate volume and pattern-set
complexity consume the cumulative work budget.

Runtime values are JSON-compatible: null, booleans, integers, finite floats,
strings, arrays, and objects. Floating arithmetic and handler results that
produce infinity or NaN fail admission instead of becoming JSON `null`.

| `RuntimeResourceLimits` field | Default |
| --- | ---: |
| `max_value_depth` | 128 |
| `max_value_nodes` | 262,144 per graph |
| `max_value_items` | 262,144 per graph |
| `max_value_bytes` | 64 MiB per graph |
| `max_total_value_bytes` | 64 MiB of values and charged handler work per evaluation |

Logical bytes include one tag byte per node plus string payloads and object
keys. `MapRuntime::from_json_bindings` applies these defaults while converting
JSON iteratively, and `MapRuntime::from_json_bindings_with_limits` accepts an
explicit policy. `MapRuntime::try_new_with_limits` validates existing `Value`
bindings before constructing a context. The effective value-depth limit is
capped at 128 even if an explicit policy requests more, and direct
`Value::try_from(serde_json::Value)` conversion uses the same secure default
graph limits. JSON tokens written in integer form outside the supported `i64`
range fail instead of converting to a lossy `f64`, including values above
`u64::MAX`; fractional and exponent-form JSON numbers remain floats.

`Value::try_into_json` applies the same default graph limits in the reverse
direction, converts iteratively, and rejects non-finite floats. The existing
`serde_json::Value::from(Value)` conversion remains an infallible compatibility
adapter and returns JSON `null` when the checked conversion rejects a directly
constructed value. Call the fallible method when rejection details matter.

`MapRuntime` implements `RuntimeContext::get_variable_borrowed`, which lets the
evaluator validate before cloning. Existing custom contexts remain compatible
through `get_variable`, but work performed inside that host method happens
before the crate can inspect its returned value. Custom contexts that expose
untrusted or large graphs should implement the borrowed method or enforce an
equivalent host-side bound. Host function and method execution itself also
remains host-owned work; returned values are checked before reuse.
Raw regex references exposed through `RuntimeCallContext` are a host integration
surface and cannot meter arbitrary host loops. Context-aware handlers should use
`precompiled_regex_is_match` or call `charge_work` before matching directly.

Compiled expressions are validation artifacts. Host applications should parse
and analyze expressions through `online-dsl-forge`; they should not treat
serialized ASTs or external data as already verified runtime input.

## Rulepack Rendering

The `rulepack_render` module provides in-memory rendering for schema version 2
rulepack manifests. It is a library boundary for policy packaging and does not
grant the expression runtime filesystem, network, callback, or external I/O
capabilities.

The render pipeline is deterministic:

- parse TOML and decode known rulepack tables with unknown fields rejected
- validate manifest metadata, variables, bindings, profiles, overrides,
  exceptions, provenance, and rule names
- resolve declared variables and binding render targets from
  `RulepackRenderOptions::variables`, falling back to declared variable
  defaults
- replace exact `{{name}}` placeholders in manifest strings as string data
- apply manifest overrides, then local overrides
- apply mode override and optional force-mode rule mutation
- append local exceptions to the manifest
- optionally pin resolved variable defaults and strip `bindings`, `profiles`,
  and `overrides` for installable output
- apply source commit and provenance overrides
- render pretty TOML

`render_rulepack_bundle` expands referenced rule and group files through a
caller-provided `FileResolver`. The built-in `MemoryFileResolver` and
`BlobFileResolver` are in-memory helpers for tests, remote bundles, database
artifacts, or host-managed file loading. The core library validates logical
paths before resolver calls: paths must be relative UTF-8 paths, must not use
absolute roots, `.` or `..`, backslashes, empty components, or control bytes,
and must end in `.oxirule.toml` for rules or `.oxirule-group.toml` for group
files.

The renderer fails closed on malformed TOML, unsupported schema versions,
unknown render variables, missing required variables or bindings, invalid
`cidr` or `rate` values, invalid provenance digests, unmatched overrides,
ambiguous action overrides, unsafe referenced paths, missing resolver content,
and exceptions that do not match an active non-stream rule.

Placeholders are single-pass: replacement text is opaque and is never scanned
again as template source. Placeholder markers in TOML keys or non-string TOML
syntax are rejected. Referenced rule and group files are parsed as TOML before
substitution. For a nested `when` field, placeholders are accepted only inside
DSL string literals; values are inserted into the AST string and emitted by the
canonical formatter, so a value cannot add an operator, call, or other syntax.
The placeholder budget counts markers after DSL string escapes are decoded.
Other nested placeholders remain TOML string data. The rendered nested file is
parsed and shape-checked again before it is returned.

All rulepack inspection and rendering compatibility entry points use
`RulepackRenderLimits::default()`. Their `*_with_limits` variants expose the
same policy to embeddings. Limits are inclusive.

| `RulepackRenderLimits` field | Default |
| --- | ---: |
| `max_manifest_bytes` | 8 MiB |
| `max_referenced_file_bytes` | 8 MiB per file |
| `max_variable_value_bytes` | 1 MiB |
| `max_total_variable_bytes` | 8 MiB |
| `max_variables` | 4,096 |
| `max_profile_assignments` | 262,144 assignments across profiles |
| `max_rulepack_files` | 4,096 rules and group files |
| `max_overrides` | 4,096 manifest and local overrides |
| `max_exceptions` | 4,096 manifest and local exceptions |
| `max_selector_work` | 16,777,216 selector comparisons |
| `max_local_option_bytes` | 16 MiB |
| `max_override_body_bytes` | 8 MiB per body |
| `max_placeholders` | 262,144 |
| `max_total_input_bytes` | 64 MiB |
| `max_total_output_bytes` | 64 MiB of output and retained render work |

Manifest and local override or exception counts share their respective limits.
The local option budget charges 512 logical bytes for each local override or
exception and 32 bytes for each string list item, in addition to string bytes.
Selector work is charged for preflight and
application, and projected action body copies and serialized output are charged
before an override copies a body into any rule. Local exception insertion,
variable pinning, mode and provenance updates, and final serialization are
projected before those option mutations grow the retained TOML graph.

The built-in memory and blob resolvers lend content to the renderer so it can
check a file before copying it. A custom `FileResolver` remains source
compatible through `resolve_file`; any allocation or external I/O performed
inside that host implementation precedes crate-owned checks. Implement
`resolve_file_borrowed` when the resolver already stores content in memory.

`render_text_with_limits` applies strict marker validation and the same
single-pass behavior. The infallible `render_text` function retains the legacy
marker behavior, applies the default limits, and returns an empty string when
admission or rendering exceeds a limit.

## CLI

`online-dsl-forgectl` provides:

- `check EXPR`: parse an expression and report success or diagnostics
- `ast EXPR`: print AST JSON
- `fmt EXPR`: print canonical expression text
- `eval EXPR --bindings JSON`: evaluate with JSON object bindings

CLI output should be deterministic and suitable for repository tests.
Expression arguments and stdin are capped at the parser's 1 MiB default before
joining or reading further. Inline binding JSON and `--bindings-file` input are
capped at 64 MiB, then converted with the default runtime graph limits.
