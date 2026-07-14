# citrusquant engine

[![CI](https://github.com/citrusquant/citrusquant/actions/workflows/ci.yml/badge.svg)](https://github.com/citrusquant/citrusquant/actions/workflows/ci.yml) [![docs](https://img.shields.io/badge/docs-pages-blue)](https://citrusquant.com/) [![yuzu-core](https://img.shields.io/crates/v/yuzu-core?label=yuzu-core)](https://crates.io/crates/yuzu-core) [![lemon-lang](https://img.shields.io/crates/v/lemon-lang?label=lemon-lang)](https://crates.io/crates/lemon-lang) [![npm yuzu-wasm](https://img.shields.io/npm/v/@citrusquant/yuzu-wasm?logo=npm&label=yuzu-wasm)](https://www.npmjs.com/package/@citrusquant/yuzu-wasm) [![npm lemon-wasm](https://img.shields.io/npm/v/@citrusquant/lemon-wasm?logo=npm&label=lemon-wasm)](https://www.npmjs.com/package/@citrusquant/lemon-wasm) [![PyPI yuzu-backtest](https://img.shields.io/pypi/v/yuzu-backtest?logo=pypi&logoColor=white&label=yuzu-backtest)](https://pypi.org/project/yuzu-backtest/) ![license](https://img.shields.io/badge/license-MIT-blue)

The open-source Rust backtest + strategy engine behind **citrusquant** — the
**yuzu** backtest core, the **pomelo** data-engineering layer, and the **lemon**
strategy DSL. Pure, I/O-free math: `(strategy spec JSON, data panels) -> Report`.
It compiles to both native (batch backtests via Rayon) and WASM (browser/Worker).

Top-level entry (in the `yuzu-core` crate):

```rust
yuzu_core::run_backtest(spec_json, ctx, price_key, cfg) -> Result<Report, EngineError>
// ctx: &EvalContext (numeric panels keyed by series name + a symbol→industry map)
```

Architecture, DSL vocabulary, NAV model, metric conventions, and the Report JSON
contract live in [`docs/backtest-engine.md`](docs/backtest-engine.md).
**Bring-your-own data** (on-disk tree, CSV columns, series names):
[`docs/data-layout.md`](docs/data-layout.md).
**FMP Starter-tier gaps** (which features need which panels):
[`docs/fmp-data-source.md`](docs/fmp-data-source.md).
**Data sources (FMP · EODHD · Alpha Vantage · Finnhub + assemble):**
[`docs/data-sources.md`](docs/data-sources.md).
**EODHD official sync:**
[`docs/eodhd-data-source.md`](docs/eodhd-data-source.md).
**Alpha Vantage official sync:**
[`docs/alpha-vantage-data-source.md`](docs/alpha-vantage-data-source.md).
**Finnhub official sync:**
[`docs/finnhub-data-source.md`](docs/finnhub-data-source.md).

## Crate families

The workspace is organized into three crate families:

- **yuzu** — the backtest engine (`yuzu-core`), multi-run research over it (`yuzu-research`), and the product apps around them (CLI/server/wasm/py).
- **pomelo** — data engineering: native I/O, storage backends, data sync, quality audit.
- **lemon** — the strategy language and its tooling.

The library crates are published so you can link them from your own service — e.g.
`pomelo-fmp::sync_into` + `pomelo-s3` for a data-sync service, or `yuzu-core` +
`pomelo-data` for a backtest service (`yuzu-server` is the reference implementation).
It's a layered stack — `pomelo-*` → `yuzu-core` → `lemon` — not parallel silos.

## Published crates

| Crate | Description |
|-------|-------------|
| [`yuzu-core`](https://crates.io/crates/yuzu-core) | Pure, I/O-free backtest engine core. |
| [`yuzu-research`](https://crates.io/crates/yuzu-research) | Multi-run research over the engine: sweeps, grids, walk-forward, lookahead-bias detection. |
| [`pomelo-data`](https://crates.io/crates/pomelo-data) | Native I/O: gzip CSV (and read-only Parquet, behind the `parquet` feature) price/fundamental files → panels. |
| [`pomelo-s3`](https://crates.io/crates/pomelo-s3) | S3-compatible `ObjectSource`/`ObjectSink` for `pomelo-data`. |
| [`pomelo-audit`](https://crates.io/crates/pomelo-audit) | Read-only data-quality audit of a data-layout tree (`yuzu-cli data-audit`). |
| [`pomelo-http`](https://crates.io/crates/pomelo-http) | Shared HTTP client, rate-limit/retry `Fetcher`, and `WriteMode` for the `pomelo-*` vendor sync crates (no vendor logic). |
| [`pomelo-fmp`](https://crates.io/crates/pomelo-fmp) | Bring-your-own-key FMP data sync + snapshot-factor formulas; writes to local disk or S3/R2. |
| [`pomelo-eodhd`](https://crates.io/crates/pomelo-eodhd) | Bring-your-own-key EODHD data sync (second official path; `yuzu-cli eodhd-sync`). |
| [`pomelo-alpha-vantage`](https://crates.io/crates/pomelo-alpha-vantage) | Bring-your-own-key Alpha Vantage data sync (epic #209; `yuzu-cli av-sync`). |
| [`pomelo-finnhub`](https://crates.io/crates/pomelo-finnhub) | Bring-your-own-key Finnhub data sync (epic #210; `yuzu-cli finnhub-sync`). |
| [`lemon-lang`](https://crates.io/crates/lemon-lang) | The lemon strategy DSL (imported as `lemon`). |

**Python**: `crates/yuzu-py` binds the engine + DSL for Python (`pip install ./crates/yuzu-py`,
module `yuzu`) — see [`crates/yuzu-py/README.md`](crates/yuzu-py/README.md).

The `wasm`/`cli`/`server` crates are not published.

## Install the lemon CLI

One command gets you the `lemon` binary — write a strategy, `lemon
strategy.lemon`, read the report (Linux / macOS; checksum-verified prebuilt
binaries from GitHub Releases):

```bash
curl -fsSL https://raw.githubusercontent.com/citrusquant/citrusquant/main/scripts/install.sh | sh
```

See [`docs/lemon.md`](docs/lemon.md) for the language and the runner
(`fmt` / `lint` / `check` / `run`). From source:
`cargo install --path crates/lemon-cli`.

## Build

```bash
cargo build --workspace
```

### WASM

The two wasm crates build to a repo-local `dist/` by default:

```bash
bash scripts/build-yuzu-wasm.sh    # -> dist/yuzu   (yuzu-core backtest boundary)
bash scripts/build-lemon-wasm.sh   # -> dist/lemon  (lemon parse/format + editor services)
```

Override the output directory with `OUT`:

```bash
OUT=/path/to/pkg bash scripts/build-yuzu-wasm.sh
```

(Requires `wasm-pack`.)

## Editor tooling

The lemon DSL ships with editor support driven by the same op catalog as the
parser and JSON schema, so highlighting, hover, and completions never drift from
the language:

- **Language server** — `lemon-lsp`, a [tower-lsp](https://github.com/ebkalderon/tower-lsp)
  server over stdio providing hover (op signatures + descriptions), completion
  (op names, keyword arguments, `let`-bound names, known series), and live
  diagnostics sourced from the DSL linter (parse errors, unused `let`s, and —
  when the engine's series list is supplied via `initializationOptions.series` —
  unknown-series warnings with did-you-mean suggestions). The intelligence is a
  pure, I/O-free `lemon::services` module; the binary is a thin shim.

  ```bash
  cargo install --path crates/lemon-lsp
  ```

- **VS Code extension** — a TextMate grammar plus a client for `lemon-lsp`, under
  [`editors/vscode`](editors/vscode).

- **In-browser editor** — the same `lemon::services` core is exported through
  `lemon-wasm` (`diagnostics` / `hover` / `completions`) for the web editor.

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
