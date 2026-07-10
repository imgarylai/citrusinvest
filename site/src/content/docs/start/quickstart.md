---
title: Quickstart
description: Install the engine, run the bundled example, and read your first backtest report.
---

You have two ways to run a backtest: **in your browser** (zero install) or
**natively in Rust**. Start with whichever fits.

## Option A — the browser playground (0 minutes)

Open the [interactive playground](../playground). It loads the WASM build of the
engine plus a small synthetic dataset and runs a real backtest locally — nothing
is sent to a server. Edit the strategy, press **Run**, and read the equity curve
and metrics. This is the fastest way to get a feel for lemon.

## Option B — native Rust (a few minutes)

You need a recent stable Rust toolchain.

```bash
git clone https://github.com/imgarylai/citrusinvest
cd citrusinvest
cargo run -p yuzu-core --example basic_backtest
```

That example is self-contained and readable top to bottom: it builds a tiny
price panel, authors a strategy in lemon, runs the backtest, and prints headline
metrics. It's the best 40 lines to read first.

### Use it as a library

Add the core crate:

```bash
cargo add yuzu-core lemon-lang
```

The top-level entry point is:

```rust
yuzu_core::run_backtest(spec_json, ctx, price_key, cfg) -> Result<Report, EngineError>
```

- `spec_json` — the JSON `Expr` tree, produced by `lemon::parse(source)`.
- `ctx` — an `EvalContext`: your numeric panels keyed by series name, plus an
  optional symbol → industry map.
- `price_key` — which series the backtest marks off (usually `"close"`).
- `cfg` — a `BacktestConfig` (fees, slippage, stops, benchmark, …).

See [Your first strategy](./first-strategy) to build the `spec`, and
[Bring your own data](../guides/bring-your-own-data) to build the `ctx`.

## What you get back

A `Report` — an equity curve (`dates` + `equity`), the trade list, and a large
[metrics block](../guides/reading-a-report). Everything the UI draws is computed
by the engine; the frontend only renders it.

## Next steps

- [Your first strategy](./first-strategy) — learn lemon by building one up.
- [Reading a report](../guides/reading-a-report) — decode every metric.
- [Lemon language reference](../reference/lemon) — the complete DSL.
