# Dependency refresh qualification: 2026-11-10

Baseline: `3100587882946adeaf4e23303a5f1ca14172bfaa`. This refresh covers the
single Cargo workspace (library and fuzz crate), both pnpm lockfile documents,
toolchains, Cargo-installed tools, and every GitHub Action reference.

## Compatibility and policy

- Minimum supported Rust version: `1.98` → `1.99.1`; stable CI: `1.98.1` → `1.99.1`.
- ASan nightly: `nightly-2026-09-08` → `nightly-2026-11-10`, including LLVM tools.
- Node.js LTS: `24.20.0` → `26.13.0`; pnpm: `12.3.4` → `12.18.0`.
- Cargo package versions remain `0.0.0`; release rewriting stays confined to release checkouts.
- pnpm keeps its 1,440-minute minimum release age, build allowlist, and upstream transitive constraints.
- JSON keeps `arbitrary_precision` and `float_roundtrip`; YAML keeps default features disabled and only `deserialize` enabled.
- Production Rust source and golden fixtures are unchanged. No DSL/API migration is required beyond the compiler minimum.

## Cargo inventory

| Package | Before | After |
| --- | --- | --- |
| `annotate-snippets` | `0.12.16` | `0.12.17` |
| `arbitrary` | `1.4.2` | `1.5.0` |
| `cc` | `1.4.5` | `1.11.1` |
| `cfg-if` | `1.0.4` | `1.0.5` |
| `encoding_rs` | `0.8.40` | `0.8.44` |
| `equivalent` | `1.0.2` | `1.0.3` |
| `find-msvc-tools` | `0.1.12` | `0.1.15` |
| `granit-parser` | `1.2.0` | `2.1.0` |
| `itoa` | `1.0.18` | `1.0.19` |
| `libc` | `0.2.189` | `0.2.191` |
| `multiversion` | `0.8.0` | `removed` |
| `multiversion-macros` | `0.8.0` | `removed` |
| `serde-saphyr` | `1.2.0` | `2.1.1` |
| `serde_json` | `1.0.151` | `1.0.154` |
| `serde_spanned` | `1.1.1` | `1.1.3` |
| `smallvec` | `1.16.0` | `1.16.5` |
| `syn` | `2.0.119, 3.0.5` | `3.0.10` |
| `target-features` | `0.1.6` | `removed` |
| `toml` | `1.1.5+spec-1.1.0` | `1.2.1+spec-1.1.0` |
| `toml_datetime` | `1.1.1+spec-1.1.0` | `1.1.3+spec-1.1.0` |
| `toml_parser` | `1.1.3+spec-1.1.0` | `1.1.5+spec-1.1.0` |
| `toml_writer` | `1.1.2+spec-1.1.0` | `1.2.0+spec-1.1.0` |
| `unicode-ident` | `1.0.24` | `1.0.27` |
| `unicode-width` | `0.2.2` | `0.2.3` |
| `winnow` | `1.0.4` | `1.0.5` |

Unchanged resolved packages: `aho-corasick 1.1.5`, `anstyle 1.0.14`, `arraydeque 0.5.1`, `autocfg 1.5.1`, `core_detect 1.0.0`, `encoding_rs_io 0.1.8`, `getrandom 0.4.3`, `hashbrown 0.17.1`, `indexmap 2.14.2`, `jobserver 0.1.35`, `libfuzzer-sys 0.4.13`, `memchr 2.8.3`, `multiversion_no_op 1.0.0`, `num-traits 0.2.19`, `proc-macro2 1.0.107`, `quote 1.0.47`, `r-efi 6.0.0`, `regex 1.13.1`, `regex-automata 0.4.18`, `regex-syntax 0.8.11`, `rustversion 1.0.23`, `scopeguard 1.2.0`, `serde 1.0.229`, `serde_core 1.0.229`, `serde_derive 1.0.229`, `shlex 2.0.1`, `simdutf8 0.1.5`, `zmij 1.0.23`.

`r-efi 6.0.0` remains below upstream `7.1.0`: `getrandom 0.4.3` requires
`r-efi ^6`. No incompatible override is introduced. Removed packages do not
need replacement upgrades.

## pnpm inventory

The package-manager and application dependency documents were both regenerated.
Package entries increased from 94 to 100, including optional platform binaries.

| Package | Before | After |
| --- | --- | --- |
| `@oxlint-tsgolint/darwin-arm64` | `7.0.2001` | `7.0.2006` |
| `@oxlint-tsgolint/darwin-x64` | `7.0.2001` | `7.0.2006` |
| `@oxlint-tsgolint/linux-arm64` | `7.0.2001` | `7.0.2006` |
| `@oxlint-tsgolint/linux-x64` | `7.0.2001` | `7.0.2006` |
| `@oxlint-tsgolint/win32-arm64` | `7.0.2001` | `7.0.2006` |
| `@oxlint-tsgolint/win32-x64` | `7.0.2001` | `7.0.2006` |
| `@types/node` | `26.4.1` | `26.6.6` |
| `es-toolkit` | `1.52.0` | `1.53.2` |
| `oxlint` | `1.81.0` | `1.93.0` |
| `oxlint-tsgolint` | `7.0.2001` | `7.0.2006` |
| `pnpm` | `12.3.4` | `12.18.0` |
| `semver` | `7.8.5` | `7.8.6` |
| `smol-toml` | `1.8.0` | `1.9.1` |
| `tsx` | `4.23.13` | `4.23.15` |
| `undici-types` | `8.3.0` | `8.9.0` |
| `zod` | `4.5.4` | `4.6.7` |

