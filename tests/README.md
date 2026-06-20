# online-dsl-forge Test Assets

- `rust/`: repository-root Cargo integration tests linked from
  `source/Cargo.toml`
- `scripts/check-tests-rustfmt.sh`: enforces `tests/rustfmt.toml` formatting for
  tracked Rust files under `tests/`
- `scripts/check-rust-module-size.sh`: keeps Rust source modules under the
  responsibility-focused review threshold

Run common checks from the repository root:

```sh
cargo fmt --check
tests/scripts/check-tests-rustfmt.sh
tests/scripts/check-rust-module-size.sh
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
