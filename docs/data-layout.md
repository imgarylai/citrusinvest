# Input data layout

This is the **canonical contract** for data you feed into citrusquant.
The engine does **not** ship market data. You bring your own files (or any
[`ObjectSource`](../crates/pomelo-data/src/source.rs) that serves the same keys).

`yuzu-core` only sees in-memory [`Panel`](backtest-engine.md#the-data-model-panel)
values. The on-disk layout below is what **`pomelo-data`** reads and what
**`yuzu-cli`** / **`yuzu-server`** expect under a data root (local directory or
S3-compatible object store via `pomelo-s3`).

For NAV / metrics / strategy semantics, see [`backtest-engine.md`](backtest-engine.md).
For strategy syntax, see [`lemon.md`](lemon.md).
If you load data from an **FMP Starter**-class key: which library features you
can honestly backtest (and which are blocked by missing panels) is documented in
[`fmp-data-source.md`](fmp-data-source.md) — feature/series gaps, not a plan
comparison table.

---

## What you must provide

| Responsibility | Who |
|----------------|-----|
| Prices, fundamentals, optional membership panels | **You** (or your pipeline) |
| Symbol list for a run (`symbols` / CLI universe) | **You** (request or files present under `prices/`) |
| Point-in-time index membership (e.g. “in S&P 500 that day”) | **You** — supply a 0/1 series or filter `symbols` yourself |
| Sector map for industry ops | **You** — optional CSV (see [Industry map](#7-industry-map)) |
| Evaluating lemon → positions → NAV | Engine |

---

## 1. Directory tree

Default key prefixes (override with env on the server: `YUZU_PRICES_DIR`,
`YUZU_FUNDAMENTALS_DIR`, `YUZU_PANELS_DIR`):

```text
<data-root>/
  prices/              # per-symbol OHLCV (required for price strategies)
    AAPL.csv.gz
    MSFT.csv.gz
  fundamentals/        # per-symbol factors (optional)
    AAPL.csv.gz
  panels/              # wide combined series (optional fast path)
    close.csv.gz
    pe.csv.gz
  tracked/             # optional; sector snapshot for industry ops
    universe.csv.gz    # any name; see industry section
```

| Path | Required? | Role |
|------|-----------|------|
| `prices/` | Yes for CLI / price-based runs | Per-symbol OHLCV archives |
| `fundamentals/` | No | Per-symbol fundamental columns |
| `panels/` | No | One wide file per series (preferred when present) |
| `tracked/` | No | Symbol → sector (and optional market cap) |

Object keys under S3 are the same relative paths (`prices/AAPL.csv.gz`, …).

**CLI note:** `yuzu-cli --data <root>` currently loads **close** (and
**volume** / a **benchmark** symbol when those config flags need them) from
`prices/`. It does not auto-load fundamentals or industry. Use `yuzu-server`,
or build an `EvalContext` in library code, for full series sets.

**Server note:** `yuzu-server` loads only series referenced by the strategy
`Data` nodes (plus always `high`/`low` for MAE/MFE, and `volume` when liquidity
or impact knobs are on). See [Series name map](#3-series-names-lemon--files).

---

## 2. File encodings

Loaders detect format from **content**, not only the extension:

| Format | Magic / notes | Typical key suffix |
|--------|---------------|--------------------|
| gzip CSV | `1f 8b` | `.csv.gz` (default write format) |
| plain CSV | UTF-8 text | `.csv` |
| Apache Parquet | `PAR1` | `.parquet` (read-only; needs `pomelo-data` `parquet` feature) |

Probe order per object: `.csv.gz` → `.parquet` (if feature enabled) → `.csv`.

CSV rules (prices and fundamentals):

- No quoting required; simple comma-separated values.
- Header row required.
- Dates in the `day` column are `YYYY-MM-DD` strings in CSV.
- In memory / API bounds (`from` / `to`), dates are packed `i32` **YYYYMMDD**
  (e.g. `20240102`).
- Empty numeric cells and the token `NaN` become floating-point NaN.
- Rows should be **oldest first**.

Parquet: column names match CSV (`day`, `adj_close`, `pe`, …; combined panels
use one column per symbol). The `day` column may be integer YYYYMMDD, a
`YYYY-MM-DD` string, or a logical date type.

---

## 3. Series names (lemon ↔ files)

Bare identifiers in lemon (e.g. `close`, `pe`) become `Data { name }` leaves.
Loaders map those names as follows.

### Price series

| Series name in lemon / panels map | Column in `prices/{SYMBOL}.*` |
|-----------------------------------|-------------------------------|
| `close` | `adj_close` |
| `open` | `adj_open` |
| `high` | `adj_high` |
| `low` | `adj_low` |
| `volume` | `volume` |

Default `price_key` for the NAV loop is `"close"`.

### Fundamental series (per-symbol files)

Column order in `fundamentals/{SYMBOL}.*` (after `day`):

```text
pe, ps, pb, roe, net_margin, debt_to_equity, market_cap, gross_margin,
receivables_turnover, debt_to_assets, revenue, revenue_growth, eps_growth,
operating_income_growth, net_income_growth, gross_profit_growth, report_event
```

| Name | Meaning |
|------|---------|
| `pe` … `gross_profit_growth` | Factor values (see list above). Dense / forward-filled is the usual convention so every trading day has a row. |
| `report_event` | `1.0` on a day a new report was disclosed, else `0.0`. Missing → NaN (treated as “no event” by the engine’s truthiness). |

These names are the `Data` series names — keep them snake_case and in this set
if you use `yuzu-server`’s automatic loaders.

### Snapshot factor panels (combined only)

These names are recognized as fundamental-side series for routing, but they are
**not** columns of the per-symbol fundamentals CSV. Supply them as wide files
under `panels/`:

```text
piotroski_score, altman_z, fcf_yield, pe_industry_pctile,
analyst_upside_pct, consensus_rating
```

### Custom series

Any other name (e.g. a membership mask `in_sp500`) is a normal panel in
`EvalContext.panels` when **you** insert it (library / WASM path).

`yuzu-server` only auto-loads known price + fundamental series names. Custom
series are skipped at load time; if the strategy still references them,
`run_backtest` fails with an unknown-series error. To use custom series today,
build the context yourself or extend the server loader.

---

## 4. Per-symbol prices — `prices/{SYMBOL}.*`

```text
prices/AAPL.csv.gz
```

CSV header and sample:

```csv
day,adj_open,adj_high,adj_low,adj_close,volume
2024-01-02,9.5,11.0,9.0,10.0,1000
2024-01-03,10.1,11.5,9.8,10.8,1200
```

| Column | Type | Notes |
|--------|------|--------|
| `day` | `YYYY-MM-DD` | One row per trading day, ascending |
| `adj_open` … `adj_close` | float | Split/dividend-adjusted prices |
| `volume` | float | Share volume |

Missing symbol file → all-NaN column for that symbol when assembling a panel.
Corrupt file is treated like missing (does not abort the whole batch).

`yuzu-cli` discovers the universe by listing files under `prices/` (suffixes
`.csv.gz`, `.parquet`, `.csv`).

---

## 5. Per-symbol fundamentals — `fundamentals/{SYMBOL}.*`

```text
fundamentals/AAPL.csv.gz
```

Header (single line):

```csv
day,pe,ps,pb,roe,net_margin,debt_to_equity,market_cap,gross_margin,receivables_turnover,debt_to_assets,revenue,revenue_growth,eps_growth,operating_income_growth,net_income_growth,gross_profit_growth,report_event
```

Example row:

```csv
2024-01-02,28.5,7.1,45.0,0.15,0.22,1.4,2.8e12,0.45,8.0,0.3,1.2e11,0.05,0.08,0.06,0.07,0.04,0.0
```

Same date and NaN conventions as prices. Column order must match
`FUNDAMENTAL_FIELDS` + trailing `report_event` in
[`crates/pomelo-data/src/fundamentals.rs`](../crates/pomelo-data/src/fundamentals.rs).

---

## 6. Combined panels — `panels/{series}.*`

Optional **wide** files: one object per series for the whole universe (one GET
instead of N per-symbol reads).

```text
panels/close.csv.gz
panels/pe.csv.gz
```

CSV shape:

```csv
day,AAPL,MSFT,GOOGL
2024-01-02,10.0,20.0,15.0
2024-01-03,10.5,,15.2
```

- Header: `day`, then symbol tickers (any order; loaders reindex to the
  requested `symbols` list).
- Empty cell → NaN.
- Symbols requested but absent from the header → NaN column.

**Load order in `yuzu-server`:** try `panels/{name}` first; if the combined
object is **absent**, fall back to per-symbol `prices/` or `fundamentals/`.
A symbol missing from an **existing** combined file is NaN until you rebuild —
the server does not merge per-symbol rows into a partial combined file.

Rebuild helper (native): `rebuild_combined_panels` in `pomelo-data` (also exposed
as the server rebuild path) writes gzip CSV under `panels/` from per-symbol
archives. Snapshot factor names listed above are **not** produced by that
rebuild; supply those panels yourself.

---

## 7. Industry map

Industry ops (`neutralize_industry`, `industry_rank`, `groupby_category`) need
`EvalContext.industry`: `symbol → sector` string.

Helper: `pomelo_data::parse_industry_csv` accepts CSV text shaped like:

```csv
symbol,sector,market_cap
NVDA,Technology,5103000000000
XOM,Energy,470000000000
```

- Header row with `symbol` is skipped.
- Rows with empty `sector` are dropped.
- Extra columns after `sector` (e.g. `market_cap`) are allowed; only the first
  two fields are used for the map.

**Wiring:** the parser is in `pomelo-data`; **`yuzu-cli` currently leaves
`industry` empty.** Pass the map when calling the library/server integration
yourself, or load a snapshot from e.g. `tracked/*.csv.gz` in your product code.

---

## 8. Universe and point-in-time membership

### Universe

The set of columns in every panel is the **`symbols` list** for that run:

- **Server:** `BacktestRequest.symbols`
- **CLI:** every symbol that has a file under `prices/` (optionally limit by
  only syncing the files you want)

There is no special “universe file” format inside the engine.

### Point-in-time (PIT) index membership

The engine does **not** maintain historical index constituents.

| Approach | How |
|----------|-----|
| Fixed list | Pass today’s (or any fixed) ticker list as `symbols` — simple, not true PIT |
| PIT list per run | Your code chooses “listed as of `from`” tickers, then passes that `symbols` | 
| Membership panel | A `dates×symbols` 0/1 panel named `in_<index>` (e.g. `in_sp500`) in `panels/`; `mask(signal, in_sp500)` in lemon holds a name only while it was a member |

**Membership-panel convention.** A membership panel is a normal combined panel
(`panels/in_sp500.csv.gz`, wide `day,SYM,…` 0/1). The CLI (`run` / `sweep`)
auto-loads `in_sp500` / `in_nasdaq` / `in_dowjones` from `panels/` when present,
so `mask(signal, in_sp500)` works without hand-building the context. The library
path can insert any such panel directly; `yuzu-server` does **not** auto-load
custom series (see §"Custom series"). `yuzu-cli fmp-sync --index sp500` produces
both the panel and the ever-member price universe (see
[`fmp-data-source.md`](fmp-data-source.md) §6). Reconstruction is index-scoped
and **degrades for very old dates** (the vendor change log thins out).

Delisted names: keep `prices/{SYM}.*` files that **end** on the last trading
day. Pair with `BacktestConfig.delist_after` / `delist_haircut` so the NAV loop
force-exits after a NaN streak. Survivorship (only shipping still-listed names)
is a **data** choice; the engine cannot invent missing history.

---

## 9. In-memory model (after load)

Every series becomes a `Panel`:

- `dates: Vec<i32>` — YYYYMMDD, sorted unique  
- `symbols: Vec<String>` — column order  
- `data: Array2<f64>` — **NaN = missing**  
- Booleans are `1.0` / `0.0` in the same matrix  

`EvalContext` = `panels: HashMap<String, Panel>` + `industry: HashMap<String, String>`.

Binary ops align panels (union of dates, intersection of symbols). Details:
[`backtest-engine.md`](backtest-engine.md#the-data-model-panel).

---

## 10. Minimal offline example

Directory:

```text
mydata/
  prices/
    AAA.csv.gz
    BBB.csv.gz
```

`prices/AAA.csv` (plain CSV also works):

```csv
day,adj_open,adj_high,adj_low,adj_close,volume
2024-01-02,10,11,9,10,1000
2024-01-03,10,12,10,11,1100
2024-01-04,11,12,10,11.5,1200
```

`prices/BBB.csv` — same dates, different closes.

Strategy as a JSON `Expr` file `mom.json` (what `yuzu-cli run --spec` accepts).
Equivalent lemon: `is_largest(pct_change(close, 1), 1)`.

```json
{
  "op": "IsLargest",
  "n": 1,
  "of": {
    "op": "PctChange",
    "n": 1,
    "of": { "op": "Data", "name": "close" }
  }
}
```

CLI (after `cargo build -p yuzu-cli`):

```bash
# dates are YYYYMMDD; universe = every symbol under prices/
yuzu-cli run --data ./mydata --spec mom.json --from 20240102 --to 20240104
```

To author in lemon text, parse with the `lemon` crate / CLI first, then pass the
JSON tree to `yuzu-cli` or `run_backtest`.

Library sketch (no server):

```rust
// load panels with pomelo_data::load_panel / load_fundamental_panel,
// insert into EvalContext, then:
yuzu_core::run_backtest(&spec_json, &ctx, "close", &BacktestConfig::default())?;
```

Server request shape: see
[`crates/yuzu-server/examples/backtest-request.json`](../crates/yuzu-server/examples/backtest-request.json).
Data root via `YUZU_DATA_DIR` (or equivalent) must contain `prices/` (and
optionally `fundamentals/`, `panels/`).

---

## 11. Source of truth in code

| Contract | Location |
|----------|----------|
| Price columns / `Field` | `crates/pomelo-data/src/csv_io.rs` |
| Fundamental column list | `crates/pomelo-data/src/fundamentals.rs` → `FUNDAMENTAL_FIELDS`, `REPORT_EVENT_FIELD`, `FACTOR_PANEL_FIELDS` |
| Combined wide panels | `crates/pomelo-data/src/combined.rs` |
| Industry CSV parse | `crates/pomelo-data/src/industry.rs` |
| Format probe | `crates/pomelo-data/src/format.rs` |
| Server series routing | `crates/yuzu-server/src/lib.rs` → `price_field`, `handle_backtest` |
| Price discovery | `crates/pomelo-data/src/source.rs` → `list_symbols`; `crates/yuzu-cli/src/lib.rs` → `load_ctx` |
| FMP builder (writes this tree) | `crates/pomelo-fmp/` → `yuzu-cli fmp-sync` |
| Tree auditor (reads this tree) | `crates/pomelo-audit/` → `pomelo_audit::run_data_audit` (exposed as `yuzu-cli data-audit`) |

If this doc and the code disagree, **trust the code** and update this file.

## Auditing a tree

`yuzu-cli data-audit --data <dir|s3://bucket[/prefix]> [--from --to] [--json]`
runs a read-only data-quality pass over this layout (coverage, calendar gaps,
adjustment sanity, survivorship, NaN density, filing-date lag, index
membership) and prints per-check `OK` / `WARN` / `FAIL`; any `FAIL` exits
non-zero. `--data` audits a local tree or an S3/R2 tree identically — discovery
goes through `ObjectLister`, reads through `ObjectSource`, both implemented for
`LocalSource` and `pomelo-s3::S3Source`/`OutStore` (#149). See
[`fmp-data-source.md`](fmp-data-source.md) § *Auditing a synced tree* for the
check table and the shallow-vs-deep cost tradeoff over S3.

---

## Related docs

- [`backtest-engine.md`](backtest-engine.md) — panels, backtest, Report JSON  
- [`lemon.md`](lemon.md) — strategy language  
- [`crates/pomelo-data/README.md`](../crates/pomelo-data/README.md) — crate feature flags  
