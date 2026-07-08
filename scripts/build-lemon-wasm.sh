#!/usr/bin/env bash
# Build the lemon-wasm crate into a wasm-bindgen web package.
# Output dir defaults to ./dist/lemon; override with the OUT env var (a consuming
# app can point it at its own tree, e.g. one whose CI has no Rust toolchain).
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."
. "$HOME/.cargo/env"
OUT="${OUT:-$PWD/dist/lemon}"
wasm-pack build crates/lemon-wasm --release --target web --out-dir "$OUT" --out-name lemon
rm -f "$OUT/.gitignore" "$OUT/README.md"   # wasm-pack drops a .gitignore that would hide the artifact
echo "lemon wasm built into $OUT"
