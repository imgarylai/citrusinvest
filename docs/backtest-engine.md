# backtest-engine — Rust strategy DSL + backtest core

A Rust workspace at `packages/backtest-engine/` for **US stocks**: build a
strategy in the **Lemon** DSL, evaluate it over price/fundamental data, and
backtest it.

> **Why Rust, not the existing TS quant layer?** The engine must run two ways from
> one codebase: in the browser/Worker via **WASM**, and as a native batch binary on
> AWS (large-scale backtests via Rayon). The behavior is pinned by golden tests.

Full design rationale: [`docs/superpowers/specs/2026-06-18-rust-backtest-engine-design.md`](./superpowers/specs/2026-06-18-rust-backtest-engine-design.md).
This page is the living overview; the spec is the frozen decision record.

---

## Workspace layout

```
packages/backtest-engine/
  Cargo.toml                 # workspace (members = crates/*)
  crates/lemon/              # the Lemon DSL — human-writable text ⇄ JSON Expr tree
    src/
      spec.rs                # Expr AST (serde-deserializable strategy tree)
      dsl/
        lex.rs               # tokenizer
        parse.rs             # text → JSON Expr (Pratt; `let` = parse-time inlining)
        print.rs             # JSON Expr → text (flat; lossy — no let/comment reconstruction)
        ops.rs               # op vocabulary: DSL names ⇄ Expr tags + field layout
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
  crates/yuzu-data/          # native-only OHLCV loader (Phase 1b-A — see below)
  crates/yuzu-wasm/          # browser/Worker bindings (run_backtest_json)
  crates/yuzu-cli/           # native batch binary
  crates/lemon-wasm/         # browser Lemon parse/format (powers the web editor)
```

**Two families.** `lemon-*` is the **language** (text ⇄ Expr tree); `yuzu-*` is the
**engine** (evaluate → backtest, plus the native data loader). `spec.rs` (the `Expr`
AST) lives in `lemon` and is re-exported by `yuzu-core` (`pub use lemon::spec`), so
both the parser and the evaluator agree on one tree shape. The user-facing language
guide is at `/docs/lemon`; this page is the engine internals.

---

## Data loader (Phase 1b-A, yuzu-data)

`crates/yuzu-data/` is a **native-only** crate that reads per-symbol OHLCV price
files from disk (or any `ObjectSource` implementation) and returns a `Panel` ready for
the evaluator. It depends on `yuzu-core`; `yuzu-core` never depends on it (keeping
the WASM build I/O-free).

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
file doesn't exist; `load_panel` treats a missing file as an all-NaN column.

**`LocalSource`** is the only implementation shipped in this crate. It resolves keys
under a root directory on the local filesystem — used by tests, the CLI, and offline
development. An R2/S3 adapter is deferred to the batch runner (Phase 1d).

### `load_panel`

```rust
pub fn load_panel<S: ObjectSource>(
    source: &S,
    symbols: &[String],
    field: Field,
    from: i32,
    to: i32,
) -> Result<Panel, DataError>
```

- `field` — which OHLCV column to assemble into the Panel:
  `Field::AdjOpen`, `Field::AdjHigh`, `Field::AdjLow`, `Field::AdjClose`,
  or `Field::Volume`.
- `from` / `to` — inclusive date bounds in `YYYYMMDD`.
- Returns a `Panel` whose rows are the **union** of all dates that appear in any
  symbol's file within `[from, to]`, and whose columns are `symbols` in the given order.
- Cells absent in a symbol's file become `NaN`; a symbol with no file at all becomes
  an all-NaN column.

Top-level re-exports (use as `yuzu_data::load_panel`, `yuzu_data::Field`,
`yuzu_data::OhlcvRow`, `yuzu_data::LocalSource`, `yuzu_data::ObjectSource`):

```rust
pub use csv_io::{Field, OhlcvRow};
pub use loader::load_panel;
pub use source::{LocalSource, ObjectSource};
```

---

## The data model: `Panel`

One type carries everything. A `Panel` is a dense `f64` matrix with:

