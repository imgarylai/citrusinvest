---
title: "Data sources (WIP)"
editUrl: false
sourceFile: docs/data-sources.md
---

<!-- Imported from docs/data-sources.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
> **Status: WIP** — tracking parent
> [#180](https://github.com/citrusquant/citrusquant/issues/180).
>
> Vendor coverage is research, not a product promise. APIs, tiers, and pricing
> change — re-check before you build a pipeline. Official one-shot sync today is
> still only `yuzu-cli fmp-sync`.
>
> **EODHD block mapping** (spike [#182](https://github.com/citrusquant/citrusquant/issues/182)):
> researched below. Other vendors may still show **TBD**.

## What is decided

1. **Contract = [`data-layout`](../reference/data-layout).** The engine never calls a
   market-data vendor.
2. **Official one-shot sync today:** `pomelo-fmp` / `yuzu-cli fmp-sync` only.
3. **Not locked to FMP:** you may BYO files that match the layout, or
   **assemble** a tree from multiple sources (extra steps OK).
4. A second full `pomelo-XXX` adapter lands **only if** some vendor can cover
   roughly the same **data blocks** as FMP (optional; not the main path).
5. **No** cross-vendor mega `sync-all --vendor=…`.

FMP Starter **feature** gaps (which lemon ops need which panels): see
[`fmp-data-source.md`](../reference/fmp-data-source).

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
| Snapshot factors | `panels/{name}.*` | No | e.g. piotroski, altman_z — often derived or vendor-specific |

---

## Block → candidate vendors (draft)

Legend: **Y** = typically usable · **P** = partial / plan-dependent · **N** =
essentially no · **TBD** = not verified here yet.

| Block | FMP (`fmp-sync`) | EODHD | Finnhub | Tiingo | Polygon | Sharadar SF1 | EDGAR DIY |
|-------|------------------|-------|---------|--------|---------|--------------|-----------|
| Adjusted OHLCV | Y (official) | Y\* | Y | Y | Y | P (separate) | N |
| Fundamentals | Y (official) | Y / P† | Y | P | P | Y | DIY |
| Industry map | Y (official) | Y | Y | P / TBD | P / TBD | TBD | extra |
| Delisted | Y (flag) | Y | P / TBD | TBD | TBD | TBD | partial |
| Index PIT | Y (flag) | Y / P‡ | Y | N / weak | N / weak | N | hard |
| Screener | Y (`fmp-symbols`) | Y | Y | weak | P | N | N |
| Snapshot scores | Y (flag) | P | P / TBD | weak | weak | N | N |

\* EODHD: full **adj OHLC** needs a local scale (see [EODHD mapping](#eodhd-mapping-spike-182)); native feed is raw OHLC + `adjusted_close`.  
† Dense historical ratios need deriving from annual/quarterly statements; Highlights/Valuation are mostly **TTM / current**.  
‡ S&P 500 historical constituents in Fundamentals package; broader index history often Marketplace add-on.

**Only FMP has an in-repo full sync CLI today.** Other columns mean “you can
often buy/fetch this block and write the layout yourself,” not “we ship an
adapter.”

---

## How to get a dataset

| Path | What you do | Status |
|------|-------------|--------|
| **One-shot (official)** | `yuzu-cli fmp-sync …` → data root | Supported |
| **BYO** | Write the [data-layout](../reference/data-layout) tree yourself | Supported |
| **Assemble** | Fill blocks from different vendors into **one** data root | Supported as a pattern; recipes below are sketches |
| **Second full adapter** | `pomelo-eodhd` + `yuzu-cli eodhd-sync` (epic [#192](https://github.com/citrusquant/citrusquant/issues/192)) | Full path live (#193–#198); see [`eodhd-data-source.md`](../reference/eodhd-data-source) |

Engine / `pomelo-data` only need the finished tree (local or S3 via
`pomelo-s3`). Optional quality pass: `yuzu-cli data-audit`.

---

## Assemble recipes (WIP)

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

**Goal:** factor-style series without a full FMP tree.

1. **Prices** from a price-strong vendor (e.g. Tiingo / Polygon / EODHD) →
   `prices/`.
2. **Fundamentals** from a fundies-strong source (e.g. EODHD / Sharadar /
   EDGAR-derived ratios) → `fundamentals/{SYM}.*` with the column set in
   [data-layout](../reference/data-layout) § fundamentals.
3. Align calendars / symbols yourself (same tickers, overlapping `day` range).
4. Optional: industry CSV under `tracked/`; optional: delisted names as
   truncated price files.

See [EODHD mapping](#eodhd-mapping-spike-182) if EODHD is one of the sources.

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

---

## EODHD mapping (spike #182)

Research date ~2026-07. Verified against EODHD public docs + `api_token=demo`
sample payloads for `AAPL.US` (not a full paid-universe audit). Plans: EOD
history + **Fundamentals** package (and higher) for fundies/index; free tier is
too thin for full-market sync.

### Gate decision

| | |
|--|--|
| **Outcome** | **Near-FMP core blocks, with accepted gaps** |
| **Full adapter (#185)** | Coverage no longer “unknown.” Optional product work — still **not** mainline; assemble/BYO remains the non-FMP path. |
| **Accepted gaps** | Full adj OHLC reconstruction; historical dense factors need statement math; snapshot scores mostly DIY; multi-index deep history may need Marketplace. |

### Block coverage

| Block | Verdict | EODHD surface | → data-layout | Notes / caveats |
|-------|---------|---------------|---------------|-----------------|
| Adjusted OHLCV | **Y** (with transform) | `GET /api/eod/{SYM}.{EX}` e.g. `AAPL.US` | `prices/{SYM}.csv.gz` | Fields: `date`, `open`, `high`, `low`, `close`, `adjusted_close`, `volume`. OHLC are **not** dividend-adjusted; only `adjusted_close` is split+dividend adjusted. For citrusquant’s full adj OHLC, scale OHLC by `adjusted_close/close` (or use Technical API `splitadjusted` and accept dividend policy). Bulk: `/api/eod-bulk-last-day/{EX}` for daily refresh. |
| Fundamentals | **Y / P** | `GET /api/v1.1/fundamentals/{SYM}.{EX}` (10 API calls / request) | `fundamentals/{SYM}.*` | **TTM/current:** `Highlights` / `Valuation` (PE, PS, PB, ROE, margins, market cap, …). **Historical densify:** `Financials.Income_Statement` / `Balance_Sheet` / `Cash_Flow` yearly|quarterly with `date` + **`filing_date`** (good for `report_event` / PIT visibility). Growth fields and several ratios need local YoY / ratio math — not a drop-in of FMP’s ratios endpoints. Bulk fundies: Extended plan `/api/v1.1/bulk-fundamentals/{EX}`. |
| Industry map | **Y** | Same fundamentals `General.Sector`, `General.Industry` (+ GICS fields) | `tracked/universe.csv.gz` | Sector string → industry map like FMP profile. Type flags (`Common Stock` / `ETF` / `FUND`) for stock-only screens. |
| Delisted | **Y** | `GET /api/exchange-symbol-list/{EX}?delisted=1` + EOD/fundamentals on those codes; `General.IsDelisted` / delisted date on fundamentals | truncated `prices/` files | Documented survivorship path; same EOD endpoints for inactive tickers. |
| Index PIT | **Y / P** | Fundamentals on index e.g. `GSPC.INDX`: current `Components`; `historical=1` → `HistoricalComponents` snapshots | `panels/in_sp500.csv.gz` | S&P 500 historical constituents in Fundamentals package (snapshots by date; quality stronger in recent decades). Other indices: current components widely; multi-year multi-index history often **Marketplace** S&P/DJ product. |
| Screener | **Y** | `GET /api/screener?filters=…` (5 API calls / request) | symbol list for sync | Filters: exchange, sector, industry, `market_capitalization`, etc. Limit/offset pagination (max limit 100). |
| Snapshot scores | **P** | No ready piotroski/altman feed; `AnalystRatings.TargetPrice` / `Rating`; statements for DIY scores | `panels/*` optional | `analyst_upside_pct` ≈ target vs close; consensus scale ≠ FMP grades-summary labels. `piotroski_score` / `altman_z` / `fcf_yield` / `pe_industry_pctile` = derive (engine already has pure helpers in `pomelo-fmp::factors` for some). |

### Suggested field map (`FUNDAMENTAL_FIELDS`)

Layout columns from `pomelo-data` (`pe`, `ps`, …). EODHD is **not** 1:1 with FMP
JSON names.

| Layout column | Primary EODHD source (draft) | Quality |
|---------------|------------------------------|---------|
| `pe` | `Highlights.PERatio` / `Valuation.TrailingPE` | TTM snapshot; historical: DIY from price + EPS |
| `ps` | `Valuation.PriceSalesTTM` | TTM |
| `pb` | `Valuation.PriceBookMRQ` | MRQ |
| `roe` | `Highlights.ReturnOnEquityTTM` | TTM |
| `net_margin` | `Highlights.ProfitMargin` | TTM |
| `debt_to_equity` | DIY BS `totalLiab` / `totalStockholderEquity` (or net debt) | needs definition parity vs FMP |
| `market_cap` | `Highlights.MarketCapitalization` | current; hist market-cap API exists separately |
| `gross_margin` | DIY IS `grossProfit` / revenue | historical OK |
| `receivables_turnover` | DIY if receivables + revenue present | often thin |
| `debt_to_assets` | DIY BS debt / `totalAssets` | historical OK |
| `revenue` | IS `totalRevenue` (name may vary) | yearly/quarterly + `filing_date` |
| `revenue_growth` … `gross_profit_growth` | YoY on IS lines | local compute |
| `report_event` | `filing_date` on financial rows | **good** for PIT |

### Price row map

| Layout (`OhlcvRow`) | EODHD EOD field |
|---------------------|-----------------|
| `day` | `date` → `YYYYMMDD` |
| `adj_close` | `adjusted_close` |
| `adj_open` / `adj_high` / `adj_low` | `open|high|low * (adjusted_close/close)` when `close ≠ 0` |
| `volume` | `volume` (EODHD: split-adjusted volume per their docs) |

### Symbol convention

EODHD uses `{CODE}.{EXCHANGE}` (e.g. `AAPL.US`). citrusquant layout keys are bare
`AAPL`. Strip exchange suffix on write; keep a side map if you need round-trips.

### FMP vs EODHD (adapter cost)

| Concern | FMP (`pomelo-fmp`) | EODHD |
|---------|-------------------|--------|
| One-shot CLI in-repo | Yes | No |
| Adj OHLC | Dividend-adjusted OHLC endpoint | Reconstruct from close + `adjusted_close` |
| Fundies | Ratios + growth endpoints, already mapped | Rich JSON; **more local math** |
| Delisted + SPX PIT | Yes | Yes (SPX hist in fundies package) |
| Snapshot factors | financial-scores / grades / targets wired | Mostly DIY + analyst block |
| Call economics | Per endpoint | EOD 1 call/symbol; fundies **10 calls**/symbol; screener 5 |

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
| This doc + recipes | [#188](https://github.com/citrusquant/citrusquant/issues/188) (merged baseline) |
| EODHD block coverage gate | [#182](https://github.com/citrusquant/citrusquant/issues/182) — **mapping landed here** |
| Finnhub free partial blocks | [#183](https://github.com/citrusquant/citrusquant/issues/183) |
| `pomelo-eodhd` + `eodhd-sync` epic | [#192](https://github.com/citrusquant/citrusquant/issues/192) (phases #193–#198); #185 superseded |
| Re-audit docs after paths mature | [#186](https://github.com/citrusquant/citrusquant/issues/186) |
| Parent research / stance | [#180](https://github.com/citrusquant/citrusquant/issues/180) |

---

## Related docs

- [`data-layout.md`](../reference/data-layout) — on-disk contract (source of truth for shapes)
- [`eodhd-data-source.md`](../reference/eodhd-data-source) — EODHD CLI, flags, plans, gaps
- [`fmp-data-source.md`](../reference/fmp-data-source) — FMP Starter vs feature families
- [`backtest-engine.md`](../reference/backtest-engine) — panels / backtest semantics
- crates `pomelo-fmp`, `pomelo-eodhd` — official one-shot syncs

