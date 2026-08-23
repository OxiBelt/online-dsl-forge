# online-dsl-forge Test Assets

- `rust/`: repository-root Cargo integration tests linked from
  `source/Cargo.toml`
- `scripts/check-tests-rustfmt.sh`: enforces `tests/rustfmt.toml` formatting for
  tracked Rust files under `tests/`
- `scripts/check-rust-module-size.sh`: keeps Rust source modules under the
  responsibility-focused review threshold
- `scripts/run-fuzz-target.sh`: runs catalogued smoke, sustained, minimization,
  coverage, and reporting operations with fixed resource bounds
- `fixtures/fuzz-regressions/`: contains only minimized, reviewed reproducers
  registered by `rust/fuzz_regressions.rs`
- `../devops/sources/versioning.ts`: validates that committed Cargo metadata
  stays at the `0.0.0` placeholder and applies SemVer tag versions in release CI

Run common checks from the repository root:

```sh
pnpm run versioning:check
cargo fmt --check
tests/scripts/check-tests-rustfmt.sh
tests/scripts/check-rust-module-size.sh
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Run a bounded stable fuzz smoke test after installing `cargo-fuzz 0.13.2`:

```sh
ONLINE_DSL_FORGE_FUZZ_PROFILE=stable \
  tests/scripts/run-fuzz-target.sh smoke dsl_expression
```

See `docs/Fuzzing.md` for the target contracts, nightly setup, sustained
campaigns, and regression-promotion workflow.