- `dates: Vec<i32>` — row index, `YYYYMMDD` (e.g. `20240102`), sorted ascending, unique.
- `symbols: Vec<String>` — column index (tickers).
- `data: Array2<f64>` — the values; **`NaN` means missing**.

Conventions that match numpy/pandas:

- **Booleans** live in the same `f64` matrix as `1.0` (true) / `0.0` (false). `NaN` is falsy. Truthiness is `x == 1.0`.
- **Arithmetic** propagates `NaN`. **Comparisons** involving `NaN` yield `false` (`0.0`).
- **Binary ops align first**: rows = union of both date indices (sorted), columns = intersection of symbols (preserving the left operand's order). Cells absent in an operand become `NaN`. (`align.rs`.)

---

## DSL vocabulary (Phase 1a-1)

Semantics are pinned by golden fixtures (committed expected outputs). The table below names ops
by their semantics; their **Lemon surface names** (e.g. `average` → `sma`/`average`,
`rank_cs` → `rank`) and call signatures are at `/docs/lemon`.

| Op                                 | Meaning                                                                                                               |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `average(n)`                       | `rolling(n, min_periods=floor(n/2)).mean()` (min ≥ 1 obs)                                                             |
| `rise(n)` / `fall(n)`              | `self > self.shift(n)` / `self < self.shift(n)`                                                                       |
| `is_largest(n)` / `is_smallest(n)` | per-row top/bottom `n` over non-NaN cells; NaN never selected; ties break by column order (stable)                    |
| `is_entry` / `is_exit`             | rising / falling edge of a bool series (`shift` fills `false`)                                                        |
| `sustain(nwindow, nsatisfy)`       | `rolling(nwindow).sum() >= nsatisfy` over a bool frame                                                                |
| `exit_when(exit)`                  | entry/exit forward-fill state machine                                                                                 |
| `hold_until(exit, …)`              | rank-priority rotation with `nstocks_limit` + optional stop_loss / take_profit / trail_stop (the one sequential loop) |
| `rebalance(freq)`                  | downsample to last obs per W / ME / QE period (or explicit dates)                                                     |
| `rank_cs(pct, ascending)`          | cross-sectional rank (axis=1), average ties                                                                           |
| `quantile_row(c)`                  | per-row quantile across columns, linear interpolation                                                                 |
| arithmetic / comparison / logical  | `+ - * /`, `> >= < <= == !=`, `& \| !`, scalar variants                                                               |

`exit_when` is implemented but not yet exposed in the `Expr` AST.

OHLCV technical indicators (`atr`, `natr`, `cci`, `aroon`, `stoch`, `adx`/`±di`,
`obv`, `mfi`, `willr`, and `vwap` = rolling-`n` `Σ(tp·vol)/Σvol` over typical
price `(H+L+C)/3`) live in `ops/ta.rs`; the full per-op reference is generated from
`lemon/src/spec.rs` (fields) + `lemon/src/dsl/ops.rs` (DSL names) into the web app's
op-reference table (`pnpm cli gen:op-docs`).

---

## Strategy spec: the `Expr` AST

A strategy is a **serializable JSON tree** (`spec.rs`, in the `lemon` crate), so the
website and the batch runner produce the same artifact. It is what **Lemon** text
compiles to — authors write `close > sma(close, 2)`, the parser lowers it to this
tree. The evaluator (`yuzu-core/eval.rs`) walks the tree against a data context
(`HashMap<String, Panel>` keyed by series name, e.g. `"close"`).

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
cargo test --manifest-path packages/backtest-engine/Cargo.toml
```

Coverage (CI gates at `--fail-under-lines 90`; the run also executes the tests):

```bash
cargo llvm-cov --manifest-path packages/backtest-engine/Cargo.toml --summary-only
```

Every op family is golden-tested — including the price-stop paths in
`hold_until` (`stop_loss` / `take_profit` / `trail_stop`).

> **Fixture note:** ops that change the row count (`rebalance`) or column count
> (`quantile_row`) write `expected_dates` / `expected_symbols`; the harness reads
> `<key>_dates` / `<key>_symbols` when present, else the shared `dates`/`symbols`.
> `assert_panel_eq` compares matrix shape + values, not the axis labels (pandas
> labels resampled rows by period-end; the engine keeps the last-observation date
> — same partition, different label).

---

## Backtest (Phase 1a-2)

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
cost     = fee_ratio * turnover + tax_ratio * sells
value   *= (1 - cost)
```

No slippage model.

**`BacktestConfig` defaults:** `fee_ratio = 0.0`, `tax_ratio = 0.0`.

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

Global conventions: annualization factor **252**, risk-free rate **0**,
`std` uses **ddof = 1**.
`year_frac = (end − start).total_seconds() / 31_557_600` (Julian year).

When there are no closed trades, trade-level metrics return `NaN` (`num_trades`
and `max_consecutive_losses` return `0`). `profit_factor` returns `∞` when
losses = 0; `recovery_factor` and `calmar` return `∞` when `max_drawdown = 0`.
The trade-level/exposure metrics beyond `avg_holding_period` are hand-verified
unit tests (not in `ffn_core.py`); `max_drawdown_duration`'s boundary excludes
the recovery row (counts rows strictly underwater).

---

### Report JSON contract

`Report` is `serde::Serialize`; the engine emits numbers only — the frontend
(Phase 1c) renders charts and tables.

```jsonc
{
  "dates":    [20240102, 20240103, ...],   // i32 YYYYMMDD, one per price row
  "equity":   [1.0, 1.02, ...],            // NAV, same length as dates
  "drawdown": [0.0, -0.01, ...],           // fraction below peak, same length
  "trades": [
    {
      "symbol":     "AAPL",
      "entry_date": 20240102,
      "exit_date":  20240115,   // null for open (mark-to-market) trades
      "ret":        0.043,      // net return (fees deducted for closed; MTM for open)
      "period":     9,          // trading days held
      "mae":       -0.041,      // worst unrealized excursion vs entry (null if no high/low)
      "mfe":        0.112       // best unrealized excursion vs entry (null if no high/low)
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
    "avg_exposure":        0.64
  }
}
```

---

### Testing basis

- **Metrics** (`metrics.rs`) — golden-tested against `ffn_core.py` via
  `tests/golden_metrics.rs`. A small equity curve and trade list are run
  through both Python and Rust; the Rust values must match within floating-point
  tolerance.
- **NAV loop** (`backtest.rs`) — pinned by an **independent pandas reference**: a
  pure-Python re-derivation of the same NAV math, replayed via committed fixtures
  (`tests/golden_backtest.rs`), the offline CI baseline.
- **Report round-trip** (`report.rs`) — `serde_json::to_string` is asserted to
  produce valid JSON containing the expected field names.

---

### Deferred (out of scope for Phase 1a-2)

These items are explicit scope cuts, not gaps:

- **Per-trade MAE / MFE** (maximum adverse / favorable excursion).
- **Advanced cost semantics** (`touched_exit` / `retain_cost_when_rebalance` / `stop_trading_next_period`) — the engine currently uses the simplified model described above.
- **Portfolio optimization** (mean-variance, risk parity, etc.).
- **Monthly / yearly metric tiers** (rolling windows, calendar buckets).
- **Alpha / beta** — requires a benchmark series; deferred to Phase 1a-3 where the benchmark data source is decided.

---

## Phasing

This crate covers **Phase 1a-1** (DSL → position matrix) and **Phase 1a-2**
(backtest loop + metrics → report). What follows, each its own plan:

- **1a-3** — factor neutralization + sector panel (+ alpha/beta with a benchmark)
- **1b** — data ingestion (two-tier D1 hot + R2 bulk, Cloudflare Workflows)
- **1c** — WASM build + web `/backtest` route (charts rendered on the frontend)
- **1d** — native CLI + AWS Rayon batch

Active plan: [`docs/superpowers/plans/2026-06-18-backtest-engine-core-dsl.md`](./superpowers/plans/2026-06-18-backtest-engine-core-dsl.md).
