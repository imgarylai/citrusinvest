# yuzu — Python bindings

Python bindings for the [citrusinvest](https://github.com/imgarylai/citrusinvest)
backtest engine: the **yuzu** backtest core plus the **lemon** strategy DSL,
compiled from Rust. The engine boundary is pure data —
`(strategy, panels, config) → report` — so the bindings are thin and fast.

```python
import yuzu

report = yuzu.run_backtest(
    "close > sma(close, 20)",          # lemon source (or a JSON Expr dict/string)
    panels={
        "close": df_close,             # DataFrame (dates × symbols), or a
                                       # {"dates": [...], "symbols": [...], "data": [[...]]} dict
    },
    config={"fee_ratio": 0.001, "benchmark_key": "spy"},
)
report["equity"]                        # NAV curve, base 1.0
report["metrics"]["sharpe"]             # headline metrics
report["monthly_returns"]               # calendar tables, trades, bootstrap bands, ...
```

The report is the same JSON contract the engine's WASM and server boundaries
emit — see `docs/backtest-engine.md` in the repository for the full schema and
the `BacktestConfig` knobs (fees, slippage, square-root market impact,
liquidity cap, delisting handling, benchmark comparison, bootstrap bands).

DSL tooling is included:

```python
yuzu.parse("close > sma(close, 20)")   # -> Expr tree as a dict
yuzu.format(tree)                       # -> canonical lemon source
yuzu.lint("clsoe > 1", ["close", "pe"]) # -> [{"line": 1, "col": 1, "message": "unknown series `clsoe` — did you mean `close`?"}]
```

## Panels

A panel is a dense dates × symbols matrix of floats (`None`/`NaN` = missing):

- **DataFrame** (pandas/polars duck-type): index = dates (`int` `YYYYMMDD`,
  strings, or anything with `strftime`), columns = symbols.
- **dict**: `{"dates": [20240102, ...], "symbols": ["AAPL", ...], "data": [[...], ...]}`.

Series names are the engine's (`close`, `high`, `low`, `volume`, `pe`, …); a
`volume` panel is required for the liquidity cap / market-impact features, and
`high`/`low` enable per-trade MAE/MFE.

## Install

```bash
pip install yuzu-backtest          # from PyPI (distribution name), imports as `yuzu`
```

## Build from source

```bash
pip install maturin
pip install ./crates/yuzu-py       # or: maturin develop -m crates/yuzu-py/Cargo.toml
```

Wheels are abi3 (Python ≥ 3.9). License: MIT.

**Versioning**: `major.minor` mirrors the engine workspace version (a
`0.2.x` wheel is built from the `0.2` engine series); the patch segment is
the bindings' own, so binding-only fixes can ship without an engine release.
Each wheel is a snapshot of the whole repository at its release tag.
