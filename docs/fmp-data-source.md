# FMP Starter data gaps vs citrusinvest features

**Snapshot date:** 2026-07-09. FMP product names, history depth, and endpoint
access change over time — re-check [FMP’s own docs and pricing](https://site.financialmodelingprep.com/pricing-plans)
before relying on tier-specific claims below.

This document answers one question:

> If I only have an **FMP Starter**-class key (short US history, limited
> fundamentals depth, no bulk), which citrusinvest **features can I honestly
> backtest**, and which ones are blocked because the **input panels** are missing
> or too coarse?

It is **not** a Starter | Premium | Ultimate marketing comparison. Richer FMP
plans (or any other vendor) matter only as “full panels are possible.” The engine
never calls FMP; ops fail only when required series are absent from
`EvalContext` / your data root. On-disk shape: [`data-layout.md`](data-layout.md).

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
([`lemon.md`](lemon.md), [`schema/op-catalog.json`](../schema/op-catalog.json)).

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
| Point-in-time membership (e.g. in-index 0/1 panel or filtered `symbols`) | Avoid “today’s survivors only” bias ([`data-layout.md`](data-layout.md) § universe / PIT) |

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
| Historical **index constituents** (PIT) | Often weak or incomplete | **Degraded / Missing** |
| **Delisted** names with complete price files | Easy to omit if you only sync “active” lists | **Degraded** unless you ingest delisteds |

“Richer” FMP tiers mainly restore **depth**, **fuller fundamentals**, and
**bulk** so large universes are practical. They do not unlock secret lemon ops.

---

## 4. Gap table: Starter → what you cannot honestly backtest

| If your Starter-fed tree is weak on… | Then these citrusinvest capabilities are blocked or misleading |
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
   (survivorship and PIT are data choices — [`data-layout.md`](data-layout.md)).

---

## 6. The `fmp-sync` builder (bring-your-own key)

`yuzu-cli fmp-sync` fetches from FMP with **your own** API key and writes the
[`data-layout.md`](data-layout.md) tree. Direct HTTPS, **no third-party FMP SDK**;
the key stays on your machine and no FMP data is redistributed. FMP lives only in
`yuzu-cli` (behind the default-on `fmp-sync` cargo feature) — never in
`yuzu-core` / `yuzu-data` / WASM.

```bash
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
  --symbols AAPL,MSFT,GOOGL --from 20200101 --to 20251231 \
  [--include-fundamentals] [--include-industry] \
  [--include-etf] [--min-market-cap 1e9] [--all-symbols] \
  [--rate-limit 300] [--max-retries 4] [--append | --resume]
```

| Output | Flag | FMP endpoint (stable) |
|--------|------|-----------------------|
| `prices/{SYM}.csv.gz` (adjusted OHLCV) | always | `historical-price-eod/dividend-adjusted` |
| `fundamentals/{SYM}.csv.gz` (dense forward-filled factors + `report_event`) | `--include-fundamentals` | `ratios` + `key-metrics` + `financial-growth` (annual) |
| `tracked/universe.csv.gz` (`symbol,sector,market_cap`) | `--include-industry` | `profile` |
| — (universe discovery / ETF & market-cap screen) | `--all-symbols`, `--min-market-cap`, default stock-only | `stock-list`, `profile` |
| symbol list file (`yuzu-cli fmp-symbols`) | `fmp-symbols --out …` | `company-screener` |

### Establishing the symbol list first

For a whole-market backtest, build the sync universe as a reviewable artifact
**before** pulling prices:

```bash
# 1. build a screened symbol list (FMP company screener)
yuzu-cli fmp-symbols --api-key "$FMP_API_KEY" --out ./universe.txt \
  --min-market-cap 1e9 --exchange NASDAQ,NYSE   # stocks only by default

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
  (a prebuilt list, e.g. from `fmp-symbols`), or `--all-symbols` to sync FMP's
  whole tradable universe (`stock-list`). Exactly one source per run. The full
  universe is large — pair it with `--min-market-cap` / `--rate-limit` / `--resume`.
- **Stocks only** — ETFs and mutual/closed-end funds are **skipped by default**
  (classified from the `profile` endpoint's `isEtf` / `isFund`); pass
  `--include-etf` to keep them.
- **Market-cap floor** — `--min-market-cap <usd>` drops symbols below that
  company market cap (`0` = off), read from the `profile` endpoint. Accepts unit
  suffixes: `1b`, `500m`, `10k`, `2.5t` (or a plain number / `1e9`).
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
calendar; fields the plan does not return are left `NaN`. Richer fundamentals,
full-universe, point-in-time membership, and delist honesty are out of scope —
see §2–§4 above for what a Starter key can and cannot honestly support, and #53
for the follow-up.

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
| On-disk series names & directories | [`data-layout.md`](data-layout.md) |
| Op list | [`lemon.md`](lemon.md), `schema/op-catalog.json` |
| NAV / costs / MAE | [`backtest-engine.md`](backtest-engine.md) |
| Fundamental field list | `crates/yuzu-data/src/fundamentals.rs` |
| TA vs single-series indicators | `ops/ta.rs` vs `ops/indicators.rs` |
| `fmp-sync` builder (endpoints, field mapping) | `crates/yuzu-cli/src/fmp.rs` |
