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
cargo llvm-cov --summary-only     # quick look

bash scripts/coverage.sh          # the CI gate: fails under 95% line + region
bash scripts/coverage.sh --html   # same, plus an HTML report in target/llvm-cov/
```

CI enforces **≥95%** line and region coverage on the workspace (`scripts/coverage.sh`).
The binary entry points (`crates/*/src/main.rs`) are excluded from the measurement —
they are thin shims (arg parsing, I/O wiring, the blocking HTTP server loop) whose
logic lives in the library crates, which are measured. New library code needs tests
to keep the gate green.

## Code style

- Format with `cargo fmt`; keep `cargo clippy` clean.
- Code, comments, and docs in English.

## Pull requests

- Keep PRs focused; one logical change per PR.
- Use [Conventional Commits](https://www.conventionalcommits.org/) for commit
  messages (e.g. `feat:`, `fix:`, `docs:`, `test:`).
- Make sure `cargo build --workspace`, `cargo test`, and `cargo fmt --check`
  pass before opening the PR.

## Releases

Releases are automated with [release-plz](https://release-plz.dev/) — every push to
`main` runs it.

- **Contributors:** nothing to do beyond Conventional Commits. `feat:` → minor,
  `fix:`/`perf:` → patch; `docs`/`chore`/`refactor`/`test`/`ci` don't trigger a release.
- **Maintainers:** release-plz keeps an open **"release" PR** that bumps the crate
  `version` fields and updates `CHANGELOG.md`. Merge it to tag, publish a GitHub
  release, and push the library crates to crates.io. That PR auto-regenerates on
  every push to `main`, so merge order relative to feature PRs doesn't matter — it
  always accumulates everything since the last release.
- **Don't** hand-edit crate `version` fields — release-plz owns them (a manual bump
  would publish immediately instead of going through the release PR).

Published library crates: `yuzu-core`, `yuzu-data`, `yuzu-source-s3`, and `lemon-lang`
(the `lemon` crate — still imported as `lemon` — publishes under the name `lemon-lang`).
The wasm/CLI/server crates are `publish = false`.

By contributing you agree that your contributions are licensed under the MIT
License (see [`LICENSE`](LICENSE)).
