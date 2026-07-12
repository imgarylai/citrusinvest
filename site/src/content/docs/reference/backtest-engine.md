---
title: "Backtest engine"
editUrl: false
sourceFile: docs/backtest-engine.md
---

<!-- Imported from docs/backtest-engine.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
A Rust workspace for **US stocks**: build a strategy in the **Lemon** DSL,
evaluate it over price/fundamental data, and backtest it. This is the engine
behind citrusquant — the repo root is the Cargo workspace.

> **Why Rust?** The engine runs two ways from one codebase: in the browser/Worker
> via **WASM**, and as a native batch binary (large-scale backtests via Rayon).
> The behavior is pinned by golden tests.

---

## Workspace layout

```
Cargo.toml                 # workspace (members = crates/*)
crates/lemon/              # the Lemon DSL — human-writable text ⇄ JSON Expr tree
  src/
    spec.rs                # Expr AST (serde-deserializable strategy tree)
    dsl/
      lex.rs               # tokenizer
      parse.rs             # text → JSON Expr (Pratt; `let` = parse-time inlining)
      print.rs             # JSON Expr → text (flat; lossy — no let/comment reconstruction)
      ops.rs               # op vocabulary: DSL names ⇄ Expr tags + field layout
crates/lemon-wasm/         # Lemon parse/format over WASM (powers the web editor)
crates/yuzu-core/          # pure, I/O-free evaluator — re-exports `lemon::spec`
  src/
    panel.rs               # Panel type (dates × symbols f64 matrix) + shift
    align.rs               # align(a,b): union rows, intersect cols
    error.rs               # EngineError
    ops/                   # one file per op family: arith, indicators, ta,
                           #   cross_section, signals, rotation, rebalance,
                           #   neutralize, linalg
    eval.rs                # eval(Expr) + run_strategy(json) + run_backtest(...)
  tests/
    golden/                # committed *.json fixtures (expected outputs)
    golden_harness.rs      # load_golden / panel_from_json / assert_panel_eq
    golden_ops.rs          # per-op golden tests
    strategy_e2e.rs        # full spec → position matrix golden
    strategy_backtest_e2e.rs  # full spec → backtest report golden
crates/pomelo-data/        # native I/O layer: OHLCV + fundamentals loaders (see below)
crates/pomelo-s3/          # generic S3 ObjectSource for pomelo-data
crates/pomelo-audit/       # read-only data-quality audit of a data-layout tree (yuzu-cli data-audit)
crates/yuzu-wasm/          # browser/Worker bindings (run_backtest_json)
crates/yuzu-server/        # native backtest server core (source-agnostic handle_backtest)
crates/yuzu-cli/           # native batch binary
```

**Three families.** `lemon-*` is the **language** (text ⇄ Expr tree); `yuzu-*` is the
**engine** and the product apps around it (evaluate → backtest, wasm, server, CLI);
`pomelo-*` is **data engineering** (native I/O, storage backends, data sync). `spec.rs` (the
`Expr` AST) lives in `lemon` and is re-exported by `yuzu-core` (`pub use lemon::spec`),
so both the parser and the evaluator agree on one tree shape.

---

## Data loader (pomelo-data)

`crates/pomelo-data/` is a **native** crate that reads per-symbol OHLCV price
files (plus fundamentals/industry) from disk or any `ObjectSource` implementation
and returns a `Panel` ready for the evaluator. It depends on `yuzu-core`;
`yuzu-core` never depends on it (keeping the WASM build I/O-free).

> **Full input contract for BYO data** (directory tree, fundamentals columns,
> combined panels, series-name map, universe/PIT responsibilities):
> [`data-layout.md`](../reference/data-layout). The subsections below are a short prices
> recap for engine readers; prefer `data-layout.md` when wiring a data root.

### File contract

Each symbol's price history lives at:

```
prices/SYMBOL.csv.gz   # gzip-compressed CSV
```

The CSV inside has six columns, no quotes, header required:

```
day,adj_open,adj_high,adj_low,adj_close,volume
2024-01-02,9.5,11.0,9.0,10.0,1000
2024-01-03,10.1,11.5,9.8,10.8,1200
...
```

- `day` — `YYYY-MM-DD` string, sorted ascending (oldest first), one row per trading day.
- `adj_open`, `adj_high`, `adj_low`, `adj_close` — adjusted prices as decimal floats.
- `volume` — share volume as a decimal float.

