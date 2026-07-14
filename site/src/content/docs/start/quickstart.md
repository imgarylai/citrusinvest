---
title: Quickstart
description: Run a real backtest three ways — in your browser, with the lemon CLI on your own data, or embedded as a Rust/Python/WASM library.
---

Three ways to run a backtest, fastest first: **in your browser** (zero install),
with the **`lemon` CLI** on your own machine, or **embedded** as a library in
Rust, Python, or JS/WASM. Start with whichever fits.

## Option A — the browser playground (0 minutes)

Open the [interactive playground](/playground). It loads the WASM build of the
engine plus three years of real daily bars for 10 US large-caps and runs a real
backtest locally — nothing is sent to a server. Edit the strategy, press
**Run**, and read the equity curve and metrics. This is the fastest way to get
a feel for lemon.

## Option B — install the CLI and `lemon run` (a few minutes)

The native path is a one-line install and a `.lemon` file you run on your own
data — no repo clone. One command installs the `lemon` binary (macOS / Linux;
on Windows, grab the [release asset](https://github.com/citrusquant/citrusquant/releases)):

```bash
curl -fsSL https://citrusquant.com/install.sh | sh
```

It installs to `~/.local/bin` (override with `$LEMON_INSTALL_DIR`), verifies the
download against its checksum, and prints the installed `lemon --version`.

### A strategy is a single file

Save this as `momentum.lemon`:

```text
#! universe: 20180101..20241231
#! symbols: AAPL, MSFT, NVDA, AMZN, GOOGL, META, JPM, XOM
#! config: { "fee_ratio": 0.001 }
#! data-source: fmp
is_largest(pct_change(close, 63), 3)
```

The `#!` lines are **front-matter**: the backtest window, the universe, the
fee, and which vendor *may* fill data gaps. They're plain comments to the
language itself — a `.lemon` file never makes a network request on its own.
(`lemon check momentum.lemon` validates the front-matter and syntax without
running anything.)

### Run it

`--sync` fetches the declared names' daily bars from the vendor the file names.
Bring your own `$FMP_API_KEY`; nothing is fetched without it, and only the
missing names are downloaded:

```bash
export FMP_API_KEY=…            # your key; the file declares the vendor
lemon momentum.lemon --sync
```

`lemon` prints the full `Report` as JSON — the headline metrics, the equity
curve (`dates` + `equity`), and the trade list — the same structure the
playground draws, computed by the same engine. Write it to a file with
`--out report.json`.

Already have a local data tree? Drop `--sync` and point `lemon` at it:
`lemon momentum.lemon --data ~/qdata` (or set `$CITRUS_DATA`) runs fully
offline. The complete front-matter keys and run flags — including the named
point-in-time `#! index: sp500` universe — are in the
[lemon reference](/reference/lemon); to sync your own data see
[Bring your own data](/guides/bring-your-own-data).

## Option C — use it as a library

The same engine embeds anywhere. In **Rust**, add the core crate:

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

To read the shortest end-to-end example first:

```bash
git clone https://github.com/citrusquant/citrusquant
cargo run -p yuzu-core --example basic_backtest
```

The same core ships for **Python** (`pip install yuzu-backtest` — abi3 wheels,
Python ≥ 3.9) and **browsers / Cloudflare Workers**
(`npm install @citrusquant/yuzu-wasm @citrusquant/lemon-wasm`). See the landing
page's [Get it](/#get-it) tabs and the [API reference](/reference/api) for each.

## What you get back

A `Report` — an equity curve (`dates` + `equity`), the trade list, and a large
[metrics block](/guides/reading-a-report). Everything the UI draws is computed
by the engine; the frontend only renders it.

## Next steps

- [Your first strategy](/start/first-strategy) — learn lemon by building one up.
- [Reading a report](/guides/reading-a-report) — decode every metric.
- [Bring your own data](/guides/bring-your-own-data) — assemble a data tree for `lemon run`.
- [lemon language reference](/reference/lemon) — the complete DSL and front-matter.
