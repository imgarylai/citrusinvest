//! Python bindings for the yuzu backtest engine and the lemon strategy DSL.
//!
//! The engine boundary is pure data — `(spec JSON, panels, config) → Report` —
//! so this layer only converts Python objects to panels/config and the Report
//! back to Python dicts/lists. `NaN` cells survive the round trip as
//! `float('nan')` in series data; metric fields that the engine defines as NaN
//! also come back as `float('nan')`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};
use std::collections::HashMap;

use yuzu_core::backtest::BacktestConfig;
use yuzu_core::panel::Panel;
use yuzu_core::EvalContext;

// ---- Python -> JSON (for specs given as dicts) --------------------------------

fn py_to_json(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if obj.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(serde_json::Value::Bool(b.is_true()));
    }
    if let Ok(i) = obj.cast::<PyInt>() {
        return Ok(serde_json::Value::from(i.extract::<i64>()?));
    }
    if let Ok(f) = obj.cast::<PyFloat>() {
        let v = f.value();
        return serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .ok_or_else(|| PyValueError::new_err("non-finite numbers are not valid in a spec"));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(serde_json::Value::String(s.to_string()));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let mut out = Vec::with_capacity(list.len());
        for item in list.iter() {
            out.push(py_to_json(&item)?);
        }
        return Ok(serde_json::Value::Array(out));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            map.insert(k.extract::<String>()?, py_to_json(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }
    Err(PyValueError::new_err(format!(
        "unsupported value in spec: {}",
        obj.get_type().name()?
    )))
}

// ---- JSON -> Python (for the Report and parse output) -------------------------

fn json_to_py<'py>(py: Python<'py>, value: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        serde_json::Value::Null => py.None().into_bound(py),
        serde_json::Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any()
            } else {
                n.as_f64().unwrap_or(f64::NAN).into_pyobject(py)?.into_any()
            }
        }
        serde_json::Value::String(s) => PyString::new(py, s).into_any(),
        serde_json::Value::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any()
        }
        serde_json::Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, json_to_py(py, v)?)?;
            }
            dict.into_any()
        }
    })
}

// ---- spec: lemon text | JSON string | dict ------------------------------------

fn spec_to_json_string(spec: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(s) = spec.extract::<String>() {
        // A string is either the JSON Expr tree or lemon source.
        if s.trim_start().starts_with('{') {
            return Ok(s);
        }
        let tree =
            lemon::parse(&s).map_err(|e| PyValueError::new_err(format!("lemon parse: {e}")))?;
        return serde_json::to_string(&tree).map_err(|e| PyValueError::new_err(e.to_string()));
    }
    let value = py_to_json(spec)?;
    if !value.is_object() {
        return Err(PyValueError::new_err(
            "spec must be lemon source, a JSON string, or an Expr dict",
        ));
    }
    serde_json::to_string(&value).map_err(|e| PyValueError::new_err(e.to_string()))
}

// ---- panels: dict {dates,symbols,data} or DataFrame duck-type ------------------

/// One date label: an int `YYYYMMDD`, or anything with `strftime` (datetime,
/// pandas Timestamp), or a `YYYY-MM-DD` string.
fn date_to_i32(obj: &Bound<'_, PyAny>) -> PyResult<i32> {
    if let Ok(i) = obj.extract::<i32>() {
        return Ok(i);
    }
    if obj.hasattr("strftime")? {
        let s: String = obj.call_method1("strftime", ("%Y%m%d",))?.extract()?;
        return s
            .parse::<i32>()
            .map_err(|e| PyValueError::new_err(format!("bad date `{s}`: {e}")));
    }
    if let Ok(s) = obj.extract::<String>() {
        let compact: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        return compact
            .parse::<i32>()
            .map_err(|e| PyValueError::new_err(format!("bad date `{s}`: {e}")));
    }
    Err(PyValueError::new_err(format!(
        "cannot interpret `{}` as a YYYYMMDD date",
        obj.get_type().name()?
    )))
}

fn cell_to_f64(obj: &Bound<'_, PyAny>) -> PyResult<f64> {
    if obj.is_none() {
        return Ok(f64::NAN);
    }
    obj.extract::<f64>()
        .map_err(|_| PyValueError::new_err("panel cells must be numbers or None"))
}

fn rows_to_panel(
    name: &str,
    dates: Vec<i32>,
    symbols: Vec<String>,
    rows: &Bound<'_, PyAny>,
) -> PyResult<Panel> {
    let (nrows, ncols) = (dates.len(), symbols.len());
    let mut data = ndarray::Array2::from_elem((nrows, ncols), f64::NAN);
    for (r, row) in rows.try_iter()?.enumerate() {
        let row = row?;
        if r >= nrows {
            return Err(PyValueError::new_err(format!(
                "panel `{name}`: more data rows than dates"
            )));
        }
        for (c, cell) in row.try_iter()?.enumerate() {
            if c >= ncols {
                return Err(PyValueError::new_err(format!(
                    "panel `{name}`: row {r} has more cells than symbols"
                )));
            }
            data[[r, c]] = cell_to_f64(&cell?)?;
        }
    }
    Panel::new(dates, symbols, data)
        .map_err(|e| PyValueError::new_err(format!("panel `{name}`: {e}")))
}

