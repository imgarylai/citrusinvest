---
title: Bring your own data
description: The engine ships no market data — how to feed your own prices and fundamentals.
---

The playground runs on a small bundled sample (10 US large-caps, 2014–2017). To
backtest **your** markets — more names, other assets, longer history — you supply
the data. The engine is pure and I/O-free: it only ever sees in-memory `Panel`
values keyed by series name. There are three ways to get your data in.

## What a panel is

A `Panel` is a dense matrix indexed by **dates × symbols**:

```jsonc
{
  "dates":   [20240102, 20240103, 20240104],  // YYYYMMDD ints, ascending
  "symbols": ["AAPL", "MSFT"],
  "data":    [[185.6, 371.2],                  // one row per date,
              [187.1, 373.0],                  // one column per symbol
              [186.4, 374.5]]                  // null = missing
}
```

An `EvalContext` is a map of series name → panel (`close`, `open`, `high`,
`low`, `volume`, `pe`, …), plus an optional symbol → industry map for the
sector operators.

## Option 1 — native files (`pomelo-data`)

Lay prices and fundamentals out on disk (gzip CSV, plain CSV, or Parquet) under a
data root and let `pomelo-data` load them into panels. The on-disk tree, series
names, and point-in-time notes are the canonical contract in
[Data layout](/reference/data-layout). This is what `yuzu-cli` and
`yuzu-server` read.

## Option 2 — build panels in code

If your data lives somewhere else, build `Panel` values yourself and assemble an
`EvalContext`:

```rust
let close = Panel::from_rows(dates, symbols, close_rows)?;
let mut panels = HashMap::new();
panels.insert("close".to_string(), close);
let ctx = EvalContext::new(panels);

let spec_json = serde_json::to_string(&lemon::parse("close > sma(close, 20)")?)?;
let report = yuzu_core::run_backtest(&spec_json, &ctx, "close", &BacktestConfig::default())?;
```

This is exactly what the `basic_backtest` example does end to end — see the
[Quickstart](/start/quickstart).

## Option 3 — the JSON / WASM boundary

`yuzu-wasm.run_backtest` takes a single JSON request — the same shape the
playground builds:

```jsonc
{
  "spec":      { /* Expr tree from lemon.parse(...) */ },
  "price_key": "close",
  "panels":    { "close": { "dates": [...], "symbols": [...], "data": [[...]] },
                 "pe":    { ... } },
  "industry":  { "AAPL": "Tech", ... },
  "config":    { "fee_ratio": 0.001 }
}
```

Fetch your prices, transform them into these panels, and call `run_backtest`.
That's how you'd wire a "bring your own FMP key" mode into a web app.

## A note on data sources

If you load from an **FMP Starter**-class key, some features need panels that
tier doesn't provide. Which ops and backtests are honestly runnable (and which
are blocked by missing series) is documented in the
[FMP data source](/reference/fmp-data-source) reference — feature/series gaps,
not a plan-comparison table.

## Licensing reminder

Most market-data vendors forbid redistribution, so don't commit vendor prices
into a public repo. The bundled sample is public-domain (CC0) data precisely to
stay shareable; keep licensed data on the user's side (native files or a
bring-your-own-key flow).
