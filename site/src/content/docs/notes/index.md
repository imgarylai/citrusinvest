---
title: Engineering notes
description: Short write-ups from building the yuzu engine and the lemon language — bugs we found, decisions we made, and what a deterministic backtest engine makes easy.
---

Short write-ups from building the engine — bugs we found (including in our own
docs), decisions we made, and what falls out of a backtest that's a pure,
deterministic function.

## Posts

- [The mask that wouldn't let go](/notes/mask-that-wouldnt-let-go) — building a
  point-in-time index universe surfaced a membership-masking bug our own docs
  taught, and the engine pinned it to an exact number.
