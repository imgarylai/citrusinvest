use ndarray::Array2;
use std::fs;
use std::path::PathBuf;
use yuzu_core::panel::Panel;

pub fn load_golden(name: &str) -> serde_json::Value {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push(format!("tests/golden/{name}.json"));
    let txt = fs::read_to_string(&p).unwrap_or_else(|_| panic!("missing fixture {name}"));
    serde_json::from_str(&txt).unwrap()
}

fn date_to_i32(s: &str) -> i32 {
    s.replace('-', "").parse().unwrap()
}

pub fn panel_from_json(v: &serde_json::Value, key: &str) -> Panel {
    // Per-key axis overrides: rebalance downsamples rows ("expected_dates"),
    // quantile_row collapses to one column ("expected_symbols"). Fall back to the
    // shared top-level "dates"/"symbols" when no override is present.
    let dates_key = format!("{key}_dates");
    let dates_val = if v.get(&dates_key).is_some() {
        &v[&dates_key]
    } else {
        &v["dates"]
    };
    let dates: Vec<i32> = dates_val
        .as_array()
        .unwrap()
        .iter()
        .map(|d| date_to_i32(d.as_str().unwrap()))
        .collect();
    let symbols_key = format!("{key}_symbols");
    let symbols_val = if v.get(&symbols_key).is_some() {
        &v[&symbols_key]
    } else {
        &v["symbols"]
    };
    let symbols: Vec<String> = symbols_val
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let rows = v[key].as_array().unwrap();
    let nrows = rows.len();
    let ncols = symbols.len();
    let mut data = Array2::from_elem((nrows, ncols), f64::NAN);
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.as_array().unwrap().iter().enumerate() {
            data[[r, c]] = if cell.is_null() {
                f64::NAN
            } else {
                cell.as_f64().unwrap()
            };
        }
    }
    Panel::new(dates, symbols, data).unwrap()
}

pub fn assert_panel_eq(got: &Panel, expected: &Panel, tol: f64) {
    assert_eq!(got.data.dim(), expected.data.dim(), "shape mismatch");
    for ((r, c), &g) in got.data.indexed_iter() {
        let e = expected.data[[r, c]];
        if e.is_nan() {
            assert!(g.is_nan(), "want NaN at [{r},{c}], got {g}");
        } else {
            assert!((g - e).abs() <= tol, "at [{r},{c}] want {e}, got {g}");
        }
    }
}
