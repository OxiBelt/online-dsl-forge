# Fuzz Testing

`online-dsl-forge` uses `cargo-fuzz` and libFuzzer to exercise untrusted DSL and
rulepack input in memory. `fuzz/targets.toml` is the canonical program: it names
every target, its owner, input limit, trust boundary, invariants, reviewed seed
directory, dictionary, coverage landmarks, and regression directory.

## Toolchain

The fuzz runner accepts two profiles:

- `stable` uses Rust `1.98.1` without a sanitizer for fast panic and invariant
  coverage. It is supported only for smoke runs.
- `asan` uses `nightly-2026-09-08` with AddressSanitizer. Sustained campaigns
  also enable leak detection and use `llvm-tools-preview` for coverage.

Install the pinned toolchains and `cargo-fuzz` release:

```sh
rustup toolchain install 1.98.1 --profile minimal
rustup toolchain install nightly-2026-09-08 --profile minimal \
  --component llvm-tools-preview
cargo +1.98.1 install cargo-fuzz --version 0.13.2 --locked
test "$(cargo +1.98.1 fuzz --version)" = "cargo-fuzz 0.13.2"
```

The runner writes mutable corpora, crash artifacts, and reproduction reports
below `${RUNNER_TEMP:-/tmp}`. Coverage is written below the ignored
`fuzz/coverage/` directory. It rejects unknown targets, unsafe catalog paths,
symlinks, oversized inputs, and corpus growth beyond the committed bounds.

## Targets

### `dsl_expression`

Arbitrary bytes are mapped through lossy UTF-8 because the Rust API accepts
`&str`. The target exercises tokenization, parsing, AST serialization,
canonical formatting, and reparsing. Token, AST, and diagnostic spans must stay
ordered, in bounds, and on UTF-8 character boundaries. Canonical formatting
must be idempotent. Successful parses must also round-trip through the default
`serde_json` recursion limit; more deeply nested AST shapes are rejected by the
parser. Parsed floats must be finite and must retain their exact `f64` bits
across that JSON round-trip, including decimal values that require the precise
float parser. Selector-derived `ParseLimits` and `AstFormatLimits` also drive
the explicit fallible entry points through zero, exact-boundary, and rejection
paths.

### `expression_pipeline`

The first seven bytes select semantic profiles, compile options, and bounded
evaluation limits. The remaining expression can be followed by
`---BINDINGS---` and a JSON bindings value. The target covers generic compile
and evaluation plus WAF and OxiRule semantic configurations. It never installs
external callbacks or performs I/O.

Successful analysis must retain the parsed AST, remain within static profile
bounds, and keep capability tickets, metadata, and precompiled regex literals
aligned. Regex literals retain every admitted source occurrence, while the
compiled cache contains exactly one entry per unique flavor and pattern pair.
Repeated compile and evaluation outcomes must be deterministic, and canonical
formatting must preserve the value or fail-closed error class. Runtime bindings
and both evaluation passes use selector-derived `RuntimeResourceLimits` so
value-graph and cumulative-byte rejections are part of the target.

### `rulepack_render`

The first two bytes select bounded render options. The remaining TOML can be
followed by `---FILES---` and JSON containing `variables`, `files`, and an
optional `source_commit`. At most eight referenced files are admitted through
`MemoryFileResolver`; filesystem and network access are outside the target.
Inspection, reference discovery, install rendering, and bundle rendering must
return deterministic values or errors. The target calls every public
`*_with_limits` renderer plus `render_text_with_limits` with selector-derived
manifest, file, variable, placeholder, input, and output budgets.

## Running and Reproducing

Run both smoke profiles for one target:

```sh
ONLINE_DSL_FORGE_FUZZ_PROFILE=stable \
  tests/scripts/run-fuzz-target.sh smoke expression_pipeline
ONLINE_DSL_FORGE_FUZZ_PROFILE=asan \
  tests/scripts/run-fuzz-target.sh smoke expression_pipeline
```

Run a fifteen-minute ASan campaign and its evidence lifecycle:

```sh
tests/scripts/run-fuzz-target.sh campaign expression_pipeline 900
tests/scripts/run-fuzz-target.sh minimize expression_pipeline 900
tests/scripts/run-fuzz-target.sh cmin expression_pipeline 900
tests/scripts/run-fuzz-target.sh coverage expression_pipeline 900
tests/scripts/run-fuzz-target.sh report expression_pipeline 900
```

To reproduce one emitted artifact directly, keep the same pinned nightly and
sanitizer:

```sh
cargo +nightly-2026-09-08 fuzz run --sanitizer address \
  expression_pipeline /absolute/path/to/crash-artifact
```

## Corpus and Regression Policy

Smoke jobs use only reviewed seeds committed below `fuzz/seeds/`. Sustained
default-branch jobs may restore a target-specific corpus, but save it only
after a successful campaign and corpus minimization. CI-generated corpus files
are candidates for review, not source updates.

For a confirmed defect:

1. Reproduce and minimize the artifact with the pinned ASan toolchain.
2. Fix the defect and add the narrowest practical ordinary regression test.
3. Place the minimized byte input below the target directory in
   `tests/fixtures/fuzz-regressions/` when replaying the full fuzz contract adds
   value.
4. Register its path and target in `tests/rust/fuzz_regressions.rs`.
5. Run the regression, catalog contract, stable smoke, and ASan smoke before
   committing.

Do not commit generated corpora wholesale, secrets, credentials, private URLs,
production inputs, unreviewed crash artifacts, or files obtained through
automatic issue creation.
