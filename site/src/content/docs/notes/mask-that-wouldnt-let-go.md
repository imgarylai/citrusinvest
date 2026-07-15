---
title: The mask that wouldn't let go
description: Building a point-in-time index universe surfaced a bug in the membership pattern our own docs taught — a masked position that the engine silently kept holding after a name left the index. Here's how a deterministic engine turned a subtle correctness bug into an exact number.
---

We shipped a feature and it caught a bug in our own documentation. This is the
good kind of embarrassing, so here's the whole thing.

## The setup: "hold the index, point-in-time"

A recurring request for [`lemon`](/reference/lemon) is a real **point-in-time**
index universe: run a strategy on *the S&P 500 as it actually was that day* —
not today's membership projected backwards, which quietly bakes in survivorship
bias. The building block was already there: a `panels/in_sp500` membership panel,
a `dates × symbols` grid of 1 (member that day) and 0 (not).

The pattern our docs taught to gate a strategy to members was a `mask`:

```text
mask(signal, in_sp500)
```

Read aloud that sounds exactly right — "keep the signal where the name is a
member, drop it otherwise." It is not right. It was quietly wrong in a way no
one had noticed, because you cannot see it by reading the code — only by running
it.

## The bug: forward-fill meets NaN

`mask` sets the non-member cells to **NaN**. And the backtest's NAV loop treats
NaN as *"no new instruction — hold last weight."* That is deliberate and
load-bearing: it's how weights drift naturally between rebalances instead of
being re-set to zero every bar.

Put those two facts together and the outer `mask` does the opposite of what it
reads like. The day a name **leaves** the index, its weight goes NaN — and the
NAV loop, seeing NaN, **keeps holding it**. The position you meant to drop rides
on, capturing whatever the name does *after* it left the index.

## The proof: an exact number

The engine is a pure function — `(strategy, panels) → Report`, no I/O, no hidden
state — so we could pin the bug to a number instead of arguing about it. Build a
four-day fixture where a name leaves the index after day 3 and then jumps +50%,
and compare the two spellings:

| spelling | equity curve | |
|---|---|---|
| `mask(normalize_row(is_largest(close, 2)), in_sp500)` — the documented pattern | `[1, 1, 1, 1.5]` | ❌ held after exit, captured the jump |
| `normalize_row(is_largest(close, 2)) * in_sp500` — the fix | `[1, 1, 1, 1.0]` | ✅ flattened on exit |

There it is: `1.5` vs `1.0` on the last day, from the same data, differing only
in `mask(…)` vs `… * in_sp500`. The masked version silently pocketed a return it
had no right to.

And the feature it was blocking, tested the same decisive way — a name that
leaves mid-window and rockets +100% afterward (`BBB`), and one that was never a
member and rockets +900% (`CCC`):

| run | last-day equity |
|---|---|
| `#! index: sp500` (point-in-time) | **1.05** — only the day's members held; `BBB` flat after exit, `CCC` excluded |
| no universe (the whole tree) | **4.37** — `CCC` and `BBB` dominate |

A 4× difference in headline return, entirely down to *which names were in the
universe on which day*. That is exactly the survivorship trap the feature exists
to close — and the buggy `mask` spelling would have leaked it back in.

## The fix

Multiply instead of mask. A membership panel is 1/0, so `signal * in_sp500` sets
a departing name to a hard **0** — flat — rather than NaN:

```text
normalize_row(is_largest(close, 2)) * in_sp500
```

`mask` isn't wrong everywhere — it's correct *inside* a selection op, where the
NaN is consumed before it reaches the NAV loop (e.g. `rank(mask(-pe, in_sp500))`
ranks only among that day's members). It's only the **outermost** `mask` against
a membership panel that's the trap.

We fixed the docs to teach `signal * in_sp500`, and the new
[`#! index: sp500`](/guides/playground-to-real-data) front-matter bakes the
correct spelling in for you: under the hood it wraps your strategy as
`signal * (in_sp500 >= 0.5)`, so a name goes flat the day it leaves the index —
you never have to think about the NaN.

## The takeaway

The bug was invisible to a code review and obvious to a four-row test. That's
the whole argument for an engine that's [a pure function](/#how-its-built):
determinism plus I/O-free means "is this right?" has a numeric answer you can
check in a unit test, not a vibe you argue about. A strategy is
[a file](/#a-strategy-is-a-file); a backtest is a value; and a subtle correctness
bug is just two numbers that should have been equal and weren't.

Want to feel it? Open the [playground](/playground), or `curl` the CLI and run a
[real point-in-time universe on your own data](/guides/playground-to-real-data).
