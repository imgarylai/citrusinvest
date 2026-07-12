---
title: "FMP data source"
editUrl: false
sourceFile: docs/fmp-data-source.md
---

<!-- Imported from docs/fmp-data-source.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
**Snapshot date:** 2026-07-09. FMP product names, history depth, and endpoint
access change over time — re-check [FMP’s own docs and pricing](https://site.financialmodelingprep.com/pricing-plans)
before relying on tier-specific claims below.

This document answers one question:

> If I only have an **FMP Starter**-class key (short US history, limited
> fundamentals depth, no bulk), which citrusquant **features can I honestly
> backtest**, and which ones are blocked because the **input panels** are missing
> or too coarse?

It is **not** a Starter | Premium | Ultimate marketing comparison. Richer FMP
plans (or any other vendor) matter only as “full panels are possible.” The engine
never calls FMP; ops fail only when required series are absent from
`EvalContext` / your data root. On-disk shape: [`data-layout.md`](../reference/data-layout).

---

## 1. Mental model

```text
lemon op / backtest knob  →  needs series (close, high, pe, …)
                           →  your pipeline fills panels
                           →  if Starter cannot fill them well, that feature is unusable *in practice*
```

| Layer | Responsibility |
|-------|----------------|
| `yuzu-core` / lemon | Pure math on panels |
| Your data (e.g. FMP sync) | Populate `prices/`, `fundamentals/`, industry map, … |
| This doc | Map **feature families → series → Starter gap** |

---

## 2. Feature families → required series

Groups match how authors use the library. Op names are the lemon surface
([`lemon.md`](../reference/lemon), [`schema/op-catalog.json`](https://github.com/citrusquant/citrusquant/blob/main/schema/op-catalog.json)).

### 2.1 Price time-series (single series)

| Needs | Ops / behavior |
|-------|----------------|
| Any numeric panel (usually `close`) | `sma`/`average`, `ema`, `std`, `rsi`, `pct_change`, `rise`, `fall`, `shift`, `rolling_max`, `ceil` |
| Same | Future BB / MACD / Donchian compositions (`sma`±`std`, `ema` stacks, `rolling_max`/`rolling_min`) — see #41 |

### 2.2 Pure cross-section (no vendor “special” fields)

| Needs | Ops |
|-------|-----|
| Any wide numeric panel | `is_largest`, `is_smallest`, `rank`, `quantile_row`, `winsorize`, `zscore`, `bucket`, `demean`, `mask`, `normalize_row` |

### 2.3 Signals & rotation on booleans / ranks

| Needs | Ops |
|-------|-----|
| Entry/exit boolean panels (from price or factors) | `sustain`, `is_entry`, `is_exit`, `exit_when`, `hold_until` (without stops), `rebalance` |
| Optional `close` for price stops on `hold_until` | `stop_loss` / `take_profit` / `trail_stop` use a price panel (typically `close`) |

### 2.4 Multi-input OHLCV TA (`ops/ta.rs`)

| Needs | Ops |
|-------|-----|
| `high`, `low`, `close` | `atr`, `natr`, `willr`, `cci`, `stoch_k`, `stoch_d`, `aroon_up`, `aroon_down`, `adx`, `plus_di`, `minus_di` |
| `close`, `volume` | `obv` |
| `high`, `low`, `close`, `volume` | `mfi`, `vwap` |

### 2.5 Report / NAV extras that need more than close

| Needs | Behavior |
|-------|----------|
| `high` / `low` | Per-trade **MAE/MFE** (server loads them when present) |
| `volume` + `initial_capital` + caps | Liquidity participation cap, square-root **impact** (`BacktestConfig`) |
| `price_key` panel (`close` / `open` / …) | NAV returns and trade marks (`run_backtest(..., price_key, cfg)` — not a `BacktestConfig` field; see #42) |
| Future `touched_exit` (#20) | Hard dependency on high/low range vs stop level |

### 2.6 Fundamental / factor strategies

| Needs | Usage |
|-------|--------|
| `pe`, `ps`, `pb`, `roe`, `net_margin`, `debt_to_equity`, `market_cap`, `gross_margin`, `receivables_turnover`, `debt_to_assets`, `revenue`, `revenue_growth`, `eps_growth`, `operating_income_growth`, `net_income_growth`, `gross_profit_growth` | Bare `Data` names in lemon; often fed into `rank` / `zscore` / `is_largest` / `neutralize` |
| `report_event` | Filing-day mask (event-style research; #45) |
| Snapshot factors: `piotroski_score`, `altman_z`, `fcf_yield`, `pe_industry_pctile`, `analyst_upside_pct`, `consensus_rating` | Combined panels or derived locally |

### 2.7 Industry-aware ops

| Needs | Ops |
|-------|-----|
| `EvalContext.industry` (symbol → sector) | `neutralize_industry`, `industry_rank`, `groupby_category`; future `in_sector` (#40), `cap_industry` (#46) |

### 2.8 Neutralize against other series

| Needs | Ops |
|-------|-----|
| Factor panels listed in `by=[...]` | `neutralize(of, by=[pe, market_cap], …)` |

### 2.9 Universe honesty (not a lemon op, but backtest quality)

| Needs | Why |
|-------|-----|
| Symbols that **delist** mid-sample + prices that end | Engine `delist_after` / `delist_haircut` only fire if dead names exist (#26) |
| Point-in-time membership (e.g. in-index 0/1 panel or filtered `symbols`) | Avoid “today’s survivors only” bias ([`data-layout.md`](../reference/data-layout) § universe / PIT) |

### 2.10 Research surfaces (planned / adjacent)

| Needs | Feature |
|-------|---------|
| Factor + forward return panels | Factor report (#45) |
| Event panel + returns | Event study (#45) |

---

## 3. Starter-tier data reality (inputs, not plans)

Labels for Starter-class access (US retail API, short history). Treat as a
checklist for your sync job; re-verify endpoints when implementing a builder.

| Input | Typical Starter situation | Label |
|-------|---------------------------|--------|
| Daily adjusted **OHLCV** (`open`/`high`/`low`/`close`/`volume`) | Available for **US** names; history often capped around **~5 years** | **Available (short)** |
| History **beyond** that window | Not available on Starter-class depth | **Missing (depth)** |
| **Annual** fundamentals / ratios usable as pe/roe/… | Often available for US, coarser than quarterly PIT research | **Degraded** |
| Dense **quarterly** / long fundamental history | Starter blurb emphasizes annual / short depth | **Degraded / Missing** |
| `report_event` (filing / report dates) | Partial; depends on calendar/filing endpoints you wire | **Degraded** |
| Company **sector** for industry map | US profile fields usually enough | **Available** |
| Snapshot scores / analyst series | Often limited, extra endpoints, or must **derive** | **Degraded / Derive** |
| Full-universe **bulk** EOD/statements | Not on Starter | **Missing (ops scale)** — ops still run on small lists |
| Historical **index constituents** (PIT) | Reconstructable from the change log via `--index` (#125); thins out for old dates | **Degraded** (recent OK, old drifts) |
| **Delisted** names with complete price files | Easy to omit if you only sync “active” lists; `--include-delisted` ingests them (#124) | **Degraded** unless you pass `--include-delisted` |

“Richer” FMP tiers mainly restore **depth**, **fuller fundamentals**, and
**bulk** so large universes are practical. They do not unlock secret lemon ops.

---

## 4. Gap table: Starter → what you cannot honestly backtest

| If your Starter-fed tree is weak on… | Then these citrusquant capabilities are blocked or misleading |
|--------------------------------------|------------------------------------------------------------------|
| History longer than ~5y | **Any** strategy needing longer samples: long walk-forwards (#21), multi-regime TA, long factor studies — all op families |
| `close` only (no high/low) | **OHLCV TA**: `atr`, `natr`, `willr`, `cci`, `stoch_*`, `aroon_*`, `adx`, `±di`; MAE/MFE quality; future **touched_exit** (#20) |
| No `volume` | `obv`, `mfi`, `vwap`; **liquidity cap** / **impact_coef** costs |
| No / annual-only / sparse fundamentals | Factor strategies on `pe`/`roe`/growth/`market_cap`; `neutralize(..., by=[pe, …])` quality; snapshot factors unless derived |
| No industry map | `neutralize_industry`, `industry_rank`, `groupby_category`; planned `in_sector` / `cap_industry` |
| No `report_event` / event dates | Event-style research (#45) |
| Active-only universe, no delisteds / no PIT | Survivorship-biased results even if metrics look fine (#26) |
| Cannot bulk-refresh thousands of names | Not an op failure — **engineering** limit; small lists still backtest |

### What still works well on Starter (typical 5y US OHLCV)

Assume you have adjusted daily **open/high/low/close/volume** for a modest US
list and ≤ ~5y:

| Family | Status |
|--------|--------|
| Price TS (`sma`, `ema`, `rsi`, …) | **OK** |
| CS preprocess (`zscore`, `winsorize`, `bucket`, `demean`, `rank`, …) | **OK** |
| Rotation on price signals (`exit_when`, `hold_until`, `rebalance`) | **OK** (stops need a price panel, usually close) |
| OHLCV TA in `ops/ta.rs` | **OK** if OHLC(V) present |
| Volume TA + impact knobs | **OK** if `volume` present |
| BB / MACD / Donchian-style compositions | **OK** (price only) |
| Fundamental / industry / event / long-history / full-market honesty | **Not OK** without better data |

---

## 5. Practical guidance

1. **Learning / demo / short US price strategies** — Starter-class data is enough
   if OHLCV is complete for your window.
2. **Factor, industry-neutral, or multi-decade research** — Starter depth and
   annual-leaning fundamentals are the bottleneck, not missing lemon syntax.
3. **Product-scale universes** — lack of bulk is a pipeline problem; the engine
   will still evaluate any panels you can build.
4. **Always** document your window and universe construction next to results
   (survivorship and PIT are data choices — [`data-layout.md`](../reference/data-layout)).

---

## 6. The `fmp-sync` builder (bring-your-own key)

`yuzu-cli fmp-sync` fetches from FMP with **your own** API key and writes the
[`data-layout.md`](../reference/data-layout) tree. Direct HTTPS, **no third-party FMP SDK**;
the key stays on your machine and no FMP data is redistributed. FMP lives in the
standalone **`pomelo-fmp`** crate (behind the default-on `fmp-sync` cargo feature)
— never in `yuzu-core` / `pomelo-data` / WASM. The CLI is a thin wrapper over
`pomelo-fmp`; a Rust service can depend on the crate directly and write the same
tree to an S3/R2 bucket via `pomelo_fmp::sync_into` + `pomelo-s3`.

```bash
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
  --symbols AAPL,MSFT,GOOGL --from 20200101 --to 20251231 \
  [--include-fundamentals] [--include-industry] [--include-snapshot-factors] \
  [--include-etf] [--include-delisted] [--min-market-cap 1b] [--all-symbols] [--index sp500] [--exchange NASDAQ,NYSE,AMEX] \
  [--rate-limit 300] [--max-retries 4] [--append | --resume]
```

### Output target: local path or S3/R2 (`--out`)

`--out` takes a local path **or** an `s3://bucket[/prefix]` URL, so a service can
sync straight to R2/S3 without a local staging dir. Both write a **byte-identical**
`data-layout.md` tree — the CLI and a Rust service (`pomelo_fmp::sync_into` +
`pomelo-s3`) share the same code path over an `ObjectSink`; only the destination
differs.

```bash
export S3_ENDPOINT=https://<accountid>.r2.cloudflarestorage.com  # or your S3 endpoint
export S3_ACCESS_KEY_ID=…  S3_SECRET_ACCESS_KEY=…  S3_REGION=auto  # region defaults to "auto"
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out s3://my-bucket/mirror/v1 \
  --symbols AAPL,MSFT --include-fundamentals --include-snapshot-factors
```

Keys are written under the URL's optional `/prefix` (e.g.
`mirror/v1/prices/AAPL.csv.gz`). **`--index` currently requires a local `--out`**
— the point-in-time membership panel is written in a post-sync pass that reads a
local trading calendar; sync the index to a local tree (or omit `--index`).

| Output | Flag | FMP endpoint (stable) |
|--------|------|-----------------------|
| `prices/{SYM}.csv.gz` (adjusted OHLCV) | always | `historical-price-eod/dividend-adjusted` |
| `fundamentals/{SYM}.csv.gz` (dense forward-filled factors + `report_event`, visible from the **filing date**) | `--include-fundamentals` | `ratios` + `key-metrics` + `financial-growth` + `income-statement` (annual) |
| `tracked/universe.csv.gz` (`symbol,sector,market_cap`) | `--include-industry` | `profile` |
| `panels/{piotroski_score,altman_z,fcf_yield,analyst_upside_pct,consensus_rating}.csv.gz` (snapshot-factor panels) | `--include-snapshot-factors` | `financial-scores` + `key-metrics-ttm` + `price-target-consensus` + `grades-summary` + `income-statement` |
| — (universe discovery / exchange, ETF & market-cap screen) | `--all-symbols`, `--exchange`, `--min-market-cap` | `company-screener`, `profile` |
| — (delisted names unioned into the universe) | `--include-delisted` | `delisted-companies` |
| `panels/in_<index>.csv.gz` (PIT membership 0/1) + ever-member price universe | `--index sp500` | `sp-500` + `historical-sp-500` |
| symbol list file (`yuzu-cli fmp-symbols`) | `fmp-symbols --out …` | `company-screener` |

### Snapshot-factor panels (`--include-snapshot-factors`, #132)

Computes five combined `panels/{name}.csv.gz` factor panels the engine reads as
bare `Data` series (feed into `rank` / `zscore` / `is_largest`, etc.):

| Series | Source | Transform |
|--------|--------|-----------|
| `piotroski_score` | `financial-scores` | `piotroskiScore` (0–9), authoritative |
| `altman_z` | `financial-scores` | `altmanZScore` |
| `fcf_yield` | `key-metrics-ttm` | `freeCashFlowYieldTTM` |
| `analyst_upside_pct` | `price-target-consensus` | `(targetConsensus − close) / close × 100` |
| `consensus_rating` | `grades-summary` | Strong Buy = 1 … Strong Sell = 5 (lower = more bullish) |

**Formulas are a native port of the web app's `factor-snapshot-panels.ts`** so
CLI- and web-built panels agree. `pe_industry_pctile` (the sixth
`FACTOR_PANEL_FIELDS` entry) needs a cross-sectional industry cohort and is
Phase 2.

**Current-snapshot semantics (honest limitation).** FMP's `financial-scores` /
`*-ttm` / `price-target-consensus` / `grades-summary` return a *current* value
with no history, so a one-shot sync writes a **current snapshot**, not a time
series: `piotroski_score` / `altman_z` / `fcf_yield` are anchored to the latest
report's **filing date** (visible from then on); `analyst_upside_pct` /
`consensus_rating` are anchored to the **last synced trading day** (final bar
only). Use these for **current-universe screening**, not deep historical
backtests — richer history needs daily snapshot accumulation over time (a service
concern). Each factor costs extra FMP requests per symbol; pair with
`--rate-limit`. On `--resume`, panels cover only the symbols processed this run.

### Establishing the symbol list first

For a whole-market backtest, build the sync universe as a reviewable artifact
**before** pulling prices:

```bash
# 1. build a screened symbol list (FMP company screener)
yuzu-cli fmp-symbols --api-key "$FMP_API_KEY" --out ./universe.txt \
  --min-market-cap 1b   # US stocks (NASDAQ,NYSE,AMEX) by default; --exchange to change

# 2. review/edit ./universe.txt, then sync prices for exactly that list
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
  --symbols-file ./universe.txt --from 20200101 --to 20251231
```

`fmp-symbols` writes one ticker per line (`#` comments allowed); `fmp-sync
--symbols-file` reads that back (also accepts a `symbol,…` CSV). This decouples
*which* symbols exist from *fetching* their data, so the universe can be curated,
diffed, and re-synced.

Universe & screening (from #52 review):

- **Symbols** — `--symbols AAPL,MSFT,…` (explicit list), `--symbols-file <path>`
  (a prebuilt list, e.g. from `fmp-symbols`), or `--all-symbols` to sync the
  screened universe (FMP `company-screener`). Exactly one source per run. The
  universe is large — pair it with `--min-market-cap` / `--rate-limit` / `--resume`.
- **Exchanges** — the universe defaults to the three **US** majors
  (`NASDAQ,NYSE,AMEX` — AMEX is now NYSE American). Override with `--exchange`
  (comma-separated FMP codes) on `fmp-symbols` / `fmp-sync --all-symbols`; pass
  `--exchange all` for every exchange.
- **Stocks only** — ETFs and mutual/closed-end funds are **skipped by default**
  (classified from the `profile` endpoint's `isEtf` / `isFund`); pass
  `--include-etf` to keep them.
- **Market-cap floor** — `--min-market-cap <usd>` drops symbols below that
  company market cap (`0` = off), read from the `profile` endpoint. Accepts unit
  suffixes: `1b`, `500m`, `10k`, `2.5t` (or a plain number / `1e9`).
- **Point-in-time index** — `--index sp500` (or `nasdaq` / `dowjones`) is a
  universe source that reconstructs membership from FMP's current snapshot
  (`sp-500`) + change log (`historical-sp-500`): it syncs every name that was
  **ever** a member over `[from,to]` (survivorship-honest, incl. names that later
  left) and writes a `panels/in_sp500.csv.gz` 0/1 membership panel. Backtest with
  `mask(signal, in_sp500)` to hold a name only while it was a member; the CLI
  `run` / `sweep` auto-loads `in_sp500` from `panels/`. **Honest weakness:**
  reconstruction is index-scoped and **drifts the further back you go** (older
  change-log rows drop the removed ticker / reason), so it's reliable in recent
  years and degrades pre-2000s. Mutually exclusive with the other universe
  sources (#125).
- **Delisted names** — `--include-delisted` unions FMP's `delisted-companies`
  universe (filtered by `--exchange`) into the symbol list, so dead securities
  are synced too. Their `prices/{SYM}.csv.gz` simply **ends at the delisting
  date** and the engine's `delist_after` forced-exit does the rest (#124 / #26).
  This removes survivorship bias at the data layer — without it, an
  `--all-symbols` sync is survivors-only. Note: `delisted-companies` carries no
  market cap, so `--min-market-cap` does **not** filter these names; and once
  dead names are present, set a real `delist_after` (e.g. `10`) on the backtest
  — the engine default stays `0` (survivorship-friendly) for compatibility.
- Screening happens **before** the price fetch, so a filtered symbol costs no
  price request. A single profile GET per symbol serves the ETF/fund screen, the
  cap screen, and `--include-industry`. A profile-endpoint error fails **open**
  (the symbol is kept) so a secondary hiccup never drops the price sync.

Operational knobs (from #52 discussion):

- **Rate limit** — `--rate-limit` requests/minute (`0` = no throttle). Set it to
  your plan's ceiling; Starter-class keys are commonly ~300/min. The tool does
  **not** auto-detect your tier — check your plan and pass the value.
- **Retry** — `--max-retries` with exponential backoff on `429` / `5xx` /
  transport errors; a `4xx` (bad key/symbol) fails fast. The API key is redacted
  from every log line and error message.
- **Resume** — `--resume` skips symbols that already have a price file, to
  continue an interrupted multi-symbol run.
- **Append** — `--append` merges freshly fetched rows into existing files
  (extend an existing tree's history); fetched rows win on a date collision.

**MVP scope.** Enough for **price-based** strategies over a short US window:
close/OHLC TA and cross-section ops on a modest symbol list (the acceptance path —
`fmp-sync` then `yuzu-cli run`). Fundamentals are **best-effort** from the annual
ratios/metrics/growth endpoints and are dense forward-filled onto the price
calendar; fields the plan does not return are left `NaN`. Delisted names can be
unioned in with `--include-delisted` for survivorship-honest backtests (#124),
and `--index sp500` reconstructs point-in-time index membership (#125).

**Point-in-time visibility (#131).** A snapshot becomes visible on the day the
report was **filed** (`filingDate` / `acceptedDate` from `income-statement`), not
on the fiscal period-end — which is typically 1–3 months earlier. This avoids the
lookahead bias of "seeing" full-year numbers before they were public. If a plan
does not serve `income-statement` for a symbol, that symbol's snapshots fall back
to period-end visibility (logged per symbol, and optimistic — the older behavior).
`report_event` fires on the filing day.
Richer fundamentals and full-universe / bulk rebuilds remain out
of scope — see §2–§4 above for what a Starter key can and cannot honestly
support, and #53 for the remaining follow-up.

---

## 7. ToS / product boundary

- Bring-your-own API key; keep vendor data on the user’s machine.
- Displaying or redistributing vendor data to end users may require a separate
  agreement with the vendor — out of scope for this engine repo.
- No FMP (or other vendor) dependency in `yuzu-core` / WASM.

Related work: data-layout contract; optional `fmp-sync` CLI (#52 / #53); delist
honesty (#26).

---

## 8. Source of truth in this repo

| Topic | Where |
|-------|--------|
| On-disk series names & directories | [`data-layout.md`](../reference/data-layout) |
| Op list | [`lemon.md`](../reference/lemon), `schema/op-catalog.json` |
| NAV / costs / MAE | [`backtest-engine.md`](../reference/backtest-engine) |
| Fundamental field list | `crates/pomelo-data/src/fundamentals.rs` |
| TA vs single-series indicators | `ops/ta.rs` vs `ops/indicators.rs` |
| `fmp-sync` builder (endpoints, field mapping) | `crates/pomelo-fmp/` |

