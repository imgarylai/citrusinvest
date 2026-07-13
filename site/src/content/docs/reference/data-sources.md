---
title: "Data sources (WIP)"
editUrl: false
sourceFile: docs/data-sources.md
---

<!-- Imported from docs/data-sources.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
> **Status: WIP / TBD** — tracking
> [#188](https://github.com/citrusquant/citrusquant/issues/188)
> (parent [#180](https://github.com/citrusquant/citrusquant/issues/180)).
>
> This page is a **draft**. Vendor rows, coverage marks, and assemble recipes
> are research notes, not product promises. APIs, tiers, and pricing change —
> re-check each vendor before you build a pipeline. Gaps and “TBD” cells mean
> we have not finished mapping yet.

## What is decided

1. **Contract = [`data-layout`](../reference/data-layout).** The engine never calls a
   market-data vendor.
2. **Official one-shot sync today:** `pomelo-fmp` / `yuzu-cli fmp-sync` only.
3. **Not locked to FMP:** you may BYO files that match the layout, or
   **assemble** a tree from multiple sources (extra steps OK).
4. A second full `pomelo-XXX` adapter lands **only if** some vendor can cover
   roughly the same **data blocks** as FMP. That is optional and unlikely to be
   the main path.
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
| Adjusted OHLCV | Y (official) | Y | Y | Y | Y | P (separate) | N |
| Fundamentals | Y (official) | Y | Y | P | P | Y | DIY |
| Industry map | Y (official) | Y | Y | P / TBD | P / TBD | TBD | extra |
| Delisted | Y (flag) | Y | P / TBD | TBD | TBD | TBD | partial |
| Index PIT | Y (flag) | Y | Y | N / weak | N / weak | N | hard |
| Screener | Y (`fmp-symbols`) | Y | Y | weak | P | N | N |
| Snapshot scores | Y (flag) | P / TBD | P / TBD | weak | weak | N | N |

**Only FMP has an in-repo full sync CLI today.** Other columns mean “you can
often buy/fetch this block and write the layout yourself,” not “we ship an
adapter.”

---

## How to get a dataset

| Path | What you do | Status |
|------|-------------|--------|
| **One-shot (official)** | `yuzu-cli fmp-sync …` → data root | Supported |
| **BYO** | Write the [data-layout](../reference/data-layout) tree yourself | Supported |
| **Assemble** | Fill blocks from different vendors into **one** data root | Supported as a pattern; **recipes below are WIP** |
| **Second full adapter** | Future `pomelo-XXX` if coverage gate passes | Not started |

Engine / `pomelo-data` only need the finished tree (local or S3 via
`pomelo-s3`). Optional quality pass: `yuzu-cli data-audit`.

---

## Assemble recipes (WIP)

These are **manual multi-step** sketches. Field mapping, rate limits, and
legal/ToS checks are **TBD** — do not treat as copy-paste production runbooks.

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

**TBD:** concrete column maps per vendor (#182 EODHD, further spikes).

### Recipe C — small free-tier demo universe

**Goal:** 50–200 US names for demos, not full-market sync.

1. Use a free tier (Finnhub / Tiingo / …) within published rate limits.
2. Sync only the blocks you need (usually prices ± thin fundies).
3. Expect incomplete delist / index PIT / snapshot coverage.

**TBD:** call-budget math (#183).

### Recipe D — FMP one-shot (reference)

```bash
# Illustrative — see pomelo-fmp / CLI help for current flags
yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
  --symbols AAPL,MSFT --from 20200101 --to 20241231
```

Optional flags cover industry, delisted, index membership, snapshot factors.
Starter-tier honesty: [`fmp-data-source.md`](../reference/fmp-data-source).

---

## Open work (track on GitHub)

| Item | Issue |
|------|--------|
| This doc + recipes | [#188](https://github.com/citrusquant/citrusquant/issues/188) (this page) |
| EODHD block coverage gate | [#182](https://github.com/citrusquant/citrusquant/issues/182) |
| Finnhub free partial blocks | [#183](https://github.com/citrusquant/citrusquant/issues/183) |
| Optional `pomelo-eodhd` (blocked) | [#185](https://github.com/citrusquant/citrusquant/issues/185) |
| Re-audit docs after paths mature | [#186](https://github.com/citrusquant/citrusquant/issues/186) |
| Parent research / stance | [#180](https://github.com/citrusquant/citrusquant/issues/180) |

When a cell moves from **TBD** → verified, update the matrix here and drop the
WIP banner only after the assemble story is good enough for newcomers.

---

## Related docs

- [`data-layout.md`](../reference/data-layout) — on-disk contract (source of truth for shapes)
- [`fmp-data-source.md`](../reference/fmp-data-source) — FMP Starter vs feature families
- [`backtest-engine.md`](../reference/backtest-engine) — panels / backtest semantics
- crate `pomelo-fmp` — only full vendor sync in-tree today

