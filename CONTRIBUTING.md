# Contributing to online-dsl-forge

Thanks for helping improve `online-dsl-forge`. This project parses and
evaluates untrusted DSL input, so changes to lexing, parsing, AST
canonicalization, semantic validation, security profiles, runtime schemas,
runtime values, operator dispatch, function/method calls, and evaluation limits
must be reviewed as
security-sensitive unless there is a clear reason they are not.

Use root-relative paths in root-level documentation, scripts, issues, and pull
request notes. For example, prefer `parser/src/parser.rs` over `src/parser.rs`
unless the text explicitly says the command is being run from a crate
subdirectory.

## Repository Layout

Generated and local-only directories such as `target/`, `source/target/`,
`node_modules/`, and `tests/.tmp/` are not source contributions and should not
be committed.

| Path | Purpose | Change here when |
| --- | --- | --- |
| `parser/` | Parser Rust crate. | You are changing lexer, parser, AST, diagnostics, spans, or canonical formatting behavior. |
| `parser/src/ast.rs` | Public AST model. | Syntax shape, serde AST, spans, or canonical representation changes. |
| `parser/src/parser.rs` and `parser/src/lexer.rs` | Handwritten parser pipeline. | Tokens, grammar, precedence, diagnostics, or parse recovery change. |
| `parser/src/format.rs` | Canonical formatting. | Normalized expression output or idempotency changes. |
| `sema/` | Semantic analysis Rust crate. | You are changing runtime schemas, security profiles, capability metadata, regex policy, body-need inference, or verified IR. |
| `sema/src/analyzer.rs` | Semantic analyzer. | Validation traversal, diagnostics, phase restrictions, cost limits, regex checks, or body-access inference change. |
| `sema/src/schema.rs` and `sema/src/profile.rs` | Host schema and security profile model. | Public semantic API, capability metadata, profile defaults, or body-access types change. |
| `source/` | Umbrella Rust crate and CLI. | You are changing runtime, CLI, public re-exports, compatibility compile API, or integration behavior. |
| `source/src/compile.rs` | Compatibility re-exports for semantic validation. | Public compile API exports change. |
| `source/src/runtime.rs` and `source/src/value.rs` | Evaluation and host integration. | Values, function/method/operator registry, limits, or evaluation semantics change. |
| `source/src/bin/` | CLI tooling. | `online-dsl-forgectl` commands or command output changes. |
| `tests/rust/` | Rust integration tests and repository-level checks. | Behavior changes need regression coverage. |
| `tests/scripts/` | Check orchestration. | Local or CI test flows change. |
| `docs/` | Technical specifications and references. | User-visible syntax, APIs, compatibility, or behavior changes. |
| `.github/workflows/` | GitHub Actions workflows. | CI job structure or required checks change. |

## Contribution Workflow

1. Identify the affected area before editing: parser, AST, formatter, semantic
   analyzer, runtime, CLI, tests, CI, or documentation.
2. Make the smallest reasonable change for the behavior being changed.
3. Add or update tests when syntax, diagnostics, canonical formatting, compile
   validation, security-profile, evaluation, CLI, or public API behavior
   changes.
4. Update documentation when behavior, syntax, commands, or public APIs change.
5. Run the relevant checks and mention any checks that could not be run.
6. Verify that generated test data and temporary files are cleaned up.

When changing Rust code, prefer workspace-level commands from the repository
root:

