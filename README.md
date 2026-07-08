# citrusinvest engine

![license](https://img.shields.io/badge/license-MIT-blue) ![rust](https://img.shields.io/badge/rust-2021-orange)

The open-source Rust backtest + strategy engine behind **citrusinvest** — the
**yuzu** backtest core plus the **lemon** strategy DSL. Pure, I/O-free math:
`(strategy spec JSON, data panels) -> Report`. It compiles to both native
(batch backtests via Rayon) and WASM (browser/Worker).

Top-level entry (in the `yuzu-core` crate):

```rust
yuzu_core::run_backtest(spec_json, ctx, price_key, cfg) -> Result<Report, EngineError>
// ctx: &EvalContext (numeric panels keyed by series name + a symbol→industry map)
```

Architecture, DSL vocabulary, NAV model, metric conventions, and the Report JSON
contract live in [`docs/backtest-engine.md`](docs/backtest-engine.md).

## Build

```bash
cargo build --workspace
```

### WASM

The two wasm crates build to a repo-local `dist/` by default:

```bash
bash scripts/build-yuzu-wasm.sh    # -> dist/yuzu   (yuzu-core backtest boundary)
bash scripts/build-lemon-wasm.sh   # -> dist/lemon  (lemon parse/format)
```

Override the output directory with `OUT`:

```bash
OUT=/path/to/pkg bash scripts/build-yuzu-wasm.sh
```

(Requires `wasm-pack`.)

## Test

```bash
cargo test
```

## Coverage

```bash
# one-time: rustup component add llvm-tools-preview && cargo install cargo-llvm-cov
cargo llvm-cov --summary-only
```

The `cargo llvm-cov` run also executes the test suite, so it doubles as the test step.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

MIT — see [`LICENSE`](LICENSE).
