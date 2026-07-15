---
title: From playground to real data
description: A linear walkthrough — take a strategy from the browser playground and run it on your own universe and dates with the lemon CLI, including a point-in-time S&P 500 universe.
---

The [playground](/playground) runs the real engine, but on a fixed toy set:
10 US large-caps, 2014–2017. This guide takes the **same** strategy and runs it
on **your** universe and dates, on your machine — same engine, no browser
ceiling. Budget about three minutes.

The tempo: **30 seconds in the browser, 3 minutes on your machine.** You already
did the browser part. Here's the machine part.

## 1. Install the CLI

One command installs the `lemon` binary (macOS / Linux; on Windows grab the
[release asset](https://github.com/citrusquant/citrusquant/releases)):

```bash
curl -fsSL https://citrusquant.com/install.sh | sh
```

It installs to `~/.local/bin` (override with `$LEMON_INSTALL_DIR`), verifies the
download against its checksum, and prints `lemon --version`. That's the whole
install — one static binary, no runtime.

## 2. Bring a data key

`lemon` ships **no** market data and never phones home. To pull daily bars you
bring your own key from a vendor. The built-in `--sync` supports
[Financial Modeling Prep](https://site.financialmodelingprep.com/pricing-plans)
(FMP), which has a free tier:

```bash
export FMP_API_KEY=…    # your key
```

Nothing is fetched unless you pass `--sync` **and** this key is set — a strategy
file can never trigger a network request on its own.

## 3. Write the strategy as a file

A strategy is a single `.lemon` file: the expression you tuned in the playground,
plus front-matter that states how to run it. Save this as `momentum.lemon`:

```text
#! universe: 20180101..20241231
#! symbols: AAPL, MSFT, NVDA, AMZN, GOOGL, META, JPM, XOM
#! config: { "fee_ratio": 0.001 }
#! data-source: fmp
is_largest(pct_change(close, 63), 3)
```

Each `#!` line is front-matter — comments to the language itself, so the
expression stays pure:

- `#! universe:` — the backtest window, `FROM..TO` (either side optional).
- `#! symbols:` — the names to run on. Start with an explicit list; §5 swaps in a
  real index.
- `#! config:` — the [engine knobs](/reference/backtest-engine) (fees, slippage,
  stops, benchmark, …).
- `#! data-source:` — which vendor `--sync` may fetch missing data from.

Sanity-check it without running anything:

```bash
lemon check momentum.lemon
```

## 4. Sync and run

`--sync` fetches the declared names' daily bars for the window — only the ones
you don't already have — then runs:

```bash
lemon momentum.lemon --sync
```

`lemon` prints the full `Report` as JSON: the headline metrics, the equity curve
(`dates` + `equity`), and every trade — the same structure the playground draws,
computed by the same engine. Write it to a file with `--out report.json`, and see
[Reading a report](/guides/reading-a-report) to decode every metric.

Already have a local data tree? Drop `--sync` and point `lemon` at it —
`lemon momentum.lemon --data ~/qdata` (or set `$CITRUS_DATA`) runs fully offline.

## 5. Level up: a real, point-in-time index universe

A hand-typed `#! symbols:` list has a subtle trap. Today's S&P 500 names weren't
all in the index back in 2018 — backtesting *today's* list over a 2018 window
quietly bakes in **survivorship bias** (you're only testing the names that
survived to today). For a real index study you want **point-in-time** membership:
who was in the index *that day*.

`lemon` spells that as `#! index:`:

```text
#! universe: 20180101..20241231
#! index: sp500
#! config: { "fee_ratio": 0.001 }
is_largest(pct_change(close, 63), 3)
```

`#! index: sp500` scopes the run to the window's ever-members and holds only each
day's actual members, **flattening a name the day it leaves the index** (the
engine wraps your strategy as `signal * (in_sp500 >= 0.5)` — a multiply, not a
`mask`, so a departing name goes flat instead of being silently held; see
[Data layout §8](/reference/data-layout)). Supported indices: `sp500`,
`nasdaq`, `dowjones`.

This one needs a bit more setup, and it's the one place the guide steps outside
`lemon`. Point-in-time membership lives in a `panels/in_sp500` panel that must be
built first — **`lemon run --sync` fetches prices only, not the membership
panel**, so it will stop with an actionable error if the panel is missing. Build
it once with the `yuzu-cli` companion, which reconstructs the index's historical
membership and syncs every ever-member's prices in one pass. It isn't published
to crates.io yet, so run it from a checkout:

```bash
git clone https://github.com/citrusquant/citrusquant
cd citrusquant
cargo run -p yuzu-cli -- fmp-sync --api-key "$FMP_API_KEY" \
  --out ~/qdata --index sp500 --from 20180101 --to 20241231
```

That writes a complete data tree to `~/qdata` — the members' price files **and**
`panels/in_sp500.csv.gz`. Now run the index strategy against it:

```bash
lemon momentum.lemon --data ~/qdata
```

No `--sync` needed — the tree already has everything. See the
[FMP data source](/reference/fmp-data-source) reference for the full `fmp-sync`
flag set (fundamentals, other indices, S3/R2 output).

## Where to go next

- [Reading a report](/guides/reading-a-report) — what every metric means.
- [Bring your own data](/guides/bring-your-own-data) — assemble a data tree by hand or from another vendor.
- [lemon language reference](/reference/lemon) — the complete DSL and front-matter.
- [Your first strategy](/start/first-strategy) — build a lemon expression up one operator at a time.
