---
title: How it compares
description: An honest look at citrusquant next to backtrader, vectorbt, and Lean — what each is genuinely good for, and where citrusquant deliberately stops.
---

Backtesting tools are not interchangeable — they make different trades. This
page is written the same way as [What it is — and isn't](/#what-it-is--and-isnt):
here's what each of these tools is genuinely good at, and where citrusquant
deliberately stops. If your problem lives in someone else's column, use their
tool — several are excellent.

citrusquant is the [`yuzu`](https://crates.io/crates/yuzu-core) engine plus the
[`lemon`](/reference/lemon) strategy language: a **daily-bar, portfolio-level**
backtester for **cross-sectional and trend** strategies, where a whole strategy
is [a single `.lemon` file](/#a-strategy-is-a-file) that runs the same native,
in CI, and in the browser.

## At a glance

| | citrusquant | backtrader | vectorbt | Lean (QuantConnect) |
|---|---|---|---|---|
| Language | Rust engine · `lemon` DSL | Python | Python (NumPy/Numba) | C# / .NET (Python API) |
| A strategy is… | a `.lemon` file | a `Strategy` subclass | array/notebook code | a project of classes |
| Bar granularity | daily bars | intraday → daily | intraday → daily | tick → daily |
| Cross-sectional / ranking | first-class (`rank`, `is_largest`, …) | manual | supported, array-shaped | supported |
| Parameter sweeps | built-in (native, Rayon) | manual loops | a core strength | supported |
| Live / paper trading | no (stops at the report) | yes (broker integrations) | no (research) | yes (a core strength) |
| Runs in the browser | yes (WASM) | no | no | no |
| Determinism | pure `(spec, panels) → Report` | stateful event loop | vectorized | stateful engine |
| License | MIT | GPL-3.0 | open-source core; paid PRO | Apache-2.0 core; hosted platform |

Every claim below is about **design shape**, not project quality — each of these
is a serious tool with real users.

## vs. backtrader

**Great for:** event-driven, per-bar decision logic on one or a few instruments,
and getting to *live/paper* trading through its broker integrations. A mature
indicator library and a large body of examples.

**Where citrusquant differs:** backtrader models a strategy as a `Strategy`
subclass with a `next()` callback that fires bar by bar — natural for
"when X, buy" event logic, less natural for "each day, rank the whole universe
and hold the top N," which is citrusquant's home turf. citrusquant is daily-bar
and portfolio-level, has no live-trading layer, and makes the strategy a file
rather than a class. Reach for backtrader when you want an event loop and a path
to a broker; reach for citrusquant when you want a readable cross-sectional
backtest you can diff and reproduce.

## vs. vectorbt

**Great for:** massive vectorized parameter sweeps and fast signal research over
big arrays, with tight NumPy/Numba/pandas integration and rich analytics — if
you're comfortable in a notebook.

**Where citrusquant differs:** vectorbt gives you enormous flexibility as array
code, at the cost of a notebook workflow whose result can depend on
cell-execution order and held state. citrusquant trades that flexibility for a
constrained, declarative file: a `.lemon` strategy is deterministic text — no
hidden state, same input → same `Report` — which is easier to review, diff, and
hand to a colleague, and it runs unchanged in the browser. citrusquant also does
parameter sweeps natively (parallelized with Rayon), though not at vectorbt's
scale-everything ambition. Choose vectorbt for maximal vectorized research;
choose citrusquant when reproducibility and a shareable artifact matter more than
raw flexibility.

## vs. Lean (QuantConnect)

**Great for:** production, multi-asset, live algorithmic trading — tick-to-daily
data, a large data library, a hosted platform, and a serious execution/brokerage
layer. If you're going to trade real money across asset classes, Lean is built
for it.

**Where citrusquant differs:** Lean is a full trading platform — a C# (or
Python-on-.NET) project with an engine, data feeds, and an execution layer.
citrusquant is deliberately *not* that: it's a small, embeddable engine that
stops at the report — no order routing, no brokerage, no tick simulation. That
smallness is the point: `cargo add yuzu-core`, or `curl … | sh`, and you're
running a backtest in minutes with no platform to adopt, and the same engine
embeds in a Rust service or a browser tab. Choose Lean to run a live,
multi-asset book; choose citrusquant to research daily cross-sectional ideas as
files, or to embed a pure backtest engine in your own stack.

## What citrusquant is *not*

Stated plainly, the same as on the [home page](/#what-it-is--and-isnt):

- **Not a data vendor** — you bring panels you're licensed to use.
- **Not tick or order-book simulation** — bars in, portfolio NAV out.
- **Not a broker or execution layer** — it stops at the `Report`.
- **Not investment advice** — it computes; you decide.

If those non-goals are dealbreakers, one of the tools above is the better fit —
and that's a fine outcome. If instead you want a readable, reproducible,
embeddable daily backtest, [start in the browser](/playground) and then
[run it on your own data](/guides/playground-to-real-data).
