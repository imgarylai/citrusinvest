---
title: Your first strategy
description: Learn the lemon DSL by building a strategy one operator at a time — then run it in the playground.
---

A lemon strategy is one expression that evaluates, every day, to a **position
matrix**: which symbols you hold. The engine turns that into an equity curve.
Let's build one up. Every snippet below runs as-is in the
[playground](/playground) against the bundled sample data.

## 1. A series

The bare name of a panel is a series. `close` is the daily close for every
symbol. On its own it isn't a strategy — it's the raw material.

```text
close
```

## 2. A signal (a boolean)

Comparisons produce a **boolean matrix** — `true` where the condition holds. A
classic trend filter: hold a name while its close is above its 20-day simple
moving average.

```text
close > sma(close, 20)
```

`sma(of, n)` is one of many rolling operators — `ema`, `std`, `rsi`,
`rolling_max`, `pct_change`, … all take `(of, n)`. See the
[full op reference](/reference/lemon).

## 3. Selecting a few names

Holding *everything* above its average is a lot of positions. `is_largest(of,
n)` keeps only the `n` names with the largest value each day:

```text
is_largest(sma(close, 2), 3)
```

Read it as *"hold the 3 names with the highest 2-day average close."* This is
the default strategy in the playground — run it, then change `3` to `1` and
watch the equity curve concentrate.

## 4. Combining conditions

Use `and` / `or` / `not` to compose signals. Momentum **and** a trend filter:

```text
is_largest(pct_change(close, 20), 3) and (close > sma(close, 50))
```

*"Of the names with the strongest 20-day return, hold the top 3 that are also
above their 50-day average."*

## 5. Using fundamentals

The sample dataset also carries a `pe` panel — real trailing P/E built from SEC
EDGAR filings (see [about the data](/playground-about#about-the-data)). Prefer
cheaper names by ranking on it:

```text
is_smallest(pe, 3) and (close > sma(close, 20))
```

*"Hold the 3 lowest-P/E names that are also trending up."* Note that P/E can be
**missing**: AMZN's last-reported (FY2014) EPS was negative, so its trailing P/E
is undefined during 2015 — and `is_smallest` simply never selects it. Missing
data is a first-class case in the engine, not an error.

## 6. Run it

Paste any of the above into the [playground](/playground) and press **Run**
(or `Ctrl`/`Cmd`+`Enter`). You'll get an equity curve plus Sharpe, max drawdown,
win rate, and more. To understand each number, read
[Reading a report](/guides/reading-a-report).

## Where to go next

- [Lemon language reference](/reference/lemon) — every operator, precedence,
  `let` bindings, and gotchas.
- [Bring your own data](/guides/bring-your-own-data) — swap the synthetic
  sample for real prices and fundamentals.
- [Strategy envelope](/reference/strategy-envelope) — package a strategy as a
  shareable, versioned, validated document.
