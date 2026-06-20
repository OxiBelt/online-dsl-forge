#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

mapfile -t test_rust_files < <(find tests -type f -name '*.rs' | sort)

if ((${#test_rust_files[@]} > 0)); then
  rustfmt \
    --check \
    --edition 2024 \
    --config-path tests/rustfmt.toml \
    --config skip_children=true \
    "${test_rust_files[@]}"
fi
