# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

citrusquant is a Rust backtest engine (a Cargo workspace). Crates fall into three
families by prefix:

- **`yuzu-*`** — the backtest engine **and the product apps around it** (CLI, server,
  wasm, Python). Rule of thumb: **apps live under the `yuzu` name**, even
  data-engineering ones — e.g. `yuzu-cli fmp-sync` is a `yuzu` binary that wraps the
  `pomelo-*` crates.
- **`pomelo-*`** — data-engineering **libraries**: native I/O, storage backends, vendor
  data sync (produce the panels the engine consumes).
- **`lemon-*`** — the strategy **language** and its tooling.

It's a layered stack, not parallel silos: `pomelo-*` → `yuzu-core` → `lemon`.

- `yuzu-core` — pure, I/O-free backtest engine core (`run_backtest`).
- `yuzu-research` — multi-run research over the engine: sweeps, grids, walk-forward, lookahead-bias detection (`run_sweep`/`run_walkforward`/…). Composes `yuzu-core::research` primitives; the CLI re-exports it.
- `pomelo-data` — native data loading (reads gzip CSV price/fundamental files into panels).
- `pomelo-s3` — an S3-backed data source (an `ObjectSource`/`ObjectSink` impl; `LocalSource` reads/writes local files).
- `pomelo-http` — shared HTTP client, rate-limit/retry `Fetcher`, and `WriteMode` for the vendor sync crates (#211). **No vendor logic**; the `pomelo-*` adapters build on it instead of re-copying the plumbing.
- `pomelo-fmp` — bring-your-own-key FMP data sync + snapshot-factor formulas. `sync_into` writes a data-layout tree to any `ObjectSink` (local or S3/R2); the CLI is a thin wrapper. FMP stays out of `yuzu-core`/`pomelo-data`/WASM.
- `pomelo-eodhd` — bring-your-own-key EODHD data sync (second official vendor path; epic #192). Same data-layout contract as FMP; CLI: `yuzu-cli eodhd-sync`.
- `pomelo-alpha-vantage` — bring-your-own-key Alpha Vantage data sync (epic #209). Same data-layout contract; CLI: `yuzu-cli av-sync` / `av-symbols`.
- `pomelo-finnhub` — bring-your-own-key Finnhub data sync (epic #210). Same data-layout contract; CLI: `yuzu-cli finnhub-sync` (phases #225–#230).
- `pomelo-audit` — read-only data-quality audit of a data-layout tree (`run_data_audit` → an `OK`/`WARN`/`FAIL` report). Exposed as `yuzu-cli data-audit`; the CLI is a thin caller.
- `lemon` / `lemon-lang` — the **strategy DSL**. Strategies are written in lemon and lowered to a JSON `Expr` tree the engine evaluates. Its `services` module provides pure editor language services (diagnostics/hover/completions).
- `lemon-lsp` — a thin `tower-lsp` language server over `lemon::services` (hover, completion, live diagnostics). Editor integration (incl. a VS Code extension + TextMate grammar) lives under `editors/`.
- `lemon-cli` — the `lemon` binary: `fmt` / `lint` / `check` plus `run`, which lowers a `.lemon` file and backtests it over a local data tree (`lemon run strategy.lemon --data …`). Lives outside `lemon-lang` because `yuzu-core` depends on the language crate — the engine wiring for `run` would be a dependency cycle there; `lemon-lang` stays pure and I/O-free.

## Build · test · lint

```bash
cargo build --workspace          # build
cargo test --workspace           # test
cargo fmt --all                  # format (CI checks --check)
cargo clippy --workspace -- -D warnings   # lint (lib + bins)
bash scripts/build-yuzu-wasm.sh  # wasm -> dist/yuzu (build-lemon-wasm.sh for the DSL)
```

MSRV: **1.86**. Edition 2021.

## Writing or editing lemon strategies

If you generate or modify strategy code, **read [`docs/lemon.md`](docs/lemon.md)** — the full
language reference. The sharp edges that trip up generators:

- There is **no** `==`, `!=`, `&`, `|`, or `!`. Logical AND/OR/NOT are the words
  `and` / `or` / `not`; comparisons (`> < >= <=`) output `1.0`/`0.0`.
- Function calls take **positional args first, then keyword args** (`fn(x, n=3)`).
- List literals `[a, b]` exist **only** inside a call argument.
- An **unknown bare identifier silently becomes a data-series reference** — typos are not caught
  until engine evaluation. Valid series names (`close`, `high`, `volume`, `pe`, `roe`, …) are
  the engine's, not the parser's.
- A strategy is exactly one top-level expression (optionally preceded by `let name = …` bindings,
  which are inlined at parse time).

Machine-readable references for tool-use / structured output:

- [`schema/op-catalog.json`](schema/op-catalog.json) — every op with its arguments and defaults.
- [`schema/lemon-spec.schema.json`](schema/lemon-spec.schema.json) — JSON Schema for the spec tree.
- A compact prompt-ready cheat sheet: [`docs/lemon-prompt.md`](docs/lemon-prompt.md).

Validate generated lemon with the parser (`lemon::parse(src)` returns `ParseError { line, col, msg }`)
or the `lemon fmt` binary — feed it back to self-correct. `lemon lint --series close,pe,…`
additionally flags unknown series names (typos become silent `Data` leaves) and unused `let`s.

## Adding a `pomelo-*` vendor sync crate

New market-data vendors mirror the existing adapters. **`pomelo-finnhub` is the reference
implementation** (newest, cleanest) — copy its shape. Conventions:
[#211](https://github.com/citrusquant/citrusquant/issues/211).

**Required surface** (match the other adapters so the CLI stays consistent):

- Build the plumbing on **`pomelo-http`**, don't re-copy it: a thin `src/http.rs` shim
  (`pub(crate) type Fetcher<'a, H> = pomelo_http::Fetcher<'a, H, SyncConfig>` + re-export
  `HttpClient` / `HttpError` / feature-gated `UreqClient`), `impl pomelo_http::RetrySettings for
  SyncConfig`, and `pub use pomelo_http::WriteMode` from `config.rs`. This keeps every
  `Fetcher::new(http, cfg)` call site and `&Fetcher<H>` signature unchanged.
- `SyncConfig` (`from`/`to`, `rate_limit_per_min`, `max_retries`, `backoff_base`, `mode`,
  `include_*` flags) + `SyncSummary`; `sync` / `sync_into` over `ObjectSink + ObjectSource`.
- Pure scoring math from **`pomelo_data::factors`** (`percentile_rank` / `pe_industry_pctile` /
  `analyst_upside_pct`); keep only the vendor-specific rating mapper in the crate (vendors take
  different inputs — a string, a 1–5 score, count buckets — so it can't be shared).
- The `<vendor>-sync` feature enables `pomelo-http/ureq`; the crate must also build
  `--no-default-features` (a dependent supplies its own `HttpClient`, no TLS stack).

**Honesty rule (the one a reviewer can't see from a diff):** a **missing block omits its CLI
flag** rather than shipping a no-op or a fabricated panel. Match flag names + layout outputs to
the other vendors **only where the block is actually implemented**. Examples: Finnhub ships
**no** `--include-delisted` (no clean dead-name feed) and Alpha Vantage writes **no**
`panels/in_sp500` (no real historical constituents) — the gap is documented, never faked.

**Docs:** write `docs/{vendor}-data-source.md` (block table → layout path → **gap → backtest
impact**; **no pricing advice**). Register it in
[`site/scripts/import-reference-docs.mjs`](site/scripts/import-reference-docs.mjs) and the
Starlight sidebar (`site/astro.config.mjs`), run `npm run import:docs`, and link it from
[`docs/data-sources.md`](docs/data-sources.md).

**CI:** `scripts/coverage.sh` gates **≥95% region coverage** workspace-wide — new adapter code
needs tests to clear it.

## Conventions

- **Conventional Commits** — release-plz derives versions/changelog from them. `feat:`→minor,
  `fix:`/`perf:`→patch; `docs`/`chore`/`refactor`/`test`/`ci` don't release.
- Don't hand-edit crate `version` fields — release-plz owns them.
- Published library crates: `yuzu-core`, `yuzu-research`, `pomelo-data`, `pomelo-s3`,
  `pomelo-http`, `pomelo-fmp`, `pomelo-eodhd`, `pomelo-alpha-vantage`, `pomelo-finnhub`,
  `pomelo-audit`, `lemon-lang` (the `lemon` crate is imported as `lemon`). The wasm/CLI/server
  crates are `publish = false`.