When loaded into memory, the `Panel` stores dates as `i32 YYYYMMDD` (e.g. `20240102`).
The `load_panel()` function's `from` and `to` parameters also use `i32 YYYYMMDD`; the
loader automatically converts the CSV's `YYYY-MM-DD` strings to this integer format.

### `ObjectSource` trait

```rust
pub trait ObjectSource {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError>;
}
```

`key` is the relative path (`prices/AAPL.csv.gz`). Returning `Ok(None)` means the
file doesn't exist; `load_panel` treats a missing file as an all-NaN column. A
sibling `ObjectSink` trait (`fn put(&self, key, bytes)`) covers writes.

**`LocalSource`** resolves keys under a root directory on the local filesystem —
used by tests, the CLI, and offline development. The `pomelo-s3` crate ships a
generic S3-backed `ObjectSource` for the batch runner.

### `load_panel`

```rust
pub fn load_panel<S: ObjectSource + Sync>(
    source: &S,
    symbols: &[String],
    field: Field,
    from: i32,
    to: i32,
    dir: &str,
) -> Result<Panel, DataError>
```

- `field` — which OHLCV column to assemble into the Panel:
  `Field::AdjOpen`, `Field::AdjHigh`, `Field::AdjLow`, `Field::AdjClose`,
  or `Field::Volume`.
- `from` / `to` — inclusive date bounds in `YYYYMMDD`.
- `dir` — the key prefix under which per-symbol files live; defaults to
  `PRICES_DIR` (`"prices"`) at call sites.
- Returns a `Panel` whose rows are the **union** of all dates that appear in any
  symbol's file within `[from, to]`, and whose columns are `symbols` in the given order.
- Cells absent in a symbol's file become `NaN`; a symbol with no file at all becomes
  an all-NaN column. Symbols are fetched concurrently; a corrupt file is treated as
  missing rather than sinking the batch.

Top-level re-exports (use as `pomelo_data::load_panel`, `pomelo_data::Field`,
`pomelo_data::OhlcvRow`, `pomelo_data::LocalSource`, `pomelo_data::ObjectSource`,
`pomelo_data::PRICES_DIR`):

```rust
pub use csv_io::{Field, OhlcvRow};
pub use loader::{load_panel, PRICES_DIR};
pub use source::{LocalSource, ObjectSink, ObjectSource};
```

---

## The data model: `Panel`

One type carries everything. A `Panel` is a dense `f64` matrix with:

- `dates: Vec<i32>` — row index, `YYYYMMDD` (e.g. `20240102`), sorted ascending, unique.
- `symbols: Vec<String>` — column index (tickers).
- `data: Array2<f64>` — the values; **`NaN` means missing**.

Element-wise conventions:

