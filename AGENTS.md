# AGENTS.md

## Project Overview

`online-dsl-forge` is a Rust-based online, in-memory DSL expression parser,
canonical AST library, compiler, and bounded runtime/operator interface.

This repository is organized as a monorepo. The main Rust implementation lives
under `source/`. Repository-level tests live under `tests/`. Technical
specifications and expression documentation live under `docs/`.

The project should stay close to the structure and contributor conventions used
by `/references/OxiBelt`, while adapting those conventions to untrusted DSL
input instead of reverse-proxy traffic.

## Repository Structure

- `parser/`
  - Rust crate for the lexer, parser, span-carrying AST, diagnostics, and
    canonical formatter.
- `source/`
  - Umbrella Rust crate for compile validation, runtime values, evaluator,
    public re-exports, and CLI.
- `parser/src/`
  - Syntax-only parser library source code.
- `source/src/`
  - Umbrella library source code for compiler and runtime behavior.
- `source/src/bin/`
  - CLI binaries.
- `tests/rust/`
  - Rust integration tests and repository-level checks.
- `tests/scripts/`
  - Test and source-layout check scripts.
- `docs/`
  - Technical specification, syntax reference, and behavior documentation.
- `.github/workflows/`
  - GitHub Actions workflows.

## Contributor Guidance

`CONTRIBUTING.md` is the source of truth for contributor workflow, security
requirements, pull request checks, and commit-message format. Use these
sections before making or reviewing changes:

- [Contribution Workflow](CONTRIBUTING.md#contribution-workflow)
- [Commit Messages](CONTRIBUTING.md#commit-messages)
- [Security Requirements](CONTRIBUTING.md#security-requirements)
- [Pull Request Checklist](CONTRIBUTING.md#pull-request-checklist)

If this file and `CONTRIBUTING.md` diverge on workflow, testing,
documentation, or Conventional Commits requirements, follow `CONTRIBUTING.md`
and update this pointer file only when agent-specific orientation changes.
