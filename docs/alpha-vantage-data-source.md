# Alpha Vantage data source for citrusquant

Bring-your-own-key sync via **`pomelo-alpha-vantage`** / **`yuzu-cli av-sync`**
(epic [#209](https://github.com/citrusquant/citrusquant/issues/209)).

The engine never calls Alpha Vantage. This crate writes the same
[`data-layout`](data-layout.md) tree as FMP/EODHD. Stance, block comparison, and
assemble recipes: [`data-sources.md`](data-sources.md).

**Not a commitment that Alpha Vantage numbers match FMP or EODHD.** Definitions
and coverage differ. **No pricing advice** — only data gaps and backtest impact.

---

## CLI cheat sheet

```bash
export ALPHA_VANTAGE_API_KEY=…   # or ALPHAVANTAGE_API_KEY / --api-key

# Prices only
yuzu-cli av-sync --out ./mydata --symbols AAPL,MSFT \
  --from 20200101 --to 20251231

# Prices + fundies + industry + snapshot factors
yuzu-cli av-sync --out ./mydata --symbols-file ./u.txt \
  --from 20200101 --to 20251231 \
  --include-fundamentals --include-industry --include-snapshot-factors

# Delisted union (survivorship)
yuzu-cli av-sync --out ./mydata --symbols AAPL --include-delisted

# Active listing → symbol file (not a market-cap screener; no index PIT)
yuzu-cli av-symbols --out ./u.txt --exchange NASDAQ --asset-type Stock --limit 500
```

`--out` accepts a local path or `s3://bucket[/prefix]` (same credentials as
`fmp-sync` / `eodhd-sync`).

**There is no `--index`.** Alpha Vantage does not provide historical index
constituents; we do not invent `panels/in_sp500.csv.gz`.

---

## What each flag writes

| Flag / mode | Layout output |
|-------------|----------------|
| (default) | `prices/{SYM}.csv.gz` — adj OHLC via `adjusted_close/close` scale (`TIME_SERIES_DAILY_ADJUSTED`, `outputsize=full`) |
| `--include-fundamentals` | `fundamentals/{SYM}.csv.gz` — annual IS/BS densify + `report_event` on **fiscal period-end** |
| `--include-industry` | `tracked/universe.csv.gz` — OVERVIEW Sector + market cap |
| `--include-delisted` | unions `LISTING_STATUS&state=delisted` into the symbol list before price fetch |
| `--include-snapshot-factors` | `panels/analyst_upside_pct`, `consensus_rating`, `fcf_yield`, `pe_industry_pctile` |
| `av-symbols` | text symbol list from `LISTING_STATUS&state=active` (+ exchange/assetType filters) |

---

## Gaps vs FMP / honesty notes

| Topic | Alpha Vantage behavior |
|-------|------------------------|
| Adj OHLC | Raw OHLC scaled by `adjusted_close/close` (same policy as EODHD) |
| Full history | Needs a key that serves `outputsize=full` on daily adjusted |
| pe / ps / pb / market_cap in fundies CSV | **NaN historically** (statement densify only) |
| `report_event` / PIT | **Period-end only** — AV statements have no `filing_date` (optimistic) |
| eps_growth | Proxy = **net income YoY** |
| Snapshot factors | **Current-as-of** last bar; OVERVIEW + latest CF free cash flow |
| piotroski / altman | **Not written** |
| consensus_rating | Weighted mean of AnalystRating* counts (StrongBuy=1 … StrongSell=5) |
| pe_industry_pctile | Cohort = **this run’s symbols** only (need ≥5 in industry) |
| Index PIT | **Not available** — no historical constituents API |
| Universe helper | Active listing CSV, **not** cap-sorted screener |
| Numbers vs FMP | Not bit-identical |

For which lemon ops need which panels, see [`fmp-data-source.md`](fmp-data-source.md)
— panel requirements are vendor-agnostic.

---

## Architecture

```text
yuzu-cli av-sync      →  pomelo-alpha-vantage::sync_into  →  ObjectSink (local / S3)
yuzu-cli av-symbols   →  LISTING_STATUS active → symbol file
yuzu-core / lemon     →  only panels on disk
```

- Crate: `crates/pomelo-alpha-vantage`
- Mapping research: [`data-sources.md`](data-sources.md) § Alpha Vantage
- Parent epic: [#209](https://github.com/citrusquant/citrusquant/issues/209)

---

## Related

- [`data-layout.md`](data-layout.md) — file shapes and series names  
- [`data-sources.md`](data-sources.md) — multi-source stance + AV/Finnhub spikes  
- [`fmp-data-source.md`](fmp-data-source.md) · [`eodhd-data-source.md`](eodhd-data-source.md)  