- **Booleans** live in the same `f64` matrix as `1.0` (true) / `0.0` (false). `NaN` is falsy. Truthiness is `x == 1.0`.
- **Arithmetic** propagates `NaN`. **Comparisons** involving `NaN` yield `false` (`0.0`).
- **Binary ops align first**: rows = union of both date indices (sorted), columns = intersection of symbols (preserving the left operand's order). Cells absent in an operand become `NaN`. (`align.rs`.)

---

## DSL vocabulary

> **Surface syntax is documented in [`lemon.md`](../reference/lemon)** — the authoritative
> reference for how a strategy is *written*: lexical rules, operators and
> precedence, the `let`/call grammar, and the **complete op table** (DSL names,
> arguments, defaults). This section covers only the **engine-side semantics** of
> a few ops that need pinning down; don't duplicate the op list here.

Semantics are pinned by golden fixtures (committed expected outputs). The table
below names ops by their engine behavior; the DSL surface names map through
`lemon/src/dsl/ops.rs` (e.g. the `Average` op is written `sma` / `average`; the
`Rank` op is written `rank`).

| Op                                 | Meaning                                                                                                               |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `Average` (`sma`) `(n)`            | `rolling(n, min_periods=floor(n/2)).mean()` (min ≥ 1 obs)                                                             |
| `rise(n)` / `fall(n)`              | `self > self.shift(n)` / `self < self.shift(n)`                                                                       |
| `is_largest(n)` / `is_smallest(n)` | per-row top/bottom `n` over non-NaN cells; NaN never selected; ties break by column order (stable)                    |
| `is_entry` / `is_exit`             | rising / falling edge of a bool series (`shift` fills `false`)                                                        |
| `sustain(nwindow, nsatisfy)`       | `rolling(nwindow).sum() >= nsatisfy` over a bool frame                                                                |
| `hold_until(exit, …)`              | rank-priority rotation with `nstocks_limit` (the one sequential loop). Price stops are a NAV-loop config feature, not part of this op |
| `rebalance(freq)`                  | downsample to last obs per W / ME / QE / YE period (or explicit dates)                                                     |
| `rank(pct, ascending)`             | cross-sectional rank (axis=1), average ties                                                                           |
| `winsorize` / `zscore` / `bucket` / `demean` | per-row cross-section preprocess (clip / standardize / q-buckets / subtract mean)                           |
| arithmetic / comparison / logical  | `+ - * /`; comparisons `> >= < <=` → `1`/`0`; logical `and` / `or`; scalar variants                                  |

**Surface-syntax notes** (see `lemon.md`): the DSL has **no** `==`, `!=`, `&`,
`|`, or `!` — logical AND/OR/NOT are the words `and` / `or` / `not`, and there
is no equality operator. `normalize_row` scales each row to unit gross weight
(explicit portfolio weights, e.g. inverse-vol via `normalize_row(sig / std(close, 20))`).
`vol_target(w, close, target=0.10, n=63)` is the risk-managed overlay: it scales
each row's weights by `min(1, target / realized_vol)`, where `realized_vol` is
the annualized (×√252) rolling-`n` std of the **implied portfolio's** daily
returns over `close`; it deleverages only (never levers up), passes through
during warmup / `n<2` / `target≤0`, and — being I/O-free — takes the price panel
as an explicit argument rather than reading it from the NAV loop. Its rolling
window ends at the current row, so lag the weights (`shift`) for strictly-causal
sizing. `exit_when(entry, exit)` holds from an entry edge until exit;
`quantile_row(of, c)` returns a one-column per-row quantile panel (both are
lemon-callable).

OHLCV technical indicators (`atr`, `natr`, `cci`, `aroon`, `stoch`, `adx`/`±di`,
`obv`, `mfi`, `willr`, and `vwap` = rolling-`n` `Σ(tp·vol)/Σvol` over typical
price `(H+L+C)/3`) live in `ops/ta.rs`; the per-op reference is the pairing of
`lemon/src/spec.rs` (Expr fields) with `lemon/src/dsl/ops.rs` (DSL names), and the
author-facing table in `lemon.md`.

---

## Strategy spec: the `Expr` AST

A strategy is a **serializable JSON tree** (`spec.rs`, in the `lemon` crate), so the
WASM (browser/Worker) path and the native batch runner produce the same artifact.
It is what **Lemon** text
compiles to — authors write `close > sma(close, 2)`, the parser lowers it to this
tree. The evaluator (`yuzu-core/eval.rs`) walks the tree against an `EvalContext`
(numeric `panels: HashMap<String, Panel>` keyed by series name, e.g. `"close"`,
plus a `symbol → industry` map used by the neutralization/grouping ops).

```jsonc
// (close > sma2).hold_until(close < sma2, nstocks_limit=1)
{
  "op": "HoldUntil",
  "entry": {
    "op": "Gt",
    "l": { "op": "Data", "name": "close" },
    "r": { "op": "Average", "of": { "op": "Data", "name": "close" }, "n": 2 },
  },
  "exit": {
    "op": "Lt",
    "l": { "op": "Data", "name": "close" },
    "r": { "op": "Average", "of": { "op": "Data", "name": "close" }, "n": 2 },
  },
  "nstocks_limit": 1,
  "rank": null,
}
```

```rust
let result: Panel = yuzu_core::run_strategy(spec_json, &ctx)?;
// result is the boolean position matrix the backtest loop consumes.
```

---

## Testing: golden fixtures

Correctness is pinned by committed expected outputs. The golden fixtures are
committed expected outputs, captured once from a reference run and replayed
offline: the JSON files under `crates/yuzu-core/tests/golden/` hold outputs over a
tiny fixed dataset, and the Rust tests load them and assert equality
(`assert_panel_eq`: NaN matches NaN, else `|a-b| <= tol`). The fixtures are the
source of truth; regenerating means re-deriving from the same sample data.

Run the suite:

```bash
cargo test
```

Coverage (the run also executes the tests):

```bash
cargo llvm-cov --summary-only
```

Every op family is golden-tested. Execution-layer **stops** (`BacktestConfig::stops`)
are unit-tested in `backtest.rs` (touched / gapped / close-fill / take-profit /
trailing / short / re-entry-block).

> **Fixture note:** ops that change the row count (`rebalance`) or column count
> (`quantile_row`) write `expected_dates` / `expected_symbols`; the harness reads
> `<key>_dates` / `<key>_symbols` when present, else the shared `dates`/`symbols`.
> `assert_panel_eq` compares matrix shape + values, not the axis labels (the
> engine labels resampled rows by the last-observation date rather than the
> period-end — same partition, different label).

---

## Backtest

### Entry point

```rust
pub fn run_backtest(
    spec_json: &str,
    ctx: &HashMap<String, Panel>,
    price_key: &str,
    cfg: &BacktestConfig,
) -> Result<Report, EngineError>
```

Evaluates the strategy spec → position matrix, then runs the NAV loop against
`ctx[price_key]`, and wraps the result in a serializable `Report`. All in one call.

**Which series drives fills and returns — `price_key`.** The NAV loop uses a
**single** price panel for both daily returns and trade marks, selected by
`price_key` (the argument above — **not** a `BacktestConfig` field). It defaults
to `"close"` (close-to-close), so the default backtest and its goldens are
unchanged. Passing `"open"` prices the whole run off the open panel instead.

- **Server:** `BacktestRequest.price_key` (defaults to `"close"`); the panel is
  force-loaded even if the spec never references it.
- **CLI:** `yuzu-cli run|sweep|grid --price-key <open|high|low|close>` (default
  `close`); the close panel is always loaded (strategies usually reference
  `close`), plus the chosen execution panel when it differs.
- **Next-open execution.** There's one price series per run, so to *signal on
  close but fill on the next open*, lag the close-based signal one day and price
  off the open: `shift(signal, 1)` with `price_key = "open"`. (Signal and fill
  price differing within a *single* run would need a separate mark-vs-fill API —
  not part of this model.)

---

### Daily-equity NAV model

**Step 1 — align & forward-fill.**
`positions` and `prices` are aligned (union rows, intersect symbols). Each
column of the position matrix is forward-filled down rows so that a signal
emitted once stays active until revoked. Missing rows in the original position
panel become the previous non-NaN value, or 0.0 at the start.

**Step 2 — row-normalize weights.**
After forward-fill, every row is scaled so the book never exceeds 1:

```
total = max(sum(|w|), 1.0)
w[i]  = w[i] / total
```

Weights that already sum to ≤ 1 are left unchanged (fractional book is
allowed). The implicit remainder is cash (undeployed capital).

Then, if `position_limit > 0`, each weight is clamped to `±position_limit`
(sign-preserving per-position cap, leaving the residual in cash). `position_limit
= 0` (the default) disables the cap.

Then, if the **liquidity cap** is active (`initial_capital > 0`,
`max_participation > 0`, and a `volume` panel is present in the context), each
weight is further clamped so the implied dollar position stays within the
symbol's tradable dollar volume:

```
|w[c]| <= max_participation * price[c] * volume[c] / initial_capital
```

Cells with NaN dollar volume (missing volume/price data) pass through uncapped —
data gaps aren't liquidity. The cap is measured against `initial_capital`, not
compounded equity.

**Delisting.** When `delist_after > 0`, a symbol whose price is missing for
that many consecutive rows is treated as delisted on the row the streak is
confirmed: the position is written down by `delist_haircut` (shorts gain
symmetrically), the remainder moves to cash with **no trading cost** (a forced
exit is not a trade), and the symbol cannot be held or re-entered until prices
resume. The closed trade fills at the last valid price × `(1 − delist_haircut)`
with no exit-leg fees. `delist_after = 0` (the default) keeps the legacy
behavior — a dead position freezes at its last value forever, which
systematically hides delisting losses; set it (e.g. `10`) for honest results on
universes containing delisted names.

**Step 3 — NAV loop.**
Starting from `equity[0] = 1.0` (minus day-0 entry cost), for each subsequent
day:

1. Compute each asset's simple return from the price panel (`0.0` when a price
   is missing).
