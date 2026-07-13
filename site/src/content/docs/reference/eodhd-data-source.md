---
title: "EODHD data source"
editUrl: false
sourceFile: docs/eodhd-data-source.md
---

<!-- Imported from docs/eodhd-data-source.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
Bring-your-own-key sync via **`pomelo-eodhd`** / **`yuzu-cli eodhd-sync`** (epic
[#192](https://github.com/citrusquant/citrusquant/issues/192)).

The engine never calls EODHD. This crate writes the same
[`data-layout`](../reference/data-layout) tree as FMP. Multi-vendor / assemble options:
[`data-sources.md`](../reference/data-sources).

**Not a commitment that EODHD numbers match FMP.** Definitions and coverage differ.

---

## CLI cheat sheet

```bash
export EODHD_API_TOKEN=…   # or EODHD_API_KEY / --api-token

# Prices only
yuzu-cli eodhd-sync --out ./mydata --symbols AAPL,MSFT \
  --from 20200101 --to 20251231

# Prices + fundies + industry + snapshot factors
yuzu-cli eodhd-sync --out ./mydata --symbols-file ./u.txt \
  --from 20200101 --to 20251231 \
  --include-fundamentals --include-industry --include-snapshot-factors

# Delisted union (survivorship)
yuzu-cli eodhd-sync --out ./mydata --symbols AAPL --include-delisted

# S&P 500 ever-members + panels/in_sp500.csv.gz (local --out only)
yuzu-cli eodhd-sync --out ./spx --index sp500 --from 20200101 --to 20241231

# Screener → symbol file
yuzu-cli eodhd-symbols --out ./u.txt --min-market-cap 1b --exchange us --limit 200
```

`--out` accepts a local path or `s3://bucket[/prefix]` (same credentials as
`fmp-sync`). **`--index` requires a local path.**

---

## What each flag writes

| Flag / mode | Layout output |
|-------------|----------------|
| (default) | `prices/{SYM}.csv.gz` — adj OHLC via `adjusted_close/close` scale |
| `--include-fundamentals` | `fundamentals/{SYM}.csv.gz` — annual densify + `report_event` |
| `--include-industry` | `tracked/universe.csv.gz` — sector + market cap |
| `--include-delisted` | unions dead names into the symbol list before price fetch |
| `--include-snapshot-factors` | `panels/analyst_upside_pct`, `consensus_rating`, `fcf_yield`, `pe_industry_pctile` |
| `--index sp500` | sync ever-members + `panels/in_sp500.csv.gz` |

---

## Plans, calls, limits (verify on EODHD site)

| Need | Typical plan surface |
|------|----------------------|
| EOD prices | EOD Historical (or higher) |
| Fundamentals / industry / fundies densify / snapshot | Fundamentals package (or All-In-One) |
| Index historical constituents | Fundamentals (`GSPC.INDX`) |
| Screener | All-In-One / extended plans (check pricing) |

Rough call costs (EODHD docs; subject to change):

| Endpoint | Cost |
|----------|------|
| EOD one symbol (any length) | ~1 call |
| Fundamentals one symbol | ~10 calls |
| Screener request | ~5 calls |
| Free tier | ~20 calls/day — demos only |

Token stays on the machine; we do not host or redistribute EODHD data.

---

## Gaps vs FMP / honesty notes

| Topic | EODHD behavior |
|-------|----------------|
| Adj OHLC | Raw OHLC scaled by `adjusted_close/close` (not FMP’s native adj OHLC) |
| pe / ps / pb / market_cap in fundies CSV | **NaN historically** (statement densify only) |
| eps_growth | Proxy = **net income YoY** (IS has no EPS field in many payloads) |
| Snapshot factors | **Current-as-of** last bar, not multi-year history |
| piotroski / altman | **Not written** (no vendor scores; DIY deferred) |
| consensus_rating | EODHD `Rating` (higher ≈ bullish) mapped as `6 − rating` → 1…5 lower-bullish |
| pe_industry_pctile | Cohort = **this run’s symbols** only (need ≥5 in industry) |
| Index | SPX first; other indices not in CLI v1 |
| Numbers vs FMP | Not bit-identical — different vendors |

For FMP Starter feature honesty (which lemon ops need which panels), see
[`fmp-data-source.md`](../reference/fmp-data-source) — the same panel requirements apply
regardless of vendor.

---

## Architecture

```text
yuzu-cli eodhd-sync  →  pomelo-eodhd::sync_into  →  ObjectSink (local / S3)
yuzu-core / lemon    →  only panels on disk
```

- Crate: `crates/pomelo-eodhd`
- Mapping research: [`data-sources.md`](../reference/data-sources) § EODHD
- Parent epic: [#192](https://github.com/citrusquant/citrusquant/issues/192)

---

## Related

- [`data-layout.md`](../reference/data-layout) — file shapes and series names  
- [`data-sources.md`](../reference/data-sources) — multi-source stance  
- [`fmp-data-source.md`](../reference/fmp-data-source) — FMP path + Starter gaps  

