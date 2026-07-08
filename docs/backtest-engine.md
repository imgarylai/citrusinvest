# citrusinvest — Rust strategy DSL + backtest engine

A Rust workspace for **US stocks**: build a strategy in the **Lemon** DSL,
evaluate it over price/fundamental data, and backtest it. This is the engine
behind citrusinvest — the repo root is the Cargo workspace.

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
crates/yuzu-data/          # native I/O layer: OHLCV + fundamentals loaders (see below)
crates/yuzu-source-s3/     # generic S3 ObjectSource for yuzu-data
crates/yuzu-wasm/          # browser/Worker bindings (run_backtest_json)
crates/yuzu-server/        # native backtest server core (source-agnostic handle_backtest)
crates/yuzu-cli/           # native batch binary
```

**Two families.** `lemon-*` is the **language** (text ⇄ Expr tree); `yuzu-*` is the
**engine** (evaluate → backtest, plus data loading, server, and CLI). `spec.rs` (the
`Expr` AST) lives in `lemon` and is re-exported by `yuzu-core` (`pub use lemon::spec`),
so both the parser and the evaluator agree on one tree shape.

---

## Data loader (yuzu-data)

`crates/yuzu-data/` is a **native** crate that reads per-symbol OHLCV price
files (plus fundamentals/industry) from disk or any `ObjectSource` implementation
and returns a `Panel` ready for the evaluator. It depends on `yuzu-core`;
`yuzu-core` never depends on it (keeping the WASM build I/O-free).

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
used by tests, the CLI, and offline development. The `yuzu-source-s3` crate ships a
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

Top-level re-exports (use as `yuzu_data::load_panel`, `yuzu_data::Field`,
`yuzu_data::OhlcvRow`, `yuzu_data::LocalSource`, `yuzu_data::ObjectSource`,
`yuzu_data::PRICES_DIR`):

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

> **Surface syntax is documented in [`lemon.md`](./lemon.md)** — the authoritative
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
| `hold_until(exit, …)`              | rank-priority rotation with `nstocks_limit` + optional stop_loss / take_profit / trail_stop (the one sequential loop) |
| `rebalance(freq)`                  | downsample to last obs per W / ME / QE period (or explicit dates)                                                     |
| `rank(pct, ascending)`             | cross-sectional rank (axis=1), average ties                                                                           |
| arithmetic / comparison / logical  | `+ - * /`; comparisons `> >= < <=` → `1`/`0`; logical `and` / `or`; scalar variants                                  |

**Surface-syntax notes** (see `lemon.md`): the DSL has **no** `==`, `!=`, `&`,
`|`, or `!` — logical AND/OR are the words `and` / `or`, and there is no equality
operator. `exit_when` and `quantile_row` are implemented as `Panel` ops
(golden-tested) but are **not** exposed as `Expr` AST variants or DSL surface
names — they are not callable from lemon.

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

Every op family is golden-tested — including the price-stop paths in
`hold_until` (`stop_loss` / `take_profit` / `trail_stop`).

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

**`BacktestConfig` defaults:** `fee_ratio = 0.0`, `tax_ratio = 0.0`,
`position_limit = 0.0` (uncapped), `slippage_ratio = 0.0`,
`initial_capital = 0.0` / `max_participation = 0.0` (liquidity cap off),
`delist_after = 0` (delisting handling off), `delist_haircut = 0.0`,
`benchmark_key = None`.

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
    "avg_exposure":        0.64,
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

### Deferred (not yet modeled)

These items are explicit scope cuts, not gaps:

- **Advanced cost semantics** (`touched_exit` / `retain_cost_when_rebalance` / `stop_trading_next_period`) — the engine currently uses the simplified model described above.
- **Volume-aware slippage** — `slippage_ratio` is a flat per-leg haircut; an impact model that scales with participation is future work.
- **Portfolio optimization** (mean-variance, risk parity, etc.).
- **Monthly / yearly metric tiers** (rolling windows, calendar buckets).
- **Walk-forward / parameter-grid tooling** — `yuzu-cli sweep` ranks hand-supplied variants; grid generation and in/out-of-sample splits are future work.

Per-trade **MAE / MFE** (maximum adverse / favorable excursion) and factor
**neutralization** (`neutralize` / `neutralize_industry` / `industry_rank` /
`groupby_category`) are now implemented and golden-tested — see the `mae`/`mfe`
trade fields above and `ops/neutralize.rs`.