2. Compute the daily portfolio return `g = Σ w_prev[c] * ret[c]`.
3. Advance equity: `value *= (1 + g)`.
4. Drift each weight: `drift[c] = w_prev[c] * (1 + ret[c]) / (1 + g)`.
5. Rebalance to today's target weights and charge cost (only when the target
   differs from the drifted position — no-op cost when the same).

**Step 4 — rebalance cost.**
Cost is charged as a multiplicative hit on NAV:

```
turnover = Σ |target[c] - drift[c]|
sells    = Σ max(drift[c] - target[c], 0)
cost     = (fee_ratio + slippage_ratio) * turnover + tax_ratio * sells
value   *= (1 - cost)
```

`slippage_ratio` is a flat per-leg haircut on turnover — a crude stand-in for
spread/impact (e.g. `0.0005` = 5 bps per trade leg). Closed-trade net returns
carry it on both legs too.

**Square-root market impact** (`impact_coef`, requires `initial_capital > 0`
and a volume panel): on each rebalance, every traded cell additionally pays

```
participation[c] = |Δw[c]| * initial_capital / dollar_volume[c]   (capped at 1)
cost += |Δw[c]| * impact_coef * sqrt(participation[c])
```

so consuming more of a symbol's daily dollar volume costs progressively more.
A cell with missing or zero dollar volume contributes no impact (the flat
`slippage_ratio` still covers it). The flat component keeps its original
accumulation order, so `impact_coef = 0` (the default) reproduces the
flat-cost path bit-for-bit. Impact affects NAV only — per-trade returns are
price-relative and carry the flat components only.

