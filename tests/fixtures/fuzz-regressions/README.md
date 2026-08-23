# Fuzz regressions

Store only minimized, reviewed reproducers for confirmed fuzz defects below the
directory matching the target in `fuzz/targets.toml`. Every non-README fixture
must be named in `tests/rust/fuzz_regressions.rs` and replayed through the
shared target exercise function.
