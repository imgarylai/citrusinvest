"""Smoke tests for the yuzu Python bindings.

Runnable with pytest or plain `python3 tests/test_yuzu.py`. The DataFrame test
uses a minimal stand-in implementing the duck-typed surface (.index, .columns,
.to_numpy().tolist()) so pandas is not a test dependency; a real DataFrame
exposes the same surface.
"""

import datetime
import math

import yuzu

CLOSE = {
    "dates": [20240102, 20240103, 20240104, 20240105],
    "symbols": ["A"],
    "data": [[10.0], [11.0], [12.0], [13.0]],
}


def test_run_backtest_from_lemon_source():
    report = yuzu.run_backtest(
        "close > sma(close, 2)", panels={"close": CLOSE}, config={"fee_ratio": 0.001}
    )
    assert len(report["equity"]) == 4
    assert report["equity"][0] == 1.0  # signal needs one bar of SMA warmup
    assert report["metrics"]["total_return"] > 0
    assert report["dates"] == CLOSE["dates"]
    assert report["monthly_returns"][0]["period"] == "2024-01"


def test_spec_as_dict_with_benchmark_and_bootstrap():
    spec = {"op": "Gt", "l": {"op": "Data", "name": "close"}, "r": {"op": "Const", "value": 0.0}}
    spy = {
        "dates": CLOSE["dates"],
        "symbols": ["SPY"],
        "data": [[100.0], [101.0], [102.0], [103.0]],
    }
    report = yuzu.run_backtest(
        spec,
        panels={"close": CLOSE, "spy": spy},
        config={"benchmark_key": "spy", "bootstrap_samples": 50},
    )
    assert len(report["benchmark"]) == 4
    assert "beta" in report["metrics"]
    assert report["bootstrap"]["n_samples"] == 50
    band = report["bootstrap"]["sharpe"]
    assert band["p05"] <= band["p50"] <= band["p95"]


class FakeFrame:
    """The duck-typed surface panel_from_py reads off a pandas DataFrame."""

    def __init__(self, index, columns, rows):
        self.index = index
        self.columns = columns
        self._rows = rows

    def to_numpy(self):
        rows = self._rows

        class _A:  # noqa: N801 - mimics ndarray.tolist()
            def tolist(self):
                return rows

        return _A()


def test_dataframe_duck_type_and_datetime_index():
    frame = FakeFrame(
        index=[datetime.date(2024, 1, 2), datetime.date(2024, 1, 3), "2024-01-04"],
        columns=["A", "B"],
        rows=[[10.0, 20.0], [11.0, None], [12.0, 22.0]],
    )
    report = yuzu.run_backtest("close > 0", panels={"close": frame})
    assert report["dates"] == [20240102, 20240103, 20240104]
    assert len(report["equity"]) == 3


def test_parse_format_lint_round_trip():
    tree = yuzu.parse("close > sma(close, 2)")
    assert tree["op"] == "Gt"
    assert yuzu.format(tree) == "(close > sma(close, 2))"

    lints = yuzu.lint("clsoe > 1", ["close", "pe"])
    assert lints[0]["line"] == 1 and "did you mean `close`" in lints[0]["message"]
    assert yuzu.lint("close > 1", ["close"]) == []


def test_errors_are_value_errors():
    for bad in [
        lambda: yuzu.run_backtest("sma(close,", panels={"close": CLOSE}),  # parse error
        lambda: yuzu.run_backtest("close > 1", panels={"close": CLOSE}, config={"bogus": 1}),
        lambda: yuzu.run_backtest("close > 1", panels={"close": {"dates": [1]}}),  # missing keys
        lambda: yuzu.parse("42"),  # bare constant
    ]:
        try:
            bad()
        except ValueError:
            continue
        raise AssertionError("expected ValueError")


def test_nan_and_none_cells_are_missing_data():
    panel = {
        "dates": [20240102, 20240103, 20240104],
        "symbols": ["A"],
        "data": [[10.0], [None], [float("nan")]],
    }
    report = yuzu.run_backtest("close > 0", panels={"close": panel})
    # missing prices -> zero return days; equity stays finite
    assert all(math.isfinite(e) for e in report["equity"])


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"{name} ok")
    print("ALL OK")
