# online-dsl-forge Expression Reference

Status: Draft

## Literals

```text
null
true
false
123
3.14
"hello"
'hello'
[1, 2, "three"]
```

Strings support `\\`, `\"`, `\'`, `\n`, `\r`, and `\t` escapes.
Other escapes, such as `\.`, fail with an `unsupported string escape`
diagnostic. To pass a backslash to a regex, write `\\` in the DSL string;
for example, `"\\."` denotes a regex that matches a literal dot.
Float literals must resolve to finite `f64` values. Decimal literals outside
that range fail with an `invalid float literal` diagnostic.

The default parser accepts at most 1 MiB of source, 262,144 tokens, 1 MiB of
decoded scalar data, 65,536 AST nodes, and 65,536 items in any array or call.
Embeddings can choose explicit bounds with `ParseLimits` and
`parse_expression_with_limits`.

## Identifiers

Identifiers start with an ASCII letter or `_` and continue with ASCII letters,
digits, or `_`.

Reserved words cannot be used as identifiers:

```text
if else for while do switch let const function import export new try catch throw await return
true false null
```

The same rules apply to identifier, member, function, method, and expression
function parameter names in ASTs constructed through the public Rust types.
The fallible formatter and semantic analyzer reject names that the lexer could
not produce, including reserved words. They also reject non-finite `Float`
values in directly constructed ASTs. Checked formatting admits scalar bytes
before scanning identifier syntax.

## Access and Calls

```text
user.name
len(items)
name.starts_with("pi")
```

Member access requires the runtime value to be an object. Function and method
names are validated during compilation against the supplied runtime schema.
Host functions, methods, and operator overrides may also be constrained by
semantic capability metadata such as phase availability, body access, regex
policy, cost, determinism, and side-effect freedom.

Capabilities that declare regex arguments can be evaluated by context-aware
runtime handlers. Strict profiles require those regex arguments to be string
literals and precompile them during semantic analysis; runtime handlers then use
the verified precompiled cache through `RuntimeCallContext`. If a handler asks
for a precompiled regex that was not admitted during analysis, evaluation fails
closed.

`Analyzer::with_regex_admission_limits` bounds unique literal patterns, their
aggregate source bytes, and each compiled regex automaton before it is retained
in the verified program. Oversized literals are rejected before lowering or
cache-key allocation copies them.

Non-WAF hosts should start with `SecurityProfile::generic_safe()` for ordinary
filtering and decision expressions over host-provided JSON-like objects.
`SecurityProfile::generic_transform()` keeps the same deterministic baseline
but allows larger AST, call-depth, and static-cost budgets for transformation
workloads. Hosts can derive stricter profiles with
`with_regex_policy(RegexPolicy::LiteralOnlyPrecompiled)`,
`deny_body_access()`, or `with_body_access_limit(...)` when untrusted
expressions must not inspect host-declared payload fields.

```rust
let ast = online_dsl_forge::parse_expression(
  "items.contains(\"pi\") && user.name.starts_with(\"pi\")",
)?;
let profile = online_dsl_forge::SecurityProfile::generic_safe()
  .with_regex_policy(online_dsl_forge::RegexPolicy::LiteralOnlyPrecompiled)
  .deny_body_access();
let verified = online_dsl_forge::Analyzer::new(profile).analyze(&ast, &schema)?;
```

## Operators

Precedence from highest to lowest:

| Operators | Meaning |
| --- | --- |
| `()` `.` calls | grouping, member access, calls |
| `!` `-` | boolean not, numeric negation |
| `*` `/` `%` | multiplication, division, remainder |
| `+` `-` | addition, subtraction, string concatenation for `+` |
| `<` `<=` `>` `>=` | comparison |
| `==` `!=` | equality |
| `&&` | boolean and |
| `||` | boolean or |

`&&` and `||` short-circuit. Arithmetic operators fail closed on invalid types,
integer or floating-point overflow, non-finite results, division by zero, or
remainder by zero.

## Built-In CLI Runtime

The CLI evaluation command registers a small default runtime:

- functions: `len(value)`
- string methods: `contains(value)`, `starts_with(value)`, `ends_with(value)`,
  `lower_ascii()`, `upper_ascii()`, `len()`
- array methods: `len()`, `contains(value)`
- object methods: `len()`, `contains_key(value)`

Host applications may provide different registries through the Rust API.

## OxiRule Compatibility

Hosts that embed OxiBelt-style OxiRule expressions should parse with the normal
parser, then analyze with `ExpressionDialect::OxiRuleV1`,
`RuntimeSchema::oxirule_waf()`, an `oxirule_waf_*` security profile, and
`ExpressionFunctionMode::CallFrame`.

The compatibility dialect rejects generic syntax outside OxiRule V1:

- array literals
- float literals
- unary numeric negation
- `-`, `*`, `/`, and `%`

The OxiRule WAF schema exposes lowerCamelCase method names such as
`startsWith`, `lowerAscii`, `anyValueMatches`, and `directPeerIpNetworkPrefix`,
plus WAF roots such as `Context`, `Request`, `DynamicPolicy`, `Response`, and
`Stream`.

Pattern-set helpers use receiver-method syntax such as
`Request.Http.Path.containsAny("blocked-paths")` and
`Request.Http.Path.matchesAny("blocked-paths")`. The compatibility schema does
not register the stale `PatternSets.contains(name, value)` helper form.
Hosts bound pattern-set count, per-pattern and aggregate source bytes, and
approximate compiled-regex size when constructing the runtime registry.
