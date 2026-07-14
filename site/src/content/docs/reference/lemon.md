---
title: "lemon language"
editUrl: false
sourceFile: docs/lemon.md
---

<!-- Imported from docs/lemon.md by site/scripts/import-reference-docs.mjs — edit the source, then re-run `npm run import:docs`. -->
lemon is the small text language you write trading strategies in. A strategy
such as

```text
close > sma(close, 2)
```

is **lowered** (compiled) into a JSON `Expr` tree — the serializable strategy
AST — which the **yuzu** backtest engine walks against price/fundamental data to
produce a position matrix. The same JSON runs identically in the browser/Worker
(WASM) and in the native batch runner, so what you write here is exactly what
gets backtested.

lemon does no math itself. It is a **surface syntax over the `Expr` AST**: the
parser turns text into JSON, and the engine (documented in
[`backtest-engine.md`](../reference/backtest-engine)) supplies all the semantics.

The crate is `lemon-lang`, imported as `lemon`. Its public API is tiny:

```rust,no_run
let tree: serde_json::Value = lemon::parse("close > sma(close, 2)").unwrap();
let text: String = lemon::format(&tree); // JSON Expr → canonical DSL text
```

- `lemon::parse(src: &str) -> Result<serde_json::Value, lemon::ParseError>` —
  DSL text to the JSON `Expr` tree.
