# online-dsl-forge-parser

`online-dsl-forge-parser` contains the syntax-only layer for
`online-dsl-forge`: the handwritten lexer and recursive-descent parser,
span-carrying AST, diagnostics, and canonical formatter.

The crate intentionally does not compile, evaluate, or dispatch host runtime
behavior. Use `online-dsl-forge` when you need the umbrella API with compile
validation, runtime values, bounded evaluation, and CLI tooling.

## Quick Start

```rust
use online_dsl_forge_parser::{format_expression, parse_expression};

let ast = parse_expression("score + 1 >= 10 && name.starts_with('pi')")?;
assert_eq!(
  format_expression(&ast),
  "score + 1 >= 10 && name.starts_with(\"pi\")"
);
# Ok::<(), Box<dyn std::error::Error>>(())
```
