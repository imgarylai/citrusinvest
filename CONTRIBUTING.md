# Contributing

Thanks for your interest in the citrusinvest engine. It's a Cargo workspace of
`yuzu-*` crates (backtest core, data, server, CLI, wasm, S3 source) plus the
`lemon` strategy DSL and its wasm binding.

## Build

```bash
cargo build --workspace
```

### WASM

The two wasm crates build to a repo-local `dist/` by default (requires
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/)):

```bash
bash scripts/build-yuzu-wasm.sh    # -> dist/yuzu
bash scripts/build-lemon-wasm.sh   # -> dist/lemon
```

Redirect the output with `OUT`:

```bash
OUT=/path/to/pkg bash scripts/build-yuzu-wasm.sh
```

## Test

```bash
cargo test
```

Engine behavior is pinned by **golden fixtures** under
`crates/yuzu-core/tests/golden/` — committed expected outputs replayed offline.
If you add or change an op, add or update its fixture in the same PR.

Coverage:

```bash
# one-time: rustup component add llvm-tools-preview && cargo install cargo-llvm-cov
cargo llvm-cov --summary-only
```

## Code style

- Format with `cargo fmt`; keep `cargo clippy` clean.
- Code, comments, and docs in English.

## Pull requests

- Keep PRs focused; one logical change per PR.
- Use [Conventional Commits](https://www.conventionalcommits.org/) for commit
  messages (e.g. `feat:`, `fix:`, `docs:`, `test:`).
- Make sure `cargo build --workspace`, `cargo test`, and `cargo fmt --check`
  pass before opening the PR.

By contributing you agree that your contributions are licensed under the MIT
License (see [`LICENSE`](LICENSE)).
