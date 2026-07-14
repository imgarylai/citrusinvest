---
title: "Data sources"
editUrl: false
sourceFile: docs/data-sources.md
---

<!-- Imported from docs/data-sources.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
> **Status: current** — official adapters FMP + EODHD (#192); AV/Finnhub **solo
> backtest spikes** [#207](https://github.com/citrusquant/citrusquant/issues/207) /
> [#208](https://github.com/citrusquant/citrusquant/issues/208) documented below
> (gaps → impact only; no pricing advice). Parent: [#180](https://github.com/citrusquant/citrusquant/issues/180).
>
> Vendor coverage is research plus what the in-repo adapters implement — not a
> product promise that numbers match across vendors. APIs and endpoint shapes
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

| Block | FMP (`fmp-sync`) | EODHD (`eodhd-sync`) | Alpha Vantage | Finnhub | Tiingo | Polygon | Sharadar SF1 | EDGAR DIY |
|-------|------------------|----------------------|---------------|---------|--------|---------|--------------|-----------|
| Adjusted OHLCV | Y (official) | Y (official\*) | Y / P§ | Y / P¶ | Y | Y | P (separate) | N |
| Fundamentals | Y (official) | Y / P† (official densify) | Y / P§ | Y / P¶ | P | P | Y | DIY |
| Industry map | Y (official) | Y (official) | Y | Y | P / TBD | P / TBD | TBD | extra |
| Delisted | Y (flag) | Y (flag) | Y | P | TBD | TBD | TBD | partial |
| Index PIT | Y (flag) | Y (SPX) / P‡ | **P / weak** | Y | N / weak | N / weak | N | hard |
| Screener | Y (`fmp-symbols`) | Y (`eodhd-symbols`) | P | Y | weak | P | N | N |
| Snapshot scores | Y (6 panels) | P (4 panels; no piotroski/altman) | P / DIY | P / DIY | weak | weak | N | N |

\* EODHD: full **adj OHLC** via local scale; native feed is raw OHLC + `adjusted_close`.  
† Dense historical ratios from annual statements; Highlights/Valuation remain **TTM / current** (used for snapshot factors, not fundies history).  
‡ S&P 500 historical constituents via `GSPC.INDX`; broader index history often Marketplace add-on.  
§ Alpha Vantage: spike [#207](https://github.com/citrusquant/citrusquant/issues/207) — see [§ Alpha Vantage](#alpha-vantage-mapping-spike-207). In-repo path: epic [#209](https://github.com/citrusquant/citrusquant/issues/209) (`av-sync` / `av-symbols`).  
¶ Finnhub: spike [#208](https://github.com/citrusquant/citrusquant/issues/208) — see [§ Finnhub](#finnhub-mapping-spike-208). In-repo path: epic [#210](https://github.com/citrusquant/citrusquant/issues/210) (`finnhub-sync` / `finnhub-symbols`, incl. `--index sp500`). See [`finnhub-data-source.md`](../reference/finnhub-data-source).

**In-repo full sync CLIs today:** FMP, EODHD, Alpha Vantage, and Finnhub.

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

## Alpha Vantage mapping (spike #207)

Research ~2026-07 against [Alpha Vantage documentation](https://www.alphavantage.co/documentation/)
and demo payloads (`INCOME_STATEMENT`, `OVERVIEW`). **No cost advice** — only
whether a solo AV tree can feed honest citrusquant backtests.

### Gate decision

| | |
|--|--|
| **Outcome** | **Go with accepted gaps** → epic [#209](https://github.com/citrusquant/citrusquant/issues/209) (`pomelo-alpha-vantage`) unblocked for phased work |
| **Solo backtest?** | **Yes** for price / TA, statement-densified factors, industry, and delisted-aware universes |
| **Not solo-strong** | Index **membership** PIT, rich screener, vendor piotroski/altman |

### Block coverage → layout → backtest impact

| Block | Verdict | Primary endpoint(s) | → layout | What’s incomplete | Backtest impact if you accept the gap |
|-------|---------|---------------------|----------|-------------------|----------------------------------------|
| Adjusted OHLCV | **Y / P** | `TIME_SERIES_DAILY_ADJUSTED` | `prices/{SYM}.*` | Returns **raw** O/H/L/C + `adjusted close` + dividend + split coefficient (not FMP-style native adj OHLC). Full multi-decade series needs the premium/full surface (compact ≈ last 100 bars). | Price TA / NAV / stops **OK** after local scale `OHLC * (adj_close/close)` (same pattern as EODHD). Wrong if you load raw OHLC as “adjusted.” Compact-only history → short samples only. |
| Fundamentals densify | **Y / P** | `INCOME_STATEMENT`, `BALANCE_SHEET`, `CASH_FLOW` (annual + quarterly); TTM multiples from `OVERVIEW` | `fundamentals/{SYM}.*` | Rows keyed by **`fiscalDateEnding` only** — **no `filing_date` / accepted date** in statement payloads (demo IBM). Historical `pe`/`ps`/`pb`/`market_cap` need DIY from price + shares/EPS, or stay NaN like EODHD densify. | Factor strategies on statement ratios / growth **OK** after densify. **`report_event` / PIT visibility degrades** to period-end (optimistic lookahead vs filing-date truth — same class of risk as FMP/EODHD filing fallback). Multiples history weak unless DIY. |
| Industry map | **Y** | `OVERVIEW` → `Sector`, `Industry` | `tracked/universe.csv.gz` | Sector/industry strings are AV’s taxonomy, not GICS-identical to FMP. | `neutralize_industry` / `industry_rank` **work**; cross-vendor industry labels **not** comparable. |
| Delisted | **Y** | `LISTING_STATUS` (`state=active` / `delisted`, optional as-of `date`) | truncated `prices/` + universe union | Completeness depends on AV’s delisted CSV; still need EOD history for dead tickers. | Survivorship-honest universes **possible** (#26) if you union delisted names and fetch their bars. Active-only lists → survivor bias. |
| Index PIT | **P / weak** | Index **price** APIs (premium index suite: SPX, etc.) | hard to get `panels/in_sp500` | Index series ≠ **constituent membership over time**. No first-class “historical SPX members” map comparable to FMP/EODHD/Finnhub. | `signal * in_sp500`-style **index-honest** backtests **not** available from AV alone without external membership DIY. Price strategies without membership **unaffected**. |
| Screener | **P** | `LISTING_STATUS&state=active` via `yuzu-cli av-symbols` (exchange/assetType filter); no cap screener | symbol list | Not FMP/EODHD-style market-cap screener. | Build a list with `av-symbols`, then `av-sync --symbols-file`. |
| Snapshot factors | **P / DIY** | `OVERVIEW` (`AnalystTargetPrice`, `AnalystRating*`, TTM PE, …); statements for DIY scores | `panels/*` optional | No vendor piotroski/altman. Ratings are **counts**, not FMP grades-summary labels. Current snapshot semantics only. | Screening-style factors possible with DIY; **deep historical** snapshot panels **no**. Missing panels → those `Data` names NaN (ops no-op / empty), not engine crash. |

### Solo “can I backtest?” matrix (AV only)

| Strategy family | Honest on AV-only? | Notes |
|-----------------|--------------------|-------|
| Price TS / OHLCV TA / rotation on price | **Yes** | After adj OHLC reconstruction + full history access |
| CS on price ranks | **Yes** | |
| Statement factor densify (roe, margins, growth, …) | **Yes, degraded PIT** | Period-end visibility unless you add external filing dates |
| Industry neutralize / rank | **Yes** | Taxonomy-specific |
| Delist haircuts | **Yes** if `LISTING_STATUS` + dead-name prices included | |
| Index-member-only (SPX PIT) | **No** (without external membership) | Largest structural hole vs FMP/EODHD/Finnhub; **no** `av-sync --index` (would be dishonest) |
| Snapshot piotroski/altman/history | **No / DIY only** | |
| Universe list helper | **Partial** | `yuzu-cli av-symbols` = active `LISTING_STATUS` + filters (#217) |

### Sources (Alpha Vantage)

- [API documentation](https://www.alphavantage.co/documentation/) — daily adjusted, fundamentals, listing status, overview  
- Demo checks: `OVERVIEW` / `INCOME_STATEMENT` for IBM (fiscal periods present; no filing timestamp on statements)

---

## Finnhub mapping (spike #208)

Research ~2026-07 against [Finnhub API docs](https://finnhub.io/docs/api) and
public endpoint descriptions. Capability notes may mark free vs paid **access**
only so implementers know what a key must unlock — **not** product pricing advice.

Widens free-tier demo spike [#183](https://github.com/citrusquant/citrusquant/issues/183)
to full solo-backtest completeness.

### Gate decision

| | |
|--|--|
| **Outcome** | **Go with accepted gaps** → epic [#210](https://github.com/citrusquant/citrusquant/issues/210) (`pomelo-finnhub`) unblocked |
| **Solo backtest?** | **Yes** for prices (with adjust care), fundies (stronger filing dates via as-reported), industry, **index historical constituents**, screener |
| **Watch** | Candle **adjust** semantics; delisted path thinner than AV `LISTING_STATUS`; free-tier history windows force multi-call stitching |

### Block coverage → layout → backtest impact

| Block | Verdict | Primary endpoint(s) | → layout | What’s incomplete | Backtest impact if you accept the gap |
|-------|---------|---------------------|----------|-------------------|----------------------------------------|
| Adjusted OHLCV | **Y / P** | `stock/candle` (`resolution=D`, `from`/`to`; optional **adjusted** flag on the candle API) | `prices/{SYM}.*` | Unadjusted candles if flag omitted. Free/low tiers often **cap range per request** (commonly ~1y daily — stitch windows). International depth/latency differs from US. | Price/TA **OK** when adjusted series is requested and windows stitched. Using unadjusted OHLC → split-distorted returns, stops, TA. Short free windows → incomplete history if not looped. |
| Fundamentals densify | **Y / P** | Standardized `stock/financials`; **`stock/financials-reported`** (as-reported + **filedDate**); `stock/metric` / basic financials for TTM / series | `fundamentals/{SYM}.*` | Standardized vs as-reported field names differ; some deep series are plan-gated. Need local ratio/YoY math into `FUNDAMENTAL_FIELDS`. | Factor densify **OK**. **`report_event` can track filing** when using as-reported `filedDate` — **better PIT story than AV**. Missing standardized history → thinner factor columns. |
| Industry map | **Y** | `stock/profile` / `stock/profile2` (`finnhubIndustry`, sector-like fields) | `tracked/universe.csv.gz` | Taxonomy ≠ FMP/AV. | Industry ops **work**; don’t mix vendor industry strings mid-sample. |
| Delisted | **P** | Exchange symbol lists / profile status; no single “delisted CSV” as clean as AV `LISTING_STATUS` | truncated `prices/` | Harder to enumerate dead names exhaustively from one call. | Survivorship **degrades** unless you maintain an external dead-name list or accept survivor-only universes. |
| Index PIT | **Y** | `index/constituents` + **`index/historical-constituents`** (e.g. `^GSPC`) | `panels/in_sp500.csv.gz` | Quality/depth can thin further back in time (vendor-dependent). Some access is plan-gated. | **Strong solo fit** for index-member strategies — main reason Finnhub can beat AV for SPX-honest research without FMP/EODHD. |
| Screener | **Y** | `stock/screener` (filters: exchange, cap, …) | symbol list | Often plan-gated; filter surface differs from FMP. | Universe discovery **OK** when endpoint is unlocked; else BYO symbol file. |
| Snapshot factors | **P / DIY** | `stock/metric`, recommendation trends, price targets | `panels/*` optional | No drop-in piotroski/altman. Current vs historical metric series varies by field. | DIY current screens possible; not FMP’s six-panel set. |

### Solo “can I backtest?” matrix (Finnhub only)

| Strategy family | Honest on Finnhub-only? | Notes |
|-----------------|-------------------------|-------|
| Price TS / OHLCV TA / rotation | **Yes** | Must use **adjusted** candles + stitch ranges |
| CS on price ranks | **Yes** | |
| Statement factors + filing-aware `report_event` | **Yes** | Prefer financials-reported for visibility |
| Industry neutralize / rank | **Yes** | |
| Delist haircuts | **Partial** | Weaker than AV/FMP/EODHD delist feeds |
| Index-member-only (SPX PIT) | **Yes** (relative strength) | Best non-FMP/EODHD story for membership |
| Snapshot score panels | **DIY / partial** | |

### Free-tier demo note (#183)

Free keys can still fill **partial** trees for small US universes (quotes/candles +
thin fundies) under rate limits and short candle windows — good for demos, not a
claim of full-market solo production parity. Completeness above is about **API
capability**, not free quotas.

### Sources (Finnhub)

- [API documentation](https://finnhub.io/docs/api) — candles, financials, profile, screener  
- [Indices historical constituents](https://finnhub.io/docs/api/indices-historical-constituents)  
- [Financials as reported](https://finnhub.io/docs/api/financials-reported)

---

## Open work (track on GitHub)

| Item | Issue |
|------|--------|
| Multi-source stance + assemble docs | [#188](https://github.com/citrusquant/citrusquant/issues/188) — **done** |
| EODHD block coverage gate | [#182](https://github.com/citrusquant/citrusquant/issues/182) — **done** |
| `pomelo-eodhd` + `eodhd-sync` | [#192](https://github.com/citrusquant/citrusquant/issues/192) (phases #193–#198) — **done** |
| Re-audit docs after second path | [#186](https://github.com/citrusquant/citrusquant/issues/186) — **done** |
| Alpha Vantage solo coverage spike | [#207](https://github.com/citrusquant/citrusquant/issues/207) — **done** |
| `pomelo-alpha-vantage` + CLI | [#209](https://github.com/citrusquant/citrusquant/issues/209) — phases #213–#218 |
| Finnhub solo coverage spike | [#208](https://github.com/citrusquant/citrusquant/issues/208) — **done** |
| `pomelo-finnhub` (gated on #208 go) | [#210](https://github.com/citrusquant/citrusquant/issues/210) |
| Shared `pomelo-*` adapter conventions | [#211](https://github.com/citrusquant/citrusquant/issues/211) |
| Finnhub free partial call-budget demos | [#183](https://github.com/citrusquant/citrusquant/issues/183) |
| Parent research / stance | [#180](https://github.com/citrusquant/citrusquant/issues/180) |

---

## Related docs

- [`data-layout.md`](../reference/data-layout) — on-disk contract (source of truth for shapes)
- [`eodhd-data-source.md`](../reference/eodhd-data-source) — EODHD CLI, flags, gaps
- [`alpha-vantage-data-source.md`](../reference/alpha-vantage-data-source) — AV CLI, flags, gaps
- [`finnhub-data-source.md`](../reference/finnhub-data-source) — Finnhub CLI, flags, index PIT, gaps
- [`fmp-data-source.md`](../reference/fmp-data-source) — FMP Starter vs feature families + `fmp-sync`
- [`backtest-engine.md`](../reference/backtest-engine) — panels / backtest semantics
- crates `pomelo-fmp`, `pomelo-eodhd`, `pomelo-alpha-vantage` — official one-shot syncs