Platform packages track their parent releases: `@pnpm/exe.*` → `12.18.0`,
`@esbuild/*` remain at `0.28.2`, and `@oxlint/*` → `1.93.0`. pnpm adds Android, FreeBSD, PowerPC, RISC-V, and s390x executable
entries where upstream provides them. These optional targets were inventoried;
execution qualification is Linux x86_64.

`undici-types 8.9.0` remains below upstream `8.12.4` because
`@types/node 26.6.6` requires `~8.9.0`. Platform pnpm packages use the exact
versions required by pnpm even where their npm `latest` tags point to older
bootstrap releases. Direct dependencies report no outstanding updates.

## CI and installed tools

| Component | Before | After |
| --- | --- | --- |
| `cargo-audit` | `0.22.2` | `0.23.0` |
| `cargo-deny` | `0.20.2` | unchanged |
| `cargo-fuzz` | `0.13.2` | unchanged |
| `actions/checkout` | `7.0.1` | unchanged |
| `actions/cache` | `6.1.0` | unchanged |
| `actions/setup-node` | `7.0.0` | `7.1.0` |
| `pnpm/setup` | `2.1.0` | `3.1.2` |
| `actions/upload-artifact` | `7.0.1` | `7.0.2` |
| `actions/download-artifact` | `8.0.1` | `8.0.2` |
| `github/codeql-action` | `4.37.9` | `4.39.3` |
| `rust-lang/crates-io-auth-action` | `1.0.5` | `1.0.6` |

Action release tags were resolved to commit SHAs; annotated tags were peeled.
The CodeQL Action release was distinguished from its bundle releases. Action
metadata supports the existing inputs; all Actions run on Node 24 internally,
independently of the project’s Node 26 release-tooling runtime.

## Qualification

- Baseline and refreshed Rust tests: 189 passed each; pnpm tests: 11 passed each.
- Rust formatting, test formatting, module-size checks, Clippy with warnings denied, and locked full-workspace compilation passed.
- Frozen pnpm install, lint, typecheck, tests, version-placeholder checks, and audit passed; audit reported zero vulnerabilities.
- `cargo audit` and `cargo deny check advisories` passed against current RustSec data.
- All four workflows passed checksum-verified `actionlint 1.7.12`; shell syntax and whitespace checks passed.
- All six stable/ASan smoke combinations passed (256 runs per combination).
- Release rewriting and locked publish dry-run passed in a disposable copy; no upload occurred.

All three ASan+LeakSanitizer campaigns passed their 900-second bounds (901 seconds
reported by libFuzzer), with no crash artifacts or leak reports.

| Target | Executions | Peak RSS (MiB) |
| --- | --- | --- |
| `dsl_expression` | 3,496,361 | 965 |
| `expression_pipeline` | 1,815,883 | 931 |
| `rulepack_render` | 1,686,491 | 577 |

The first corpus-minimization attempts failed with `Invalid cross-device link`
after successful libFuzzer merge: pinned `cargo-fuzz 0.13.2` creates its merge
directory under the checkout, then renames into the requested corpus directory.
This environment mounts `/tmp` separately. All three minimization reruns passed
using an ignored `target/dependency-refresh-temp` directory on the checkout
filesystem. The original corpus coverage replays also passed for all targets.
Coverage and report replay of the minimized corpora passed for all three targets.
The initial lifecycle returned failure because of the filesystem error; the
successful reruns are separate evidence, not a relabeling of that attempt.

## Resolved graph checksums

SHA-256 of the qualified committed lockfile contents:

| File | SHA-256 |
| --- | --- |
| `Cargo.lock` | `bcc2ba4bd6be8bd1194df71ad7f9ac169731f3bf06539b28f620d3c15bcee102` |
| `pnpm-lock.yaml` | `ce9f15a476c80056dda3ddacfbdece74ed95a18883b2d37c5e74a213f4ebcdea` |

## GitHub reconciliation and limits

The planning/implementation read found no open non-PR issues and 14 open
dependency PRs: #38, #41–49, #50–51, and #53–54. Discussions and reviews contained
no additional objections. Signed commit [`4b67891`](https://github.com/OxiBelt/online-dsl-forge/commit/4b678914d43ad359b493127de34553b0036f4220)
landed the pnpm/Node and Action batch after standalone staged-tree qualification.
GitHub verified its signature and trailer. PRs #38 and #41–49 were then closed
through the GitHub connector with assistance-disclosed commit references.
Signed commit [`4879492`](https://github.com/OxiBelt/online-dsl-forge/commit/4879492afb857911886420ea15e7c72111c5207d)
landed the qualified Cargo/toolchain batch with a GitHub-verified signature and
trailer. PRs #50–51 and #53–54 were then closed through the same connector.
All 14 original dependency proposals are covered and closed.

The initial local Cargo cache was a broken symlink. Qualification used isolated
Cargo caches and artifacts under `/tmp/online-dsl-refresh`. Baseline Rust tests
were rerun from an immutable copy with a separate target directory so concurrent
workflow edits and cached absolute paths could not contaminate the result.

The repository contains no vendored source, container image, Kubernetes/Helm,
or browser dependency surface. Cluster qualification is not applicable. Hosted
Actions execution after delivery is not part of this local qualification.
Generated corpora, coverage, logs, and temporary release copies are excluded
from commits. Checkout-local campaign temporary files were removed; coverage
and reproduction evidence are retained under `/tmp/online-dsl-refresh`. Dependency resolution is complete subject to the two upstream
transitive constraints documented above.
