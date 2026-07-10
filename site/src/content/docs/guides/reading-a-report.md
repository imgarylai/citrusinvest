---
title: Reading a report
description: What every series and metric in a yuzu Report means.
---

`run_backtest` returns a `Report`. The engine computes everything; a UI only
draws it. This page decodes what you see.

## The series

| Field | What it is |
|-------|-----------|
| `dates` | Trading dates as `YYYYMMDD` integers (e.g. `20240102`). |
| `equity` | The NAV curve, **rebased to 1.0** at the first date. `1.35` means +35%. |
| `benchmark` | Benchmark equity rebased to 1.0 and aligned to `dates` — only when a `benchmark_key` was configured. `NaN` before the benchmark's first point. |
| `rolling_sharpe` / `rolling_vol` | 252-day rolling annualized figures, aligned to `dates`. |

## Headline metrics

Everything derivable from the equity curve. Returns are fractions (`0.20` = 20%).

| Metric | Meaning |
|--------|---------|
| `total_return` | End-to-end return over the whole sample. |
| `cagr` | Compound annual growth rate. |
| `ann_volatility` | Annualized standard deviation of daily returns. |
| `sharpe` / `sortino` | Risk-adjusted return; Sortino penalizes only downside. |
| `max_drawdown` | Worst peak-to-trough decline (a negative-ish magnitude). |
| `calmar` | CAGR ÷ max drawdown. |
| `max_drawdown_duration` | Longest time underwater, in trading days. |
| `ulcer_index` / `avg_drawdown` | Depth-and-duration shape of drawdowns. |

## Trade-level metrics

Derived from the realized trade list.

| Metric | Meaning |
|--------|---------|
| `num_trades` | Number of closed trades. |
| `win_rate` | Fraction of trades that were profitable. |
| `profit_factor` | Gross profit ÷ gross loss. |
| `expectancy` | Average P&L per trade. |
| `avg_win` / `avg_loss` / `payoff_ratio` | Average winner, loser, and their ratio. |
| `best_trade` / `worst_trade` | Extremes. |
| `avg_holding_period` | Mean bars held per trade. |
| `max_consecutive_losses` | Longest losing streak. |
| `recovery_factor` | Total return ÷ max drawdown. |

## Exposure & distribution

| Metric | Meaning |
|--------|---------|
| `time_in_market` / `avg_exposure` | How often / how much you were invested. |
| `best_day` / `worst_day` | Best and worst single-day returns. |
| `skew` / `kurtosis` | Shape of the daily-return distribution. |
| `var_95` / `cvar_95` | Value-at-risk and conditional VaR at 95%. |

## Conditional blocks

Some fields appear only when applicable:

- `ytd`, `one_year`, `three_year` — lookback returns, present when the sample is
  long enough to cover the window.
- `benchmark_return`, `alpha`, `beta`, `excess_return`, `tracking_error`,
  `information_ratio` — present only when a benchmark was supplied.
- A **live segment** block — metrics on the post-go-live slice of the curve,
  present only when `live_performance_start` was set.

For the exact NAV model, metric conventions, and the full JSON contract, see the
[backtest engine reference](../reference/backtest-engine).