**`BacktestConfig` defaults:** `fee_ratio = 0.0`, `tax_ratio = 0.0`,
`position_limit = 0.0` (uncapped), `slippage_ratio = 0.0`,
`initial_capital = 0.0` / `max_participation = 0.0` (liquidity cap off),
`impact_coef = 0.0` (square-root impact off),
`delist_after = 0` (delisting handling off), `delist_haircut = 0.0`,
`benchmark_key = None`, `bootstrap_samples = 0` (bootstrap off) /
`bootstrap_block = 0` (auto `⌊√n⌋`),
`live_performance_start = None` (no post-go-live segment block).

---

### Metric conventions

All metrics are standard (CAGR, Sharpe, Sortino, max drawdown, etc.) and
golden-tested.

| Metric               | Definition                                                  |
| -------------------- | ----------------------------------------------------------- |
| `total_return`       | `equity[-1] / equity[0] - 1`                                |
| `cagr`               | `(equity[-1] / equity[0]) ^ (1 / year_frac) - 1`            |
| `ann_volatility`     | `std(daily_returns, ddof=1) * sqrt(252)`                    |
| `sharpe`             | `mean(r) / std(r, ddof=1) * sqrt(252)`, rf = 0              |
| `sortino`            | `mean(r) / std(min(r[1:], 0), ddof=1) * sqrt(252)`, rf = 0  |
| `max_drawdown`       | `min(equity / cummax(equity) - 1)`                          |
| `calmar`             | `cagr / abs(max_drawdown)`                                  |
| `win_rate`           | `count(closed trades where ret > 0) / count(closed trades)` |
| `profit_factor`      | `sum(gains) / abs(sum(losses))` over closed trades          |
| `expectancy`         | `mean(ret)` over closed trades                              |
| `avg_holding_period` | `mean(period)` in trading days over closed trades           |
| `num_trades`           | count of closed trades                                                       |
| `avg_win` / `avg_loss` | `mean(ret)` over closed winners / losers (`avg_loss` is negative)            |
| `payoff_ratio`         | `avg_win / abs(avg_loss)`                                                    |
| `best_trade` / `worst_trade` | max / min `ret` over closed trades                                    |
| `max_consecutive_losses` | longest run of losing closed trades, ordered by `exit_date`                |
| `recovery_factor`      | `total_return / abs(max_drawdown)` (∞ when no drawdown)                      |
| `max_drawdown_duration` | longest run of consecutive rows below the running peak (trading days)       |
| `time_in_market`       | fraction of rows with gross exposure > 0                                     |
| `avg_exposure`         | mean per-day gross exposure (`Σ|weight|`)                                    |

**Distribution / tail & drawdown-shape metrics** (always emitted; over daily
returns unless noted):

| Metric          | Definition                                                                   |
| --------------- | ---------------------------------------------------------------------------- |
| `best_day` / `worst_day` | max / min single-day return                                         |
| `skew`          | population skewness `m3 / m2^1.5` (NaN if < 2 returns or zero variance)       |
| `kurtosis`      | population **excess** kurtosis `m4 / m2² − 3`                                 |
| `var_95`        | historical 5th-percentile daily return (linear interpolation); a loss is negative |
| `cvar_95`       | mean of daily returns at or below `var_95` (expected shortfall)              |
| `avg_drawdown`  | mean of the drawdown series (zeros at new highs included; ≤ 0)               |
| `ulcer_index`   | root-mean-square drawdown, as a **fraction** (not ×100), consistent with `max_drawdown` |

**Lookback returns** (emitted only when the backtest covers the window;
`skip_serializing_if` omits the rest): each is `equity_last / equity_anchor − 1`.

