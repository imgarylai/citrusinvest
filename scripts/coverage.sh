#!/usr/bin/env bash
# Workspace test-coverage gate. Runs cargo-llvm-cov over the whole workspace and
# fails if line or region coverage drops below the threshold (default 95%).
#
#   scripts/coverage.sh              # enforce the default 95% floor
#   scripts/coverage.sh --html       # also write an HTML report under target/llvm-cov/
#   COVERAGE_MIN=90 scripts/coverage.sh
#
# One-time setup:
#   rustup component add llvm-tools-preview
#   cargo install cargo-llvm-cov
#
# Binary-only glue is excluded: the entry points (`src/main.rs`) and the
# yuzu-cli command layer (`yuzu-cli/src/commands/`) are thin shims — arg
# parsing, I/O wiring, the blocking HTTP server loop, and vendor-sync
# orchestration — whose reusable logic lives in the library crates, which ARE
# measured. (This CLI code was previously inside `main.rs` and thus already
# excluded; the module split just moved it under `commands/`, so it stays
# exempt to keep the number stable.) Keeping this exclusion list in one place
# means local runs and CI agree on the number.
set -euo pipefail

MIN="${COVERAGE_MIN:-95}"
IGNORE='src/main\.rs$|yuzu-cli/src/commands/'

extra=()
for arg in "$@"; do
  case "$arg" in
    --html) extra+=(--html) ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

cargo llvm-cov --workspace \
  --ignore-filename-regex "$IGNORE" \
  --fail-under-lines "$MIN" \
  --fail-under-regions "$MIN" \
  "${extra[@]}"
