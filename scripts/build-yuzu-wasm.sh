#!/usr/bin/env bash
# Build the yuzu-wasm crate into a wasm-bindgen web package.
# Output dir defaults to ./dist/yuzu; override with the OUT env var (a consuming
# app can point it at its own tree, e.g. one whose CI has no Rust toolchain).
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."
. "$HOME/.cargo/env"
OUT="${OUT:-$PWD/dist/yuzu}"
wasm-pack build crates/yuzu-wasm --release --target web --out-dir "$OUT" --out-name yuzu
rm -f "$OUT/.gitignore" "$OUT/README.md"   # wasm-pack drops a .gitignore that would hide the artifact
echo "wasm built into $OUT"