| Metric       | Anchor                                                                        |
| ------------ | ----------------------------------------------------------------------------- |
| `ytd`        | last equity point of the **prior calendar year**; omitted if the run never reaches a prior year |
| `one_year`   | last point on or before the same date one year earlier; omitted if history < 1y |
| `three_year` | same, three years earlier; omitted if history < 3y                            |

(Inception-to-date return is `total_return`, so it is not repeated here.)

**Benchmark-relative metrics** (emitted only when `benchmark_key` is set; the
benchmark series is forward-filled onto the report dates and rebased to 1.0 at
its first observation):

| Metric              | Definition                                                        |
| ------------------- | ----------------------------------------------------------------- |
| `benchmark_return`  | benchmark total return over its first/last observations           |
| `beta`              | `cov(r, b) / var(b)` over paired daily returns (ddof = 1)         |
| `alpha`             | annualized CAPM alpha, rf = 0: `(mean(r) − beta · mean(b)) · 252` |
| `excess_return`     | `total_return − benchmark_return`                                 |
| `tracking_error`    | `std(r − b, ddof = 1) · sqrt(252)`                                |
| `information_ratio` | `mean(r − b) / std(r − b, ddof = 1) · sqrt(252)`                  |

**Calendar & rolling series** (always emitted): `monthly_returns` /
`yearly_returns` are calendar buckets chained off each bucket's closing equity
(`[{"period": "2024-01", "ret": 0.021}, …]`); `rolling_sharpe` /
`rolling_volatility` are 252-day rolling series aligned to `dates` (NaN → JSON
`null` during warmup).

**Bootstrap bands** (only when `bootstrap_samples > 0`): a circular block
bootstrap over the daily returns (block length `bootstrap_block`, `0` = auto
`⌊√n⌋`; deterministic fixed-seed PRNG) rebuilds `bootstrap_samples` synthetic
equity curves and reports p05/p50/p95 for Sharpe, CAGR, and max drawdown under
`report.bootstrap`.

**Live segment** (only when `live_performance_start` is set): equity-curve
metrics for the slice starting at the first backtest date **on or after** that
`YYYYMMDD`, reported under `report.live`. It carries only equity-derived
figures (`total_return`, `cagr`, `ann_volatility`, `sharpe`, `sortino`,
`max_drawdown`, `calmar`) plus the resolved `start` date and `days` count.
Every figure normalizes by the segment's own first equity point, so the block
is identical whether or not the segment is rebased to 1.0 (it is, in effect) —
it describes the post-go-live stretch in isolation, independent of the
pre-live NAV. Trade-level stats are intentionally omitted, since a trade can
straddle the live date. The full-sample equity curve and metrics are
unaffected; this is a report-only view. A live date past the last backtest row
omits the block.

Global conventions: annualization factor **252**, risk-free rate **0**,
`std` uses **ddof = 1**.
`year_frac = (end − start).total_seconds() / 31_557_600` (Julian year).

When there are no closed trades, trade-level metrics return `NaN` (`num_trades`
and `max_consecutive_losses` return `0`). `profit_factor` returns `∞` when
losses = 0; `recovery_factor` and `calmar` return `∞` when `max_drawdown = 0`.
The trade-level/exposure metrics beyond `avg_holding_period` are hand-verified
unit tests; `max_drawdown_duration`'s boundary excludes
the recovery row (counts rows strictly underwater).

---

### Report JSON contract

`Report` is `serde::Serialize`; the engine emits numbers only — the frontend
renders charts and tables.