fn panel_from_py(name: &str, obj: &Bound<'_, PyAny>) -> PyResult<Panel> {
    // pandas/polars-style duck-type: .index / .columns / .to_numpy()
    if obj.hasattr("columns")? && obj.hasattr("index")? {
        let mut dates = Vec::new();
        for d in obj.getattr("index")?.try_iter()? {
            dates.push(date_to_i32(&d?)?);
        }
        let mut symbols = Vec::new();
        for s in obj.getattr("columns")?.try_iter()? {
            symbols.push(s?.str()?.to_string());
        }
        let rows = obj.call_method0("to_numpy")?.call_method0("tolist")?;
        return rows_to_panel(name, dates, symbols, &rows);
    }
    // plain mapping: {"dates": [...], "symbols": [...], "data": [[...]]}
    if let Ok(dict) = obj.cast::<PyDict>() {
        let get = |key: &str| {
            dict.get_item(key)?.ok_or_else(|| {
                PyValueError::new_err(format!("panel `{name}`: missing key `{key}`"))
            })
        };
        let mut dates = Vec::new();
        for d in get("dates")?.try_iter()? {
            dates.push(date_to_i32(&d?)?);
        }
        let mut symbols = Vec::new();
        for s in get("symbols")?.try_iter()? {
            symbols.push(s?.extract::<String>()?);
        }
        return rows_to_panel(name, dates, symbols, &get("data")?);
    }
    Err(PyValueError::new_err(format!(
        "panel `{name}` must be a DataFrame or a dict with dates/symbols/data"
    )))
}

// ---- config: dict of the BacktestConfig knobs ----------------------------------

fn config_from_py(config: Option<&Bound<'_, PyDict>>) -> PyResult<BacktestConfig> {
    let mut cfg = BacktestConfig::default();
    let Some(dict) = config else { return Ok(cfg) };
    for (key, value) in dict.iter() {
        let key: String = key.extract()?;
        match key.as_str() {
            "fee_ratio" => cfg.fee_ratio = value.extract()?,
            "tax_ratio" => cfg.tax_ratio = value.extract()?,
            "position_limit" => cfg.position_limit = value.extract()?,
            "slippage_ratio" => cfg.slippage_ratio = value.extract()?,
            "initial_capital" => cfg.initial_capital = value.extract()?,
            "max_participation" => cfg.max_participation = value.extract()?,
            "impact_coef" => cfg.impact_coef = value.extract()?,
            "delist_after" => cfg.delist_after = value.extract()?,
            "delist_haircut" => cfg.delist_haircut = value.extract()?,
            "benchmark_key" => cfg.benchmark_key = value.extract()?,
            "bootstrap_samples" => cfg.bootstrap_samples = value.extract()?,
            "bootstrap_block" => cfg.bootstrap_block = value.extract()?,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown config key `{other}`"
                )))
            }
        }
    }
    Ok(cfg)
}

// ---- exported functions ---------------------------------------------------------

/// Run a backtest. `spec` is lemon source, a JSON string, or an Expr dict;
/// `panels` maps series names to DataFrames or `{dates, symbols, data}` dicts;
/// `config` takes the `BacktestConfig` knobs by name. Returns the Report as a
/// dict (the same JSON contract the WASM/server boundaries emit).
#[pyfunction]
#[pyo3(signature = (spec, panels, config=None, price_key="close", industry=None))]
fn run_backtest(
    py: Python<'_>,
    spec: &Bound<'_, PyAny>,
    panels: &Bound<'_, PyDict>,
    config: Option<&Bound<'_, PyDict>>,
    price_key: &str,
    industry: Option<HashMap<String, String>>,
) -> PyResult<Py<PyAny>> {
    let spec_json = spec_to_json_string(spec)?;
    let cfg = config_from_py(config)?;
    let mut panel_map = HashMap::new();
    for (name, value) in panels.iter() {
        let name: String = name.extract()?;
        let panel = panel_from_py(&name, &value)?;
        panel_map.insert(name, panel);
    }
    let ctx = EvalContext {
        panels: panel_map,
        industry: industry.unwrap_or_default(),
    };
    let report = yuzu_core::run_backtest(&spec_json, &ctx, price_key, &cfg)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let value = serde_json::to_value(&report).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(json_to_py(py, &value)?.unbind())
}

/// Parse lemon source into the JSON Expr tree (as a dict).
#[pyfunction]
fn parse(py: Python<'_>, src: &str) -> PyResult<Py<PyAny>> {
    let tree = lemon::parse(src).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(json_to_py(py, &tree)?.unbind())
}

/// Render an Expr tree (dict or JSON string) back to canonical lemon source.
#[pyfunction(name = "format")]
fn format_(tree: &Bound<'_, PyAny>) -> PyResult<String> {
    let value = if let Ok(s) = tree.extract::<String>() {
        serde_json::from_str(&s).map_err(|e| PyValueError::new_err(e.to_string()))?
    } else {
        py_to_json(tree)?
    };
    Ok(lemon::format(&value))
}

/// Lint lemon source. `series` enables the unknown-series check; unused-`let`
/// is always checked. Returns `[{"line", "col", "message"}, ...]`.
#[pyfunction]
#[pyo3(signature = (src, series=None))]
fn lint(py: Python<'_>, src: &str, series: Option<Vec<String>>) -> PyResult<Py<PyAny>> {
    let lints =
        lemon::lint(src, series.as_deref()).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let list = PyList::empty(py);
    for l in lints {
        let d = PyDict::new(py);
        d.set_item("line", l.line)?;
        d.set_item("col", l.col)?;
        d.set_item("message", &l.message)?;
        list.append(d)?;
    }
    Ok(list.into_any().unbind())
}

#[pymodule]
fn yuzu(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_backtest, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(format_, m)?)?;
    m.add_function(wrap_pyfunction!(lint, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
