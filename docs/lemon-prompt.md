# Lemon prompt card

A compact, prompt-ready reference for generating **lemon** strategies. Drop it into a system
prompt. Full reference: `docs/lemon.md`.

## What lemon is

A strategy is **one expression** (optionally preceded by `let name = expr` bindings). It
evaluates, per trading day and across symbols, to a matrix the backtester consumes — usually a
boolean "hold this symbol" panel or a selection/rotation.

## Syntax

- **Numbers**: `10`, `3.5`, `1_000_000`, `5e8`. **Strings**: `"ME"` (double quotes, no escapes).
  **Comments**: `# ...` to end of line.
- **Data series** = bare identifiers: `close`, `open`, `high`, `low`, `volume`, `pe`, `roe`,
  `market_cap`, `revenue_growth`, … (an unknown identifier is silently treated as a data series).
- **Operators**, lowest→highest precedence, all left-associative:
  `or` · `and` · `not` (prefix) · `>` `<` `>=` `<=` · `+` `-` · `*` `/` · unary `-`.
  Comparisons yield `1`/`0`. `not a > b` is `not (a > b)`.
  **There is NO `==`, `!=`, `&`, `|`, `!`** — use `and`/`or`/`not` and `>=`/`<=`.
- **Calls**: `fn(pos1, pos2, key=value)` — positional args first, then keyword args.
  List literals `[a, b]` appear **only** as call arguments.
- **`let`**: `let x = sma(close, 50)` then reuse `x`; inlined at parse time; no re-binding.

## Op reference

`?` = optional; defaults noted. `of`/`entry`/`exit`/`by` are expressions; `n`/`d` are numbers.

**Moving averages & rolling**: `sma(of, n)` (alias `average`) · `ema(of, n)` · `std(of, n)` ·
`rsi(of, n)` · `pct_change(of, n)` · `rise(of, n)` / `fall(of, n)` (1 if rose/fell n days in a row) ·
`shift(of, n)` (lag) · `rolling_max(of, n)`.

**OHLCV indicators**: `atr(high,low,close,n)` · `natr(...)` · `willr(...)` · `cci(...)` ·
`stoch_k(high,low,close,n)` · `stoch_d(high,low,close,n, d?=3)` · `aroon_up(high,n)` ·
`aroon_down(low,n)` · `adx(high,low,close,n)` · `plus_di(...)` · `minus_di(...)` ·
`obv(close,volume)` · `mfi(high,low,close,volume,n)` · `vwap(high,low,close,volume,n)`.

**Cross-section & selection** (per row, across symbols): `is_largest(of, n)` / `is_smallest(of, n)`
(top/bottom n) · `rank(of, pct?=true, ascending?=true)` (percentile rank) ·
`quantile_row(of, c)` (per-row quantile, e.g. `c=0.5` median) ·
`winsorize(of, lower, upper)` · `zscore(of)` · `bucket(of, n)` · `demean(of)` ·
`mask(of, by)` (keep `of` where `by` is true) · `normalize_row(of)` (scale each row to unit
gross weight — explicit portfolio weights, e.g. inverse-vol: `normalize_row(sig / std(close, 20))`) ·
`vol_target(of, prices, target?=0.1, n?=63)` (scale weights toward annualized portfolio-vol target; deleverage only).

**Streaks, edges & rotation**: `sustain(of, nwindow, nsatisfy?)` · `is_entry(of)` / `is_exit(of)`
(rising/falling edge) · `exit_when(entry, exit)` · `hold_until(entry, exit, nstocks_limit?, rank?)`
(stateful selection; `rank` is an expression. **Price stops live in the backtest config, not the op.**) ·
`rebalance(of, freq?, on?)` (`freq` = `"W"`/`"ME"`/`"QE"`/`"YE"`).

**Neutralization / industry**: `neutralize(of, by=[...], add_const?=true)` (`by` is a list) ·
`neutralize_industry(of, add_const?=true)` · `industry_rank(of, categories?=["..."])` ·
`cap_industry(of, max_weight?=0.3)` (cap each industry's gross weight, scale down; residual → cash) ·
`groupby_category(of, agg)` (`agg` e.g. `"mean"`) · `in_sector(of, name)` (exact sector string
match → 0/1 mask; pair with `mask`).

**Scalar**: `ceil(of)`.

## Examples

```lemon
# 1. Hold names trading above their 200-day average.
close > sma(close, 200)

# 2. Oversold, but only large caps.
mask(rsi(close, 14) < 30, market_cap > 1000000000)

# 3. Top 20 by a cheap-quality blend, rebalanced monthly.
rebalance(is_largest(rank(-pe) + rank(roe), 20), freq = "ME")

# 4. Momentum rotation, max 20 names. (A stop-loss is set on the backtest
#    config — e.g. `--stop-loss 0.08` — not in the strategy.)
hold_until(
  entry = close > sma(close, 200),
  exit  = close < sma(close, 50),
  nstocks_limit = 20,
)
```

## Validating output

Parse it: `lemon::parse(src)` returns `ParseError { line, col, message }` on failure — feed the
message back and regenerate. A clean parse means the syntax is valid (series names are checked
later by the engine).