```jsonc
{
  "dates":    [20240102, 20240103, ...],   // i32 YYYYMMDD, one per price row
  "equity":   [1.0, 1.02, ...],            // NAV, same length as dates
  "drawdown": [0.0, -0.01, ...],           // fraction below peak, same length
  "benchmark": [1.0, 1.01, ...],           // rebased benchmark curve — only when benchmark_key is set
  "monthly_returns": [{ "period": "2024-01", "ret": 0.021 }, ...],
  "yearly_returns":  [{ "period": "2024", "ret": 0.18 }, ...],
  "rolling_sharpe":     [null, ..., 1.2],  // 252-day window; null during warmup
  "rolling_volatility": [null, ..., 0.14],
  "bootstrap": {                           // only when bootstrap_samples > 0
    "n_samples": 1000, "block_len": 15,
    "sharpe":       { "p05": 0.4, "p50": 1.1, "p95": 1.9 },
    "cagr":         { "p05": -0.02, "p50": 0.12, "p95": 0.31 },
    "max_drawdown": { "p05": -0.28, "p50": -0.14, "p95": -0.07 }
  },
  "live": {                                  // only when live_performance_start is set
    "start": 20240601,                       // first backtest date on/after the requested day
    "days": 148,                             // equity points in the segment
    "total_return": 0.09, "cagr": 0.19, "ann_volatility": 0.13,
    "sharpe": 1.31, "sortino": 1.92, "max_drawdown": -0.06, "calmar": 3.17
  },
  "trades": [
    {
      "symbol":      "AAPL",
      "entry_date":  20240102,
      "exit_date":   20240115,   // null for open (mark-to-market) trades
      "ret":         0.043,      // net return (fees deducted for closed; MTM for open)
      "period":      9,          // trading days held
      "mae":        -0.041,      // worst unrealized excursion vs entry (null if no high/low)
      "mfe":         0.112,      // best unrealized excursion vs entry (null if no high/low)
      "entry_price": 182.4,      // price-panel fill on entry_date (null if missing)
      "exit_price":  190.2,      // fill on exit_date, or last-valid × (1 - delist_haircut)
                                 //   for a delisting exit; omitted for open trades
      "side":        "long"      // "long" / "short", from the sign of the entry weight
    }
  ],
  "metrics": {
    "total_return":        0.18,
    "cagr":                0.22,
    "ann_volatility":      0.14,
    "sharpe":              1.45,
    "sortino":             2.10,
    "max_drawdown":       -0.08,
    "calmar":              2.75,
    "win_rate":            0.60,
    "profit_factor":       2.40,
    "expectancy":          0.031,
    "avg_holding_period":  7.2,
    "num_trades":          42,
    "avg_win":             0.052,
    "avg_loss":           -0.021,
    "payoff_ratio":        2.48,
    "best_trade":          0.31,
    "worst_trade":        -0.12,
    "max_consecutive_losses": 3,
    "recovery_factor":     2.25,
    "max_drawdown_duration": 34,
    "time_in_market":      0.78,
    "avg_exposure":        0.64,
    "best_day":            0.041,
    "worst_day":          -0.038,
    "skew":               -0.32,
    "kurtosis":            2.10,
    "var_95":             -0.021,
    "cvar_95":            -0.030,
    "avg_drawdown":       -0.018,
    "ulcer_index":         0.043,
    "ytd":                 0.07,    // lookbacks: only when the window is covered
    "one_year":            0.15,
    "three_year":          0.62,
    // only when benchmark_key is set:
    "benchmark_return":    0.11,
    "alpha":               0.06,
    "beta":                0.85,
    "excess_return":       0.07,
    "tracking_error":      0.09,
    "information_ratio":   0.78
  }
}
```

---

### Testing basis

- **Metrics** (`metrics.rs`) — golden-tested via `tests/golden_metrics.rs`. The
  expected values were derived once independently and committed as
  fixtures; the Rust values must match within floating-point tolerance. (The
  reference tooling is not part of this repo — the fixtures are the source of truth.)
- **NAV loop** (`backtest.rs`) — pinned by an **independent re-derivation**
  of the same NAV math, captured once and replayed via committed fixtures
  (`tests/golden_backtest.rs`), the offline baseline.
- **Report round-trip** (`report.rs`) — `serde_json::to_string` is asserted to
  produce valid JSON containing the expected field names.

---

### Parameter grids & walk-forward (yuzu-cli)

The batch CLI can expand a **parameter grid** and run research workflows over
it. A grid file is a spec template plus value lists — any JSON string equal to
`"$name"` inside the spec is substituted:

```jsonc
{
  "spec": { "op": "IsLargest", "of": { "op": "Data", "name": "close" }, "n": "$n" },
  "params": { "n": [10, 20, 50] }
}
```

- `yuzu-cli grid --data D --grid g.json [--sort sharpe] [--top 20]` — expand
  the cartesian product (variant names like `"n=20"`) and emit a ranked
  leaderboard (same output as `sweep`).
- `yuzu-cli walkforward --data D --grid g.json --train-days 504 --test-days 126
  [--warmup-days N] [--sort sharpe]` — roll an in-sample/out-of-sample window
  over the trading-day axis: pick the best variant on each train slice, run it
  on the following test slice, and chain the out-of-sample equity into one
  curve. Output: per-window selection table (`chosen`, `in_sample_metric`,
  `oos_return`) plus the stitched OOS `equity`/`dates` and summary metrics.
  Indicators **warm up on the rows before each window** (default `--warmup-days`
  auto = the largest window argument in any variant) while P&L is counted only
  inside the window, and each test segment prices the boundary-day return from
  the previous window's last close — so long-lookback variants aren't
  handicapped in selection and the stitched curve doesn't drop returns at
  window seams. The very first train window has no earlier data and still
  starts cold. **Holdings carry across seams:** each test segment starts from
  the previous segment's terminal book (keyed by symbol), so a seam that keeps
  the same names pays turnover only on the *difference* rather than a full
  re-entry. With zero fees/slippage this changes nothing; it only matters under
  costs, where the old flat-restart biased walk-forward against stable-holding
  strategies. (Engine hook: `backtest::run_with_initial(..., initial_weights)`
  plus `BacktestRun::terminal_weights`.)