```sh
cargo fmt --check
tests/scripts/check-tests-rustfmt.sh
tests/scripts/check-rust-module-size.sh
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

When dependency state changes, also run:

```sh
cargo audit
cargo deny check advisories
```

When changing versioning or release automation, also run:

```sh
pnpm install --frozen-lockfile
pnpm run lint
pnpm run typecheck
pnpm run test
pnpm run versioning:check
```

The committed Cargo version for `online-dsl-forge-parser`,
`online-dsl-forge-sema`, and `online-dsl-forge` must remain `0.0.0` in
`parser/Cargo.toml`, `sema/Cargo.toml`, `source/Cargo.toml`, and `Cargo.lock`.
Release CI derives the publish version from the triggering SemVer tag, rewrites
all three crates in the checkout for that workflow run, publishes
`online-dsl-forge-parser` first, then `online-dsl-forge-sema`, and then the
umbrella `online-dsl-forge` crate.

## Commit Messages

Use Conventional Commits for commit messages:

```text
<type>(<scope>): <subject>
```

- `type` must be one of `feat`, `fix`, `chore`, `docs`, `ci`, `refactor`,
  `security`, `tests`, or `perf`.
- `scope` is the area touched by the code, such as `parser`, `ast`, `runtime`,
  `cli`, `workflows`, or `docs`.
- `subject` is a short imperative summary. Use a present-tense verb. Do not use
  past tense or past-perfect wording.
- In commit titles and detailed descriptions, wrap code keywords, paths,
  commands, configuration keys, function names, variable names, type names,
  module names, and literal values in Markdown inline code spans.

Valid examples:

```text
feat(parser): add `??` null-coalescing syntax
fix(runtime): reject integer division by zero
security(compile): fail closed on unknown methods
ci(workflows): run advisory checks
```

## Rust Module Organization

Do not force unrelated functionality into an existing Rust source file just
because the file already exists.

If new code belongs to a different responsibility or feature category, add a
new Rust module or source file under the most appropriate directory and wire it
through `lib.rs` or the relevant binary as needed.

Treat 750 lines as the review threshold for Rust source files under
`parser/src/`, `sema/src/`, and `source/src/`. Files above that threshold should
be split into smaller responsibility-focused modules unless there is a
documented reason to keep the implementation together.

Keep module boundaries explicit:

- Lexing and tokenization should not be placed in runtime files.
- Parse-tree and public AST definitions should not be mixed with evaluation.
- Security profiles, capability metadata, regex policy, and verified IR belong
  in `sema/`, not in parser or runtime files.
- Host registry behavior should stay in runtime-focused modules.
- CLI argument handling should stay in binary or CLI support modules.
- Detailed syntax rules belong in `docs/Expression.md`, not only in comments.

## Security Requirements

Do not hard-code secrets, tokens, credentials, private URLs, cookies,
certificates, private keys, or production data in tests or examples.

Treat all DSL source strings and runtime JSON bindings as untrusted input.

When modifying parser, semantic analyzer, or runtime behavior, explicitly
consider:

- stack depth and recursive input shape
- expression step limits
- integer overflow
- division or remainder by zero
- string and array growth
- unknown variables, functions, methods, and object members
- profile phase restrictions and body-access inference
- regex admission, literal precompilation, and dynamic regex rejection paths
- malformed UTF-8 boundaries in diagnostics
- serialization compatibility
- fail-closed behavior on validation or runtime errors

For security-sensitive changes:

1. Identify attacker-controlled inputs.
2. Identify the affected trust boundary.
3. Add or update regression tests whenever practical.
4. Prefer fail-closed behavior for syntax, validation, and runtime errors.
5. Avoid `unwrap`, `expect`, `panic!`, `todo!`, or `unreachable!` on externally
   reachable input paths.
6. Run relevant tests or clearly state why they could not be run.
7. Summarize remaining risks and compatibility concerns.

## Do Not

- Do not remove tests just to make CI pass.
- Do not commit `target/`, generated build artifacts, temporary configs, logs,
  or local fixture output unless explicitly required.
- Do not silently change public syntax, canonical formatting, serialized AST,
  or runtime evaluation behavior.
- Do not add a parser generator or parser-combinator dependency without
  documenting why the handwritten parser is no longer sufficient.
- Do not add external I/O, callbacks, loops, mutation, or general-purpose
  scripting to the expression runtime without a new security design.

## Pull Request Checklist

Before opening or marking a pull request ready:

- The commit messages use the documented Conventional Commits format.
- The affected area is clear in the pull request description.
- User-visible behavior changes are covered in `README.md` or `docs/`.
- Syntax, canonical-formatting, compile, runtime, or CLI changes include
  regression tests whenever practical.
- Relevant local checks were run, or any skipped checks are explained.
- Temporary test data was removed.
- Security-sensitive changes describe attacker-controlled inputs, failure
  behavior, remaining risks, and compatibility concerns.
