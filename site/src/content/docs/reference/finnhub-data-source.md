---
title: "Finnhub data source"
editUrl: false
sourceFile: docs/finnhub-data-source.md
---

<!-- Imported from docs/finnhub-data-source.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
Bring-your-own-key sync via **`pomelo-finnhub`** / **`yuzu-cli finnhub-sync`**
(epic [#210](https://github.com/citrusquant/citrusquant/issues/210)).

The engine never calls Finnhub. This crate writes the same
[`data-layout`](../reference/data-layout) tree as FMP/EODHD/Alpha Vantage. Stance, block
comparison, and assemble recipes: [`data-sources.md`](../reference/data-sources).

**Not a commitment that Finnhub numbers match FMP, EODHD, or Alpha Vantage.**
Definitions and coverage differ. **No pricing advice** — only data gaps and
backtest impact. Your key never leaves the machine; no Finnhub data is
redistributed.

---

## CLI cheat sheet

```bash
export FINNHUB_API_KEY=…          # or --api-key

# Prices only
yuzu-cli finnhub-sync --out ./mydata --symbols AAPL,MSFT \
  --from 20200101 --to 20251231

# Prices + fundies + industry + snapshot factors
yuzu-cli finnhub-sync --out ./mydata --symbols-file ./u.txt \
  --from 20200101 --to 20251231 \
  --include-fundamentals --include-industry --include-snapshot-factors

# S&P 500 point-in-time universe → prices for ever-members + panels/in_sp500
yuzu-cli finnhub-sync --out ./mydata --index sp500 \
  --from 20150101 --to 20251231

# Exchange listing → symbol file (not a market-cap screener)
yuzu-cli finnhub-symbols --out ./u.txt --exchange US \
  --security-type "Common Stock" --limit 500
```

`--out` accepts a local path or `s3://bucket[/prefix]` (same credentials as
`fmp-sync` / `eodhd-sync`). **`--index` needs a local `--out`** — the membership
panel is placed on the synced tree's trading calendar.

---

## What each flag writes

| Flag / mode | Layout output |
|-------------|----------------|
| (default) | `prices/{SYM}.csv.gz` — adjusted OHLC from `/stock/candle` (`resolution=D`, `adjusted=true`) |
| `--include-fundamentals` | `fundamentals/{SYM}.csv.gz` — annual `financials-reported` densify + `report_event` on the real **`filedDate`** |
| `--include-industry` | `tracked/universe.csv.gz` — `/stock/profile2` `finnhubIndustry` + market cap |
| `--include-snapshot-factors` | `panels/analyst_upside_pct`, `consensus_rating`, `fcf_yield`, `pe_industry_pctile` |
| `--index sp500` | `panels/in_sp500.csv.gz` + syncs the index's ever-members over `[from,to]` |
| `finnhub-symbols` | text symbol list from `/stock/symbol` (+ security-type / limit filters) |

---

## Where Finnhub is strong

| Topic | Finnhub behavior |
|-------|------------------|
| **Index PIT** | **Real historical constituents** (`index/constituents` + `index/historical-constituents`) → honest `panels/in_sp500.csv.gz`. This is the reason to pick Finnhub over Alpha Vantage, which cannot write a truthful membership panel. |
| **Filing-aware fundamentals** | `financials-reported` carries **`filedDate`**, so `report_event` fires on the actual SEC disclosure date — a stronger PIT story than AV's fiscal period-end. |

---

## Gaps / honesty notes

| Topic | Finnhub behavior |
|-------|------------------|
| Adjusted OHLC | `adjusted=true` returns already-adjusted candles (no local rescale). **Unadjusted risk:** some plans ignore the flag → split-distorted OHLC. Free/low tiers cap the range per request → stitch windows with `--from`/`--to` + `--append`. |
| Fundamentals field mapping | As-reported US-GAAP concept tags matched by concept tail (candidate list per field); coverage varies by filer. `pe`/`ps`/`pb`/`market_cap` in the fundies CSV are **NaN** (no historical price/shares join). |
| `eps_growth` | Proxy = **net income YoY**. |
| Index coverage | **S&P 500 only** (`^GSPC`) in v1; changelog depth thins in older history, so membership is reliable in recent years and degrades further back. |
| Delisted | **No clean `LISTING_STATUS`-style dead-name feed.** A Finnhub-only universe is survivor-biased unless you supply an external dead-name list or drive it from `--index` (whose ever-members include names that later left). No `--include-delisted`. |
| Snapshot factors | **Current-as-of** the last bar; price targets and some metrics are **plan-gated**, so a series is simply absent when the endpoint returns nothing — never faked. |
| `consensus_rating` | Weighted mean of the latest `/stock/recommendation` count buckets (StrongBuy=1 … StrongSell=5). |
| `pe_industry_pctile` | Cohort = **this run's symbols** only (need ≥5 in an industry); industry from `finnhubIndustry` (its own taxonomy — don't mix vendor industry strings mid-sample). |
| piotroski / altman | **Not written.** |
| Numbers vs other vendors | Not bit-identical. |

For which lemon ops need which panels, see [`fmp-data-source.md`](../reference/fmp-data-source)
— panel requirements are vendor-agnostic.

---

## Architecture

```text
yuzu-cli finnhub-sync     →  pomelo-finnhub::sync_into      →  ObjectSink (local / S3)
yuzu-cli finnhub-sync --index  →  index/historical-constituents → panels/in_sp500
yuzu-cli finnhub-symbols  →  /stock/symbol listing → symbol file
yuzu-core / lemon         →  only panels on disk
```

- Crate: `crates/pomelo-finnhub`
- Mapping research: [`data-sources.md`](../reference/data-sources) § Finnhub
- Parent epic: [#210](https://github.com/citrusquant/citrusquant/issues/210)

---

## Related

- [`data-layout.md`](../reference/data-layout) — file shapes and series names  
- [`data-sources.md`](../reference/data-sources) — multi-source stance + AV/Finnhub spikes  
- [`fmp-data-source.md`](../reference/fmp-data-source) · [`eodhd-data-source.md`](../reference/eodhd-data-source) · [`alpha-vantage-data-source.md`](../reference/alpha-vantage-data-source)  

