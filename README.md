# backtest-engine

A Rust backtest engine with a strategy DSL. The `engine-core` crate is
pure and I/O-free: `(strategy spec JSON, data panels) -> Report`.

Top-level entry: `engine_core::run_backtest(spec_json, ctx, price_key, cfg) -> Result<Report, EngineError>`.

Architecture, DSL vocabulary, NAV model, metric conventions, Report JSON contract,
and phasing live in [`docs/backtest-engine.md`](docs/backtest-engine.md).

## Test

```bash
cargo test --manifest-path Cargo.toml
```

## Coverage

```bash
# one-time: rustup component add llvm-tools-preview && cargo install cargo-llvm-cov
cargo llvm-cov --manifest-path Cargo.toml --summary-only
```

CI gates on `--fail-under-lines 90` (the same `cargo llvm-cov` run also executes the
test suite, so it doubles as the test step).

## Regenerate golden fixtures

```bash
cd crates/engine-core/tests/golden
uv run --with pandas --with numpy --with scipy python generate.py
```

Fixtures are committed; regenerate only when adding ops or changing the sample data.
