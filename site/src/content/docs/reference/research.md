---
title: "Research (factor / event)"
editUrl: false
sourceFile: docs/research.md
---

<!-- Imported from docs/research.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
Factor diagnostics and event studies are **research workflows**, not portfolio
construction. They take panels and return JSON summaries; they are deliberately
**outside the lemon AST** — no positions, no NAV, no change to backtest
semantics. They live in `yuzu_core::research` as pure functions, with thin
`yuzu-cli` entry points.

## Factor report

Rank **IC** (information coefficient), **ICIR**, **quantile-portfolio** returns,
and the **long-short spread** of a factor against forward returns.

```rust
use yuzu_core::research::{factor_report, forward_returns};
let fwd = forward_returns(&close, 21);           // 21-day forward simple return
let rep = factor_report(&factor, &fwd, 5);       // 5 quantile buckets
// rep.mean_ic, rep.icir, rep.quantile_returns (low→high), rep.long_short,
// rep.top_quantile_turnover, rep.ic (per-period series), rep.dates
```

- **IC** is the per-date **Spearman rank correlation** between the factor and
  forward returns, over the symbols where both are finite (≥2 needed). `mean_ic`
  / `ic_std` / `icir = mean_ic / ic_std` summarize it (ICIR is **not**
  annualized — multiply by √periods-per-year if you want that).
- **Quantiles** bucket symbols by factor rank each date (bucket 0 = lowest);
  `quantile_returns` is each bucket's equal-weighted mean forward return averaged
  over periods, and `long_short` is top − bottom.
- **Industry-neutral** factors: neutralize first —
  `factor.neutralize_industry(&industry, true)` — then pass the residual in.

CLI:

```bash
yuzu-cli factor --data ./mydata --spec factor.json \
  --from 20180101 --to 20251231 --horizon 21 --quantiles 5 [--neutralize-industry]
```

`spec` is any lemon/JSON `Expr` evaluated to the factor panel; forward returns
come from the close panel over `--horizon` days.

## Event study

Average (and cumulative) return path around a **0/1 event panel** over a
`[-pre, +post]` window.

```rust
use yuzu_core::research::{event_study, daily_returns};
let rets = daily_returns(&close);                // backward daily returns
let es = event_study(&events, &rets, 5, 5);      // 5 rows pre / post
// es.lags (−pre..=post), es.avg_return, es.cumulative, es.event_count
```

For each cell where `events == 1`, the same symbol's returns from `pre` rows
before to `post` rows after are averaged across all events by lag. Returns are
**raw** — for *abnormal* returns, subtract a benchmark return panel from `rets`
before the call (a market model is out of scope for v1). Events near a panel edge
simply contribute to fewer lags.

CLI:

```bash
yuzu-cli event --data ./mydata --spec event.json \
  --from 20180101 --to 20251231 --pre 5 --post 5
```

`report_event` (the fundamentals 0/1 filing-day series in
[`data-layout.md`](../reference/data-layout)) is a natural event input.

## In the browser (WASM)

Both workflows are exposed at the `yuzu-wasm` JSON boundary, mirroring
`run_backtest`: string in, string out, no market data crosses except the panels
you supply.

```js
run_factor(JSON.stringify({
  spec, panels, industry,            // panels: { name: { dates, symbols, data } }
  horizon: 21, quantiles: 5,         // defaults: horizon 1, quantiles 5
  neutralize_industry: false,        // demean the factor within sector first
}));                                 // -> FactorReport JSON

run_event(JSON.stringify({
  spec, panels, industry,
  pre: 5, post: 5,                   // defaults: 5 / 5
}));                                 // -> EventStudy JSON
```

The factor / event panel comes from `spec` (any lemon/JSON `Expr`); forward
returns (`run_factor`) and daily returns (`run_event`) both come from the `close`
panel — a missing `close` is an error. The pure `run_factor_json` /
`run_event_json` functions are unit-tested natively.

## Non-goals (v1)

Model training / ML pipelines, charts, significance testing, and multi-factor
attribution. The outputs are deterministic plain numbers a UI or notebook can
plot; the functions are golden-simple to keep them trustworthy.