- `yuzu-cli lookahead --data D --spec s.json [--shift-days 1] [--profile]` —
  run the strategy as-is and again with the position matrix lagged (signals
  executed late), and report both legs' metrics plus the deltas. A clearly
  positive baseline (`sharpe > 0.5`) that loses more than half its Sharpe
  under the lag is flagged `suspicious` — its edge lives in same-close
  execution or same-day data it couldn't have had. `--profile` runs the full
  decay ladder (shifts 1, 2, 3, 5, 10, 21) in one pass; the curve's SHAPE is
  the diagnosis: a cliff at shift 1 = same-close execution dependence, smooth
  decay = genuinely fast alpha, performance that holds until shift ≈ N then
  drops = data stamped ~N days ahead of its real publication date. Signals are
  evaluated once — extra levels only cost extra NAV loops.

  Note the DSL-level counterpart: `shift(expr, n)` lets a strategy bake the
  lag in directly — `shift(<signal>, 1)` is next-day execution, and
  `shift(pe, 21)` simulates a realistic one-month publication delay on a
  fundamental series.

All `BacktestConfig` flags (fees, slippage, liquidity cap, delisting,
benchmark, bootstrap) apply to these commands too.

### Deferred (not yet modeled)

These items are explicit scope cuts, not gaps:

- **Advanced cost semantics** (`retain_cost_when_rebalance` / `stop_trading_next_period`) — two remaining opt-in NAV-loop flags; the simplified cost model is otherwise as described above. (`touched_exit` shipped — see **Execution-layer stops** below.)
- **Portfolio optimization** (mean-variance, risk parity, etc.).

### Execution-layer stops (`BacktestConfig::stops`)

Stop-loss / take-profit / trailing stops are an **execution** feature, not a
strategy op — the NAV loop applies them to *any* position book, tracking each
holding from its entry price. Off by default (`StopConfig` all-`INF` sentinels),
so an unset config leaves every equity curve unchanged.

- **Trigger & fill** (`StopFill`): `Touched` (default) triggers on the intraday
  range and fills at the stop level, or at the day's **open** when the bar
  gapped through it (`min(open, level)` long / `max` short — a worse-than-stop
  fill you couldn't avoid). `Close` triggers on the close and fills there (an
  "end-of-day rule" style). A same-day stop-loss + take-profit double-touch
  assumes the stop-loss first (conservative; unavoidable without intraday data).
- **Trailing** keys off the peak return established on *prior* days (a wide
  up-day can't self-trip), and arms only after `trail_stop_activation`.
- **After a stop** the name goes to cash and re-entry is blocked until the
  position signal drops and re-adds it — so a stopped `hold_until` slot refills
  only at the next rebalance (more realistic than same-day refill).
- **Needs OHLC**: `Touched` reads open/high/low. `run_backtest` picks up
  `open`/`high`/`low` panels from the context. Surfaces that expose the knobs and
  load the panels automatically: the **CLI** (`--stop-loss` / `--take-profit` /
  `--trail-stop` / `--trail-stop-activation` / `--stop-fill`), the **server**
  request (`stop_loss` / … / `stop_fill`, flat fields), and the **WASM** request
  (`config.stops.{stop_loss,…,fill}`). All default to off.

Per-trade **MAE / MFE** (maximum adverse / favorable excursion) and factor
**neutralization** (`neutralize` / `neutralize_industry` / `industry_rank` /
`cap_industry` / `groupby_category`) are now implemented and golden-tested — see
the `mae`/`mfe` trade fields above and `ops/neutralize.rs`. `cap_industry(w,
max_weight=0.3)` bounds each industry's **gross** weight (Σ|w|) per row, scaling
the over-cap industry's names down proportionally (sign-preserving) and leaving
the freed weight as cash — the NAV loop's row-normalize takes the book from
there. Symbols with no industry share the single "其他" bucket, so an empty
industry map caps total gross exposure; `max_weight <= 0` is a no-op.

