---
title: "Data sources"
editUrl: false
sourceFile: docs/data-sources.md
---

<!-- Imported from docs/data-sources.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
> **Status: current** — re-audited after epic
> [#192](https://github.com/citrusquant/citrusquant/issues/192) (`pomelo-eodhd` /
> `eodhd-sync`) landed. Parent stance: [#180](https://github.com/citrusquant/citrusquant/issues/180).
>
> Vendor coverage is research plus what the in-repo adapters implement — not a
> product promise that numbers match across vendors. APIs, tiers, and pricing
> change; re-check before you build a pipeline.

## What is decided

1. **Contract = [`data-layout`](../reference/data-layout).** The engine never calls a
   market-data vendor.
2. **Two official one-shot syncs** (both write the same layout via `ObjectSink`):
   - `pomelo-fmp` / `yuzu-cli fmp-sync` — see [`fmp-data-source.md`](../reference/fmp-data-source)
   - `pomelo-eodhd` / `yuzu-cli eodhd-sync` — see [`eodhd-data-source.md`](../reference/eodhd-data-source)
3. **Not locked to either vendor:** you may BYO files that match the layout, or
   **assemble** a tree from multiple sources (extra steps OK).
4. Further `pomelo-XXX` adapters only if a vendor can cover roughly the same
   **data blocks** (optional).
5. **No** cross-vendor mega `sync-all --vendor=…`.

FMP Starter **feature** gaps (which lemon ops need which panels) are independent
of vendor: see [`fmp-data-source.md`](../reference/fmp-data-source).

---

## Data blocks → layout paths

These are the blocks citrusquant cares about. Fill what your strategy needs;
everything except prices is optional for pure price/TA work.

| Data block | Layout target | Required? | Notes |
|------------|---------------|-----------|--------|
| Adjusted OHLCV | `prices/{SYM}.*` | For price strategies / CLI universe | Columns: see [data-layout](../reference/data-layout) § prices |
| Fundamentals / ratios | `fundamentals/{SYM}.*` | No | `pe`, `roe`, growth fields, … |
| Industry / sector map | `tracked/*.csv.gz` (or load map in code) | No | Needed for `neutralize_industry`, `industry_rank`, … |
| Delisted names | keep dead symbols’ `prices/` files (end on last trade) | No | Survivorship-honest universes |
| Index PIT membership | `panels/in_<index>.*` (0/1 wide panel) | No | e.g. `in_sp500`; engine does not invent membership |
| Universe / screener | symbol list / which `prices/` files exist | You | Cap/exchange filters are **your** job unless a sync tool helps |
| Snapshot factors | `panels/{name}.*` | No | e.g. piotroski, altman_z, analyst upside — vendor- or DIY-derived |

---

## Official adapters: block coverage (post-#192 re-audit)

What **in-repo** CLIs actually write today (not “the vendor has an API somewhere”).

| Block | `fmp-sync` | `eodhd-sync` | Honest gap if you pick EODHD only |
|-------|------------|--------------|-----------------------------------|
| Adjusted OHLCV | Y — FMP dividend-adjusted EOD | Y — scale OHLC by `adjusted_close/close` | Adj policy may differ from FMP; not bit-identical |
| Fundamentals densify | Y — ratios + growth endpoints + filing visibility | Y — annual IS/BS densify + `filing_date` → `report_event` | `pe`/`ps`/`pb`/`market_cap` left **NaN** historically (statement ratios only); `eps_growth` = NI YoY proxy |
| Industry map | Y — `--include-industry` | Y — `--include-industry` | Sector string source differs |
| Delisted | Y — `--include-delisted` | Y — `--include-delisted` | List completeness differs by vendor |
| Index PIT | Y — `--index sp500` (also nasdaq/dowjones on FMP) | Y — `--index sp500` only | No multi-index CLI on EODHD v1 |
| Screener | Y — `fmp-symbols` | Y — `eodhd-symbols` | Filter surface / plan limits differ |
| Snapshot factors | Y — 6 panels (incl. piotroski, altman) | P — 4 panels: analyst upside/rating, `fcf_yield`, `pe_industry_pctile` | **No** piotroski/altman; all current-as-of last bar |

Legend for the wider candidate table below: **Y** = typically usable · **P** =
partial / plan-dependent · **N** = essentially no · **TBD** = not verified here.

| Block | FMP (`fmp-sync`) | EODHD (`eodhd-sync`) | Finnhub | Tiingo | Polygon | Sharadar SF1 | EDGAR DIY |
|-------|------------------|----------------------|---------|--------|---------|--------------|-----------|
| Adjusted OHLCV | Y (official) | Y (official\*) | Y | Y | Y | P (separate) | N |
| Fundamentals | Y (official) | Y / P† (official densify) | Y | P | P | Y | DIY |
| Industry map | Y (official) | Y (official) | Y | P / TBD | P / TBD | TBD | extra |
| Delisted | Y (flag) | Y (flag) | P / TBD | TBD | TBD | TBD | partial |
| Index PIT | Y (flag) | Y (SPX) / P‡ | Y | N / weak | N / weak | N | hard |
| Screener | Y (`fmp-symbols`) | Y (`eodhd-symbols`) | Y | weak | P | N | N |
| Snapshot scores | Y (6 panels) | P (4 panels; no piotroski/altman) | P / TBD | weak | weak | N | N |

\* EODHD: full **adj OHLC** via local scale; native feed is raw OHLC + `adjusted_close`.  
† Dense historical ratios from annual statements; Highlights/Valuation remain **TTM / current** (used for snapshot factors, not fundies history).  
‡ S&P 500 historical constituents via `GSPC.INDX`; broader index history often Marketplace add-on.

**In-repo full sync CLIs:** FMP and EODHD. Other columns mean “you can often
buy/fetch this block and write the layout yourself,” not “we ship an adapter.”

---

## How to get a dataset

| Path | What you do | Status |
|------|-------------|--------|
| **One-shot FMP** | `yuzu-cli fmp-sync …` → data root | Official |
| **One-shot EODHD** | `yuzu-cli eodhd-sync …` → data root | Official (epic #192) |
| **BYO** | Write the [data-layout](../reference/data-layout) tree yourself | Supported |
| **Assemble** | Fill blocks from different vendors into **one** data root | Supported as a pattern; recipes below are sketches |

### Choosing FMP vs EODHD vs assemble

| If you want… | Prefer |
|--------------|--------|
| Maximum snapshot factor set (piotroski, altman, …) | FMP `--include-snapshot-factors` |
| Vendor choice / non-FMP key, same layout | EODHD `eodhd-sync` (accept gaps above) |
| Dense historical `pe`/`ps`/`pb` in fundies CSV | FMP (EODHD leaves them NaN historically) |
| SPX-only PIT + statement densify without FMP | EODHD |
| Mix best price feed + best fundies | Assemble (Recipe B) — do not dual-write conflicting keys |
| Price/TA only | Either one-shot, or Recipe A |

**What you lose leaving FMP:** bit-identical numbers; FMP-native adj OHLC;
historical price multiples in fundies; piotroski/altman panels; multi-index CLI
flags; FMP screener / Starter-tier economics (you gain EODHD’s plan/call model
instead).

**Dual-running both adapters:** fine for comparison research — write to
**separate** data roots. Do not merge two vendors’ `prices/` for the same symbol
without an explicit reconciliation step (splits/dividends/definitions differ).

Engine / `pomelo-data` only need the finished tree (local or S3 via
`pomelo-s3`). Optional quality pass: `yuzu-cli data-audit`.

---

## Assemble recipes

These are **manual multi-step** sketches. Legal/ToS and rate limits are your
responsibility — not copy-paste production runbooks.

### Recipe A — price strategies only (minimal)

**Goal:** TA / cross-section on prices; no fundamentals.

1. Pick any EOD source you trust (Tiingo, Polygon, EODHD, FMP, …).
2. For each symbol, write adjusted OHLCV to `prices/{SYM}.csv.gz` per
   [data-layout](../reference/data-layout).
3. Point `yuzu-cli --data <root>` or your loader at that root.

**Not covered:** factor panels, industry ops, delist haircuts unless you also
add those blocks.

### Recipe B — prices + fundamentals (two sources)

**Goal:** factor-style series without a single-vendor full tree.

1. **Prices** from a price-strong vendor (e.g. Tiingo / Polygon / EODHD) →
   `prices/`.
2. **Fundamentals** from a fundies-strong source (e.g. EODHD densify / Sharadar /
   EDGAR-derived ratios) → `fundamentals/{SYM}.*` with the column set in
   [data-layout](../reference/data-layout) § fundamentals.
3. Align calendars / symbols yourself (same tickers, overlapping `day` range).
4. Optional: industry CSV under `tracked/`; optional: delisted names as
   truncated price files.

Official one-shot alternatives: full `fmp-sync` or full `eodhd-sync` with
`--include-fundamentals`.

### Recipe C — small free-tier demo universe

**Goal:** 50–200 US names for demos, not full-market sync.

1. Use a free tier (Finnhub / Tiingo / …) within published rate limits.
2. Sync only the blocks you need (usually prices ± thin fundies).
3. Expect incomplete delist / index PIT / snapshot coverage.

**TBD:** call-budget math ([#183](https://github.com/citrusquant/citrusquant/issues/183)).

### Recipe D — FMP one-shot (reference)

```bash
# Illustrative — see pomelo-fmp / CLI help for current flags
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
  --symbols AAPL,MSFT --from 20200101 --to 20241231
```

Optional flags cover industry, delisted, index membership, snapshot factors.
Starter-tier honesty: [`fmp-data-source.md`](../reference/fmp-data-source).

### Recipe E — EODHD one-shot (reference)

```bash
export EODHD_API_TOKEN=…   # or EODHD_API_KEY
yuzu-cli eodhd-sync --out ./mydata --symbols AAPL,MSFT \
  --from 20200101 --to 20241231 \
  --include-fundamentals --include-industry --include-snapshot-factors
```

Plans, call costs, and gap table: [`eodhd-data-source.md`](../reference/eodhd-data-source).

---

## EODHD mapping (spike #182 → shipped #192)

Research date ~2026-07; adapter shipped in phases #193–#198. Verified against
EODHD public docs + `api_token=demo` samples and the in-repo crate. Plans: EOD
history + **Fundamentals** package (and higher) for fundies/index; free tier is
too thin for full-market sync.

### Gate decision

| | |
|--|--|
| **Outcome** | **Near-FMP core blocks, with accepted gaps** → full adapter **shipped** |
| **Adapter** | `pomelo-eodhd` + `yuzu-cli eodhd-sync` / `eodhd-symbols` (epic [#192](https://github.com/citrusquant/citrusquant/issues/192), closed) |
| **Accepted gaps** | Adj OHLC reconstruction; historical price multiples NaN in fundies; snapshot subset (no piotroski/altman); SPX-only index CLI; not bit-identical vs FMP |

### Block coverage (vendor surface → layout)

| Block | Verdict | EODHD surface | → data-layout | Notes / caveats |
|-------|---------|---------------|---------------|-----------------|
| Adjusted OHLCV | **Y** (with transform) | `GET /api/eod/{SYM}.{EX}` | `prices/{SYM}.csv.gz` | Scale OHLC by `adjusted_close/close`. Bulk: `/api/eod-bulk-last-day/{EX}` for daily refresh (not wired in CLI v1). |
| Fundamentals | **Y / P** | `GET /api/v1.1/fundamentals/{SYM}.{EX}` (~10 calls / request) | `fundamentals/{SYM}.*` | Adapter densifies **annual** IS/BS + `filing_date` visibility. Price multiples NaN historically. |
| Industry map | **Y** | fundamentals `General.Sector` / Industry | `tracked/universe.csv.gz` | Via `--include-industry`. |
| Delisted | **Y** | `exchange-symbol-list?delisted=1` | truncated `prices/` | Via `--include-delisted`. |
| Index PIT | **Y / P** | `GSPC.INDX` historical components | `panels/in_sp500.csv.gz` | `--index sp500` (local `--out` only). |
| Screener | **Y** | `GET /api/screener` (~5 calls / request) | symbol list | `yuzu-cli eodhd-symbols`. |
| Snapshot scores | **P** | AnalystRatings + Highlights + CF yearly | 4 `panels/*` | No piotroski/altman. Current-as-of last bar. |

### Suggested field map (`FUNDAMENTAL_FIELDS`)

Layout columns from `pomelo-data` (`pe`, `ps`, …). EODHD is **not** 1:1 with FMP
JSON names. What the adapter **writes** today:

| Layout column | EODHD adapter source | Quality |
|---------------|----------------------|---------|
| `pe` / `ps` / `pb` / `market_cap` | (not densified) | **NaN** in historical fundies CSV |
| `roe` | NI / equity from annual statements | densified |
| `net_margin` | NI / revenue | densified |
| `debt_to_equity` | liab-or-debt / equity | densified |
| `gross_margin` | GP / revenue | densified |
| `receivables_turnover` | revenue / receivables | densified when present |
| `debt_to_assets` | debt-or-liab / assets | densified |
| `revenue` | IS `totalRevenue` | densified |
| `revenue_growth` … `gross_profit_growth` | YoY on IS lines | densified; `eps_growth` = NI YoY proxy |
| `report_event` | `filing_date` (else period-end fallback) | PIT visibility |

TTM Highlights/Valuation remain useful for **snapshot** panels, not fundies history.

### Price row map

| Layout (`OhlcvRow`) | EODHD EOD field |
|---------------------|-----------------|
| `day` | `date` → `YYYYMMDD` |
| `adj_close` | `adjusted_close` |
| `adj_open` / `adj_high` / `adj_low` | `open|high|low * (adjusted_close/close)` when `close ≠ 0` |
| `volume` | `volume` (EODHD: split-adjusted volume per their docs) |

### Symbol convention

EODHD uses `{CODE}.{EXCHANGE}` (e.g. `AAPL.US`). citrusquant layout keys are bare
`AAPL`. The adapter strips the exchange suffix on write.

### FMP vs EODHD (adapter cost)

| Concern | FMP (`pomelo-fmp`) | EODHD (`pomelo-eodhd`) |
|---------|-------------------|------------------------|
| One-shot CLI in-repo | Yes (`fmp-sync`) | Yes (`eodhd-sync`) |
| Adj OHLC | Dividend-adjusted OHLC endpoint | Reconstruct from close + `adjusted_close` |
| Fundies | Ratios + growth endpoints | Annual statements + local math; multiples NaN hist |
| Delisted + SPX PIT | Yes | Yes (SPX; FMP has more index flags) |
| Snapshot factors | 6 panels incl. piotroski/altman | 4 panels; DIY scores deferred |
| Call economics | Per endpoint | EOD ~1 call/symbol; fundies ~10; screener ~5 |

### Sources (EODHD)

- [EOD historical prices](https://eodhd.com/financial-apis/api-for-historical-data-and-volumes)
- [Fundamentals (stocks / indices / constituents)](https://eodhd.com/financial-apis/stock-etfs-fundamental-data-feeds)
- [Exchange symbol list + delisted](https://eodhd.com/financial-apis/exchanges-api-list-of-tickers-and-trading-hours)
- [Delisted / survivorship notes](https://eodhd.com/financial-academy/financial-faq/survivorship-bias-free-financial-analysis)
- [Screener](https://eodhd.com/financial-apis/stock-market-screener-api)
- [Bulk EOD](https://eodhd.com/financial-apis/bulk-api-eod-splits-dividends)

---

## Open work (track on GitHub)

| Item | Issue |
|------|--------|
| Multi-source stance + assemble docs | [#188](https://github.com/citrusquant/citrusquant/issues/188) — **done** |
| EODHD block coverage gate | [#182](https://github.com/citrusquant/citrusquant/issues/182) — **done** |
| `pomelo-eodhd` + `eodhd-sync` | [#192](https://github.com/citrusquant/citrusquant/issues/192) (phases #193–#198) — **done** |
| Re-audit docs after second path | [#186](https://github.com/citrusquant/citrusquant/issues/186) — **this revision** |
| Finnhub free partial blocks | [#183](https://github.com/citrusquant/citrusquant/issues/183) |
| Parent research / stance | [#180](https://github.com/citrusquant/citrusquant/issues/180) |

---

## Related docs

- [`data-layout.md`](../reference/data-layout) — on-disk contract (source of truth for shapes)
- [`eodhd-data-source.md`](../reference/eodhd-data-source) — EODHD CLI, flags, plans, gaps
- [`fmp-data-source.md`](../reference/fmp-data-source) — FMP Starter vs feature families + `fmp-sync`
- [`backtest-engine.md`](../reference/backtest-engine) — panels / backtest semantics
- crates `pomelo-fmp`, `pomelo-eodhd` — official one-shot syncs

