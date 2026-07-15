---
title: Playground — data & internals
description: Where the playground's sample data comes from, why it ends in 2017, and the exact pipeline each run goes through.
---

The [interactive playground](/playground) runs a **real backtest in your
browser** — the yuzu engine and the lemon parser are compiled to WebAssembly,
and every run is evaluated against real daily bars for 10 US large-caps
(Nov 2014 – Nov 2017). Nothing is sent to a server.

This page is the fine print behind that demo: exactly what the data is, why it
ends where it does, and the pipeline each run goes through.

## About the data

The sample dataset is **real market data**, chosen so it can live in a public
repo without a license fight:

- **Prices** — daily OHLCV for AAPL, MSFT, NVDA, AMZN, WMT, JPM, GS, XOM, JNJ
  and PFE, from the
  [Huge Stock Market Dataset](https://www.kaggle.com/datasets/borismarjanovic/price-volume-data-for-all-us-stocks-etfs)
  (**CC0 / public domain**), adjusted for splits and dividends.
- **P/E** — computed as adjusted close ÷ the last reported fiscal-year diluted
  EPS from [SEC EDGAR](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)
  XBRL filings (US government data, public domain). Each 10-K's EPS only becomes
  visible **on its filing date** — not the fiscal-period end — so there is no
  look-ahead. Try `is_smallest(pe, 3)` and note that AMZN's P/E goes *missing*
  during 2015: its FY2014 EPS was negative, so trailing P/E is undefined.
- **Benchmark** — every run is compared against a benchmark series (the engine's
  `benchmark_key` config). When the bundled data ships an index series it is
  used directly; otherwise the playground builds a **daily-rebalanced
  equal-weight basket of the same 10 names**. For a stock-picking strategy on a
  fixed universe that's the honest yardstick anyway: it answers "did picking
  *these* names beat just holding *all* of them?" — alpha, beta, excess return
  and the relative tabs all measure against it.

Why does the data end in 2017? Nearly every "free" market-data source (Yahoo,
Stooq, FMP, Tiingo, …) forbids redistribution, which rules them out for a
static site. This CC0 dataset is the newest daily OHLCV that is genuinely
public domain. The engine itself doesn't care — it ships **no** data and runs
on any panels you feed it: [Bring your own data](/guides/bring-your-own-data).
Everything is reproducible via
[`fetch-sample-data.mjs`](https://github.com/citrusquant/citrusquant/blob/main/site/scripts/fetch-sample-data.mjs).

One honest caveat: bundled prices are dividend-adjusted while EPS is
as-reported, so P/E early in the window is understated by a few percent for
high-dividend names. Good enough to demo mechanics; bring your own data for
research.

:::caution[Not investment advice]
The playground is a tool for reasoning about strategy mechanics on historical
data. Past performance does not predict future results, and nothing here is a
recommendation to buy or sell anything.
:::

## How it works

1. Your source is parsed by `lemon-wasm` into a JSON `Expr` tree (the *spec*).
2. The spec, the sample panels, a benchmark series and a config
   (`fee_ratio`, `benchmark_key`, `bootstrap_samples`) are handed to
   `yuzu-wasm.run_backtest(...)`.
3. The returned `Report` — equity, drawdown and rolling series, calendar
   returns, bootstrap confidence bands, the full trade list and ~40 metrics —
   is rendered into the tabs beside the editor. The frontend only draws; every
   number is computed by the engine (plus a few purely presentational reshapes).

That's the same pipeline the native engine runs — see
[Reading a report](/guides/reading-a-report) to decode the output.

## Things to try

Open the [playground](/playground) and paste any of these into the editor:

- `is_largest(sma(close, 2), 1)` — concentrate into a single name.
- `close > sma(close, 50)` — a pure trend filter across the whole universe.
- `is_smallest(pe, 3) and (close > sma(close, 20))` — cheap **and** trending.
- `is_largest(rsi(close, 14), 3)` — momentum by RSI. With NVDA's 2016–17 run in
  the universe, momentum strategies have an unfair advantage — watch what they hold.

New to the syntax? Start with [Your first strategy](/start/first-strategy), or
browse the [lemon reference](/reference/lemon) for the complete operator set.

## Shareable links

Every run has a link. Hit **🔗 Share** next to *Run* and the playground copies a
URL with your strategy encoded in it (`/playground#s=…`) — open it and you land
on the same strategy, already loaded. The strategy lives in the URL *hash*, so
it's never sent to a server; the link is self-contained.

Docs, issues, and posts can deep-link the same way — for example, open these in
the playground:

- [Momentum — hold the 3 strongest names](/playground#s=is_largest%28pct_change%28close%2C%2063%29%2C%203%29)
- [Trend + RSI filter](/playground#s=close%20%3E%20sma%28close%2C%2050%29%20and%20rsi%28close%2C%2014%29%20%3C%2070)
