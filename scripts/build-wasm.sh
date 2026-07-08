#!/usr/bin/env bash
# Build yuzu-wasm and copy the wasm-bindgen output into the web app.
# Re-run whenever yuzu-core or yuzu-wasm changes. Output is committed
# because the web CI has no Rust toolchain.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
cd "$SCRIPT_DIR/.."
. "$HOME/.cargo/env"
OUT="$REPO_ROOT/apps/web/src/lib/yuzu/wasm"
wasm-pack build crates/yuzu-wasm --release --target web --out-dir "$OUT" --out-name yuzu
rm -f "$OUT/.gitignore" "$OUT/README.md"   # wasm-pack drops a .gitignore that would hide the artifact
echo "wasm built into $OUT"