- `lemon::format(&serde_json::Value) -> String` — a tree back to canonical,
  re-indented DSL text (a "gofmt for lemon"). Note: `format` cannot reconstruct
  `let` bindings — see [`let` is parse-time inlining](#let-bindings).

A `ParseError` carries a **1-based** `line` and `col` and a message, and prints
as `line:col: message`.

---

## 1. Lexical elements

### Numbers

- Plain integers and decimals: `2`, `0.5`, `500000000`.
- **Underscore digit separators** anywhere in the digits: `1_000_000` is
  `1000000`. Underscores are simply stripped.
- **Scientific notation** with `e`/`E` and an optional sign: `5e8`, `5E8`,
  `1.5e-3`, `2e+6`. (The exponent must be followed by a digit, or a sign then a
  digit, or the `e`/`E` is not treated as part of the number.)

Integral values (no fractional part) are emitted as JSON integers so they
deserialize into the engine's `usize` window fields; non-integral values stay
floats.

### Strings

Double-quoted: `"ME"`, `"tech"`. Strings are used only for a few enum-like
fields (`freq`, `agg`, `industry_rank` categories).

**There are no escape sequences.** A backslash is a literal backslash, and there
is no way to embed a `"` inside a string — the first closing quote ends it. An
unterminated string is a parse error.

### Comments

`#` starts a line comment that runs to end-of-line:

```text
close > sma(close, 20)  # golden-cross-ish entry
```

### Identifiers

ASCII only: `[A-Za-z_][A-Za-z0-9_]*` — a letter or underscore, then letters,
digits, or underscores. Examples: `close`, `market_cap`, `revenue_growth`,
`_tmp`. There is no Unicode in identifiers.

### Booleans

`true` and `false` are **lexed as ordinary identifiers** and only recognized as
boolean literals by the parser. They are valid only where a boolean is expected
(the `true`/`false` keyword arguments like `ascending`, `pct`, `add_const`); a
boolean cannot stand alone as an expression.

### Keywords and punctuation

The only reserved word is `let`. The logical operators `and` / `or` / `not`
are also words but are lexed as identifiers and recognized positionally. Punctuation:
`(` `)` `[` `]` `,` `=` and the operator characters below.

Any other character (`$`, `@`, `!`, `&`, `|`, `%`, `;`, `{`, `}`, `.` outside a
number, etc.) is a lexer error: `unexpected character`.

---

## 2. Operators

All operators are **infix and left-associative**, except unary minus which is
prefix. Comparisons and logical ops produce `1`/`0` panels (see the engine's
boolean convention). Precedence, **lowest to highest**:

| Precedence | Operators        | Meaning                         | Assoc. |
| ---------- | ---------------- | ------------------------------- | ------ |
| 1 (lowest) | `or`             | logical OR                      | left   |
| 2          | `and`            | logical AND                     | left   |
| 2.5        | `not`            | logical NOT (prefix)            | —      |
| 3          | `>` `<` `>=` `<=`| comparisons → `1`/`0`           | left   |
| 4          | `+` `-`          | add / subtract                  | left   |
| 5          | `*` `/`          | multiply / divide               | left   |
| 6 (highest)| unary `-`        | negation (prefix)               | —      |

So `a and b or c` parses as `(a and b) or c`, and `2 * x + y` parses as
`(2 * x) + y`. `not` binds looser than comparisons and tighter than `and`:
`not a > b` is `not (a > b)`, and `not a and b` is `(not a) and b`. NaN is
falsy, so `not` of NaN is `1` (matching `and`/`or`).

### Operators that do NOT exist

There is **no `==`, no `!=`, no `&`, no `|`, and no `!`.** Logical AND / OR /
NOT are the **words** `and` / `or` / `not`. Equality is deliberately absent —
you compare with `>` / `<` / `>=` / `<=`. Typing `==`, `&`, `|`, or `!` is a
parse or lex error, so a strategy that "looks right" from another language will
fail loudly rather than silently misbehave.

---

## 3. Grammar and expression forms

### Program shape

A program is:

1. Zero or more `let NAME = EXPR` bindings, followed by
2. **exactly one** top-level expression.

That final expression is your strategy. It must be an **op node** — a function
call or an operator expression — not a bare number or string. `42` on its own is
rejected (`a strategy must be an expression, not a bare constant`); wrap it in
something that produces a signal, e.g. `close > 42`.

A rough grammar sketch:

```text
program     := let_binding* expr EOF
let_binding := "let" IDENT "=" expr
expr        := or_expr
or_expr     := and_expr   ("or"  and_expr)*
and_expr    := cmp_expr   ("and" cmp_expr)*
cmp_expr    := add_expr   ( ("<"|">"|"<="|">=") add_expr )*
add_expr    := mul_expr   ( ("+"|"-") mul_expr )*
mul_expr    := unary      ( ("*"|"/") unary )*
unary       := "-" unary | primary
primary     := NUMBER | STRING | "true" | "false"
             | "(" expr ")"
             | IDENT "(" args ")"          # function call
             | IDENT                        # let-bound name OR a Data series
args        := (arg ("," arg)*)?
arg         := IDENT "=" arg_value          # keyword arg
             | arg_value                     # positional arg
arg_value   := "[" (expr ("," expr)*)? "]"  # list literal (call args only)
             | expr
```

### <a id="let-bindings"></a>`let` is parse-time inlining

`let` does **not** create a runtime variable. At parse time the parser
**substitutes the bound subtree at every use site**. Given

```text
let ma = sma(close, 20)
hold_until(entry = close > ma, exit = close < ma, nstocks_limit = 5)
```

the resulting tree contains **two independent copies** of `sma(close, 20)` — one
in `entry`, one in `exit`. `let` is purely for readability and de-duplication in
the source; it changes nothing about what the engine sees.

Consequences:

- **Re-binding a name is an error.** `let a = close` then `let a = pe` fails with
  `` `a` is already defined ``.
- **`format` cannot reconstruct `let`s.** Because bindings are inlined before the
  tree exists, `lemon::format` re-emits the fully expanded form. Round-tripping
  source through the formatter drops your `let`s and inlines them.

### Function calls: positional first, then keyword

Arguments are given **positionally first, then by keyword** (`name = value`),
exactly like Python:

```text
sma(close, 20)                       # both positional
rank(close, ascending=false)         # positional `of`, keyword `ascending`
rebalance(x, freq="ME")              # positional `of`, keyword `freq`
```

A **positional argument after a keyword argument is an error**
(`positional argument after keyword argument`). Each op has a fixed field order
(see the reference below); positional args fill fields left to right, and any
remaining fields can be supplied by name. Unknown keyword names and too many
positional args are errors.

### List literals — call arguments only

`[ ... ]` list syntax exists **only inside a call's argument list**, for the ops
that take a list field (`neutralize(..., by=[pe, market_cap])`,
`industry_rank(..., categories=["tech", "fin"])`). There is no general list
value and no list operator elsewhere in the language.

### No subscript / indexing

There is **no `[]` indexing** and no `.` member access. `x[0]` and `x.field` are
not valid syntax.

### Unknown identifiers become `Data` series — silently

A bare identifier that is neither a `let` name nor immediately followed by `(`
becomes a **`Data` series reference** (`{"op":"Data","name":"..."}`). This is how
you reference inputs: `close`, `pe`, `market_cap`, `revenue_growth`.

**There is no parse-time check that the series exists.** A typo like `clsoe`
parses happily as `Data("clsoe")` and only fails later, at engine evaluation
time, when no such series is found. The set of valid series names is the
**engine's**, not lemon's — lemon will accept any identifier. Proof-read your
series names.

---

## 4. Built-in op reference

Every function-style op below is a **call**: `name(args...)`. Arguments are
listed in **positional order**; `?` marks an optional argument with its default.
`sma`/`average` are two names for the same op — the first name is canonical (the
one the formatter emits).

Unless noted, `of` (and `high`/`low`/`close`/`volume`) arguments are
expressions — usually a `Data` series like `close`, but any sub-expression
works. `n`-style arguments are **plain numbers**, not expressions.

### Moving averages, momentum & rolling stats

| Call                    | Arguments               | Meaning                                             |
| ----------------------- | ----------------------- | --------------------------------------------------- |
| `sma` / `average`       | `of`, `n`               | Simple moving average of `of` over `n` days.        |
| `ema`                   | `of`, `n`               | Exponential moving average over `n` days.           |
| `std`                   | `of`, `n`               | Rolling standard deviation over `n` days.           |
| `rsi`                   | `of`, `n`               | Relative Strength Index over `n` days.              |
| `pct_change`            | `of`, `n`               | Percentage change of `of` over `n` days.            |
| `rise`                  | `of`, `n`               | `1` where `of` is above its value `n` days ago, else `0`. |
| `fall`                  | `of`, `n`               | `1` where `of` is below its value `n` days ago, else `0`. |
| `shift`                 | `of`, `n`               | `of` lagged forward by `n` days.                    |
| `rolling_max`           | `of`, `n`               | Rolling maximum over `n` days.                       |
| `rolling_min`           | `of`, `n`               | Rolling minimum over `n` days.                       |

### Bands & channels (single-series composites)

These take one series (`of`, usually `close`) and are composed from the moving
averages / rolling stats above. Bollinger bands warm up with their `std` term
(NaN until row `n-1`), the mid line with `sma` (`min_periods = n/2`); MACD lines
warm up once the slow EMA seeds; Donchian bands at row `n-1`.

| Call              | Arguments                                | Meaning                                                        |
| ----------------- | ---------------------------------------- | -------------------------------------------------------------- |
| `bollinger_mid`   | `of`, `n`                                | Mid band: `sma(of, n)`.                                        |
| `bollinger_upper` | `of`, `n`, `k?`=`2`                      | Upper band: `sma(of, n) + k * std(of, n)`.                     |
| `bollinger_lower` | `of`, `n`, `k?`=`2`                      | Lower band: `sma(of, n) - k * std(of, n)`.                     |
| `macd`            | `of`, `fast?`=`12`, `slow?`=`26`         | MACD line: `ema(of, fast) - ema(of, slow)`.                    |
| `macd_signal`     | `of`, `fast?`=`12`, `slow?`=`26`, `signal?`=`9` | `signal`-day EMA of the MACD line.                      |
| `macd_hist`       | `of`, `fast?`=`12`, `slow?`=`26`, `signal?`=`9` | MACD line minus its signal line.                        |
| `donchian_high`   | `of`, `n`                                | Upper channel: rolling `n`-day high (`rolling_max`).           |
| `donchian_low`    | `of`, `n`                                | Lower channel: rolling `n`-day low (`rolling_min`).            |
| `donchian_mid`    | `of`, `n`                                | Channel mid: `(rolling_max + rolling_min) / 2`.               |

### OHLCV technical indicators

These take price/volume series explicitly (so you decide which series feed them).

| Call        | Arguments                              | Meaning                                       |
| ----------- | -------------------------------------- | --------------------------------------------- |
| `atr`       | `high`, `low`, `close`, `n`            | Average True Range over `n` days.             |
| `natr`      | `high`, `low`, `close`, `n`            | Normalized ATR (percent) over `n` days.       |
| `willr`     | `high`, `low`, `close`, `n`            | Williams %R over `n` days.                     |
| `cci`       | `high`, `low`, `close`, `n`            | Commodity Channel Index over `n` days.         |
| `stoch_k`   | `high`, `low`, `close`, `n`            | Stochastic %K over `n` days.                   |
| `stoch_d`   | `high`, `low`, `close`, `n`, `d?`=`3`  | Stochastic %D: `d`-day average of %K.          |
| `aroon_up`  | `high`, `n`                            | Aroon Up over `n` days (from high).            |
| `aroon_down`| `low`, `n`                             | Aroon Down over `n` days (from low).           |
| `adx`       | `high`, `low`, `close`, `n`            | Average Directional Index over `n` days.       |
| `plus_di`   | `high`, `low`, `close`, `n`            | +DI over `n` days.                             |
| `minus_di`  | `high`, `low`, `close`, `n`            | −DI over `n` days.                             |
| `obv`       | `close`, `volume`                      | On-Balance Volume.                             |
| `mfi`       | `high`, `low`, `close`, `volume`, `n`  | Money Flow Index over `n` days.                |
| `vwap`      | `high`, `low`, `close`, `volume`, `n`  | Volume-Weighted Average Price over `n` days.   |

### Cross-section & selection (per-row, across symbols)

| Call          | Arguments                                    | Meaning                                                                 |
| ------------- | -------------------------------------------- | ---------------------------------------------------------------------- |
| `is_largest`  | `of`, `n`                                    | `1` for the `n` highest values in each row, else `0`.                   |
| `is_smallest` | `of`, `n`                                    | `1` for the `n` lowest values in each row, else `0`.                    |
| `rank`        | `of`, `pct?`=`true`, `ascending?`=`true`     | Cross-sectional rank per row. `pct=true` → `0..1` percentile; `ascending=true` → smallest ranks lowest. |
| `quantile_row`| `of`, `c`                                    | Per-row quantile of `of` across symbols at level `c` (e.g. `0.5` = median). Result is one column named `quantile`. |
| `winsorize`   | `of`, `lower`, `upper`                       | Per-row clip to empirical quantiles `lower`/`upper` in `0..1` (linear interp). NaN preserved. |
| `zscore`      | `of`                                         | Per-row `(x − mean) / std` (population std). Constant rows → `0`; empty → NaN. |
| `bucket`      | `of`, `n`                                    | Per-row quantile buckets labeled `1..=n` (average ranks for ties). Empty/`n=0` → NaN. |
| `demean`      | `of`                                         | Per-row subtract cross-sectional mean of non-NaN cells.                 |
| `mask`        | `of`, `by`                                   | Keep `of` only where `by` is true; drop (NaN) elsewhere.                |
| `normalize_row` | `of`                                       | Scale each row so gross weight (Σ\|w\|) is 1 — explicit portfolio weights. NaN preserved; zero rows unchanged. |
| `vol_target`  | `of`, `prices`, `target?`=`0.1`, `n?`=`63`   | Scale each row of the weight panel `of` toward annualized portfolio-vol `target` over a trailing `n`-return window of `prices`; deleverage only (scale ≤ 1). Warmup / `n<2` / `target≤0` pass through. |
| `industry_rank`| `of`, `categories?`                         | Rank `of` within each industry; optionally restrict to `categories` (list of strings). |
| `cap_industry` | `of`, `max_weight?`=`0.3`                   | Cap each industry's gross weight (Σ\|w\|) in the weight panel `of` at `max_weight`, scaling that industry's names down proportionally (sign-preserving); residual left as cash. |
| `groupby_category`| `of`, `agg`                             | Aggregate `of` within each industry using `agg` (e.g. `"mean"`); `agg` is a required string. |

### Streaks, edges & stateful rotation

| Call         | Arguments                                                                                             | Meaning                                                                                        |
| ------------ | ----------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `sustain`    | `of`, `nwindow`, `nsatisfy?`                                                                           | `1` where `of` was true at least `nsatisfy` times within the last `nwindow` rows.             |
| `is_entry`   | `of`                                                                                                   | `1` on the row where `of` turns false→true (rising edge).                                      |
| `is_exit`    | `of`                                                                                                   | `1` on the row where `of` turns true→false (falling edge).                                     |
| `exit_when`  | `entry`, `exit`                                                                                        | Hold true from an entry edge of `entry` until an exit edge of `entry` or `exit` is true (simpler than `hold_until`; no rotation cap). |
| `hold_until` | `entry`, `exit`, `nstocks_limit?`, `rank?` | Stateful rotation: enter on `entry`, exit on `exit`, hold up to `nstocks_limit` names prioritized by `rank`. **Price stops** (stop-loss / take-profit / trailing) are **not** here — they live in the backtest config (`BacktestConfig::stops`), applied by the NAV loop. `rank` is an **expression**. |
| `rebalance`  | `of`, `freq?`, `on?`                                                                                   | Hold `of`, refreshing on calendar `freq` (`"W"`/`"ME"`/`"QE"`/`"YE"`) or on rows where the `on` expression is true. |

### Neutralization

| Call                  | Arguments                              | Meaning                                                                    |
| --------------------- | -------------------------------------- | ------------------------------------------------------------------------- |
| `neutralize`          | `of`, `by` (list), `add_const?`=`true` | Cross-sectionally regress `of` against the `by` factors and take residuals; `add_const=true` adds an intercept. `by` is a list: `by=[pe, market_cap]`. |
| `neutralize_industry` | `of`, `add_const?`=`true`              | Neutralize `of` within each industry/sector.                               |
| `in_sector`           | `of`, `name` (string)                  | Boolean mask: `1` where the symbol's industry **exactly** equals `name` (case-sensitive); shape follows `of`. Symbols missing from the industry map are `0`. Use with `mask`. |

### Scalar / element-wise unary

| Call    | Arguments | Meaning            |
| ------- | --------- | ------------------ |
| `ceil`  | `of`      | Ceiling of `of`.   |

(Negation is the prefix operator `-`, not a call.)

### Operator ops and leaves

These are not written as calls but are still nodes in the tree:

| Surface form         | Op tag                             | Meaning                                        |
| -------------------- | ---------------------------------- | ---------------------------------------------- |
| `a > b` `a < b` `a >= b` `a <= b` | `Gt` `Lt` `Ge` `Le`  | Comparisons → `1`/`0`.                         |
| `a and b` / `a or b` | `And` / `Or`                       | Logical AND / OR.                              |
| `a + b` `a - b` `a * b` `a / b` | `Add` `Sub` `Mul` `Div` | Element-wise arithmetic.                       |
| `-a`                 | `Neg`                              | Negation (prefix).                             |
| `close`, `pe`, …     | `Data`                             | A raw input series by name (bare identifier).  |
| `42`, `0.5`, `5e8`   | `Const`                            | A constant scalar, broadcast across the panel. A bare number used as an operand is auto-promoted to a `Const`. |

That is the complete surface: **71 op tags** total in the engine — the leaves
`Data` and `Const`, the 10 operator ops above, the prefix ops `Neg` and `not`,
and the 57 function-style calls in the tables.

---

## 5. Worked examples

Each snippet parses; you can check any of them with `lemon fmt` (see
[Validating](#validating)).

### 5.1 A simple filter

```text
close > sma(close, 2)
```

Buy signal where today's close is above the 2-day simple moving average. The `>`
yields a `1`/`0` panel.

### 5.2 Combining conditions

```text
close > sma(close, 50) and rsi(close, 14) < 30
```

Uptrend (`close` above its 50-day average) **and** oversold (14-day RSI below
30). `and` binds looser than the comparisons, so this is
`(close > sma(...)) and (rsi(...) < 30)` — no parentheses needed.

### 5.3 Ranking and top-N selection

```text
is_largest(rank(-pe), 30)
```

`rank(-pe)` ranks stocks by *negated* P/E (so cheaper = higher), then
`is_largest(..., 30)` keeps the top 30 each day. Note `rank` defaults to
`pct=true, ascending=true`; here the `-` flips the ordering instead of passing
`ascending=false`.

### 5.4 `let` for readability

```text
let ma = sma(close, 20)
hold_until(entry = close > ma, exit = close < ma, nstocks_limit = 5)
```

Enter when price crosses above its 20-day average, exit when it crosses below,
holding at most 5 names. `ma` is inlined into both `entry` and `exit` — the tree
contains two copies of `sma(close, 20)`.

### 5.5 Mask by liquidity, then rank

```text
mask(rank(revenue_growth), market_cap > 500000000)
```

Rank stocks by `revenue_growth`, but only keep those with market cap above
$500M (`mask` drops everything where `by` is false). Equivalently the threshold
could be written `5e8`.

### 5.6 Factor neutralization

```text
neutralize(rank(-pe), by=[pe, market_cap])
```

Take the value signal `rank(-pe)` and regress out the `pe` and `market_cap`
factors cross-sectionally, keeping the residual — a "value signal, controlling
for size and raw cheapness."

---

## 6. Sharp edges & gotchas

- **No equality operator.** There is no `==` or `!=`. Compare with
  `> < >= <=`. Logical AND/OR/NOT are the words `and` / `or` / `not` — not
  `&` / `|` / `!`, which are lexer errors.
- **Typos become silent `Data` leaves.** Any unknown bare identifier is treated
  as a series reference with no parse-time validation. `clsoe > 2` parses fine
  and fails only at engine eval. The valid series set is the engine's, not
  lemon's.
- **`let` is inlined, not a variable.** Each use site gets its own copy of the
  subtree. Re-binding the same name is an error, and `format` cannot recover
  `let`s (it emits the expanded tree).
- **Strings have no escapes.** You cannot put a `"` inside a string; there are no
  `\n`, `\t`, `\"` sequences. Strings are only for enum-ish fields (`freq`,
  `agg`, `categories`).
- **Default values worth knowing:**
  - `rank(x)` defaults to `pct=true` (percentile, `0..1`) and `ascending=true`
    (smallest value gets the lowest rank).
  - `stoch_d(...)`'s `d` defaults to `3`.
  - `neutralize(...)` and `neutralize_industry(...)` default `add_const=true`
    (an intercept is added to the regression).
- **`hold_until` is selection only.** `entry`, `exit`, and `rank` are
  expressions; `nstocks_limit` is a number. **Price stops are not part of the
  op** — set `stop_loss` / `take_profit` / `trail_stop` on the backtest config
  (they apply to any strategy, not just `hold_until`).
- **A strategy must be an op node.** A bare `42` or `"ME"` at the top level is
  rejected — the top-level expression has to produce a signal.
- **`n`-style window arguments are numbers, not expressions.** `sma(close, x)`
  where `x` is a series is a type error (`` `n` must be a number ``).

---

## <a id="validating"></a>Validating a snippet

The `lemon-cli` crate ships the `lemon` binary — a formatter that parses first,
so it doubles as a syntax checker. Pipe source on stdin:

```sh
printf '%s' 'close > sma(close, 2)' | cargo run -q -p lemon-cli -- fmt
```

It prints the canonical (re-indented) form on success, or `line:col: message` on
a parse error with a non-zero exit code. `lemon fmt -w file.lemon` formats files
in place.

### Linting

`lemon lint` reports the two mistakes the parser cannot catch — **unknown
series names** (a typo like `clsoe` parses fine as a `Data` leaf and only fails
at engine eval) and **unused `let` bindings** (inlined at parse time, so they
vanish silently):

```sh
printf '%s' 'clsoe > 1' | cargo run -q -p lemon-cli -- lint --series close,pe
# <stdin>:1:1: warning: unknown series `clsoe` — did you mean `close`?
```

Pass the valid series set with `--series a,b,c` or `--series-file path` (one
name per line, `#` comments); without it only the unused-`let` check runs.
Exit code is non-zero when there are warnings. The same checks are exposed to
the web editor via `lemon-wasm`'s `lint(src, series_json)` export and to Rust
callers as `lemon::lint(src, known_series)`.

---

## Running a strategy

`lemon run` lowers a `.lemon` file and backtests it over a local
[data-layout tree](../reference/data-layout) in one step:

```sh
lemon run strategy.lemon --data ~/qdata --from 20180101 --to 20241231 --fee-ratio 0.001
```

It prints the engine's `Report` JSON (equity curve, trades, metrics — see
[reading a report](../reference/backtest-engine)) to stdout; `--out report.json`
writes a file instead. The flags mirror `yuzu-cli run`: `--slippage-ratio`,
`--price-key open|high|low|close`, `--benchmark SPY`. With no file it reads
stdin, like the other subcommands.

Two scope notes, shared with `yuzu-cli`: the CLI data loader reads **close**
(plus **volume** / a **benchmark** symbol when the config needs them) from
`prices/` — strategies referencing fundamentals (`pe`, …) need `yuzu-server`
or library code for now (see [`data-layout.md`](../reference/data-layout)); and the
full stop/delist/bootstrap knob set stays on `yuzu-cli run`.

---

## See also

- [`backtest-engine.md`](../reference/backtest-engine) — engine semantics: the `Panel`
  data model, NaN handling, alignment rules, and per-op numerical behavior.
- Source of truth for this reference:
  `crates/lemon-lang/src/dsl/{lex,parse,ops,print}.rs` and
  `crates/lemon-lang/src/spec.rs`.

