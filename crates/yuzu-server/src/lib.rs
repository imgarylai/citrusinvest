//! Native backtest server core. `handle_backtest` is the source-agnostic unit:
//! it reads only the price/fundamental series a spec references (from any
//! `ObjectSource` — `LocalSource` in tests, `S3Source`/R2 in the container),
//! assembles an `EvalContext`, and runs `yuzu_core::run_backtest`. `main.rs`
//! wraps it in a tiny HTTP server. No Cloudflare specifics live here.

mod cache;

use std::collections::{BTreeSet, HashMap};

use serde::Deserialize;
use serde_json::Value;

use yuzu_core::backtest::BacktestConfig;
use yuzu_core::report::Report;
use yuzu_core::{run_backtest, EvalContext};
use yuzu_data::{
    is_fundamental_series, load_combined_panel, load_fundamental_panel, load_panel,
    rebuild_combined_panels, Field, ObjectSink, ObjectSource, RebuildSummary, FUNDAMENTALS_DIR,
    PANELS_DIR, PRICES_DIR,
};

/// Object-key layout, overridable via env so a custom bucket prefix works.
pub struct DataDirs {
    pub prices: String,
    pub fundamentals: String,
    pub panels: String,
}

impl Default for DataDirs {
    fn default() -> Self {
        DataDirs {
            prices: PRICES_DIR.to_string(),
            fundamentals: FUNDAMENTALS_DIR.to_string(),
            panels: PANELS_DIR.to_string(),
        }
    }
}

impl DataDirs {
    /// `YUZU_PRICES_DIR` / `YUZU_FUNDAMENTALS_DIR` / `YUZU_PANELS_DIR`, each defaulting to the constant.
    pub fn from_env() -> Self {
        let d = DataDirs::default();
        DataDirs {
            prices: std::env::var("YUZU_PRICES_DIR").unwrap_or(d.prices),
            fundamentals: std::env::var("YUZU_FUNDAMENTALS_DIR").unwrap_or(d.fundamentals),
            panels: std::env::var("YUZU_PANELS_DIR").unwrap_or(d.panels),
        }
    }
}

/// One backtest request. `spec` is the strategy `Expr` JSON tree (object).
#[derive(Deserialize, Default)]
pub struct BacktestRequest {
    pub spec: Value,
    pub symbols: Vec<String>,
    pub from: i32,
    pub to: i32,
    #[serde(default)]
    pub fee_ratio: f64,
    #[serde(default)]
    pub position_limit: f64,
    #[serde(default = "default_price_key")]
    pub price_key: String,
    #[serde(default)]
    pub slippage_ratio: f64,
    #[serde(default)]
    pub initial_capital: f64,
    #[serde(default)]
    pub max_participation: f64,
    #[serde(default)]
    pub delist_after: usize,
    #[serde(default)]
    pub delist_haircut: f64,
    /// Symbol to load (close prices) as the benchmark, e.g. "SPY". Loaded into
    /// a dedicated "benchmark" panel — it does not need to be in `symbols`.
    #[serde(default)]
    pub benchmark_symbol: Option<String>,
    #[serde(default)]
    pub bootstrap_samples: usize,
    #[serde(default)]
    pub bootstrap_block: usize,
}

fn default_price_key() -> String {
    "close".to_string()
}

/// One rebuild request — the Worker passes the symbol universe (the container
/// stays list-free).
#[derive(Deserialize)]
pub struct RebuildRequest {
    pub symbols: Vec<String>,
}

/// Rebuild every combined panel file from the per-symbol archives.
pub fn handle_rebuild<S: ObjectSource + ObjectSink + Sync>(
    source: &S,
    req: &RebuildRequest,
    dirs: &DataDirs,
) -> Result<RebuildSummary, String> {
    rebuild_combined_panels(
        source,
        &req.symbols,
        &dirs.prices,
        &dirs.fundamentals,
        &dirs.panels,
    )
    .map_err(|e| e.to_string())
}

/// Map a `Data` series name to an OHLCV field (None ⇒ not a price series).
fn price_field(name: &str) -> Option<Field> {
    match name {
        "close" => Some(Field::AdjClose),
        "open" => Some(Field::AdjOpen),
        "high" => Some(Field::AdjHigh),
        "low" => Some(Field::AdjLow),
        "volume" => Some(Field::Volume),
        _ => None,
    }
}

/// Walk the spec tree and collect every `{ "op": "Data", "name": ... }` name.
fn collect_series(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(map) => {
            if map.get("op").and_then(Value::as_str) == Some("Data") {
                if let Some(name) = map.get("name").and_then(Value::as_str) {
                    out.insert(name.to_string());
                }
            }
            for v in map.values() {
                collect_series(v, out);
            }
        }
        Value::Array(arr) => arr.iter().for_each(|v| collect_series(v, out)),
        _ => {}
    }
}

/// Load referenced panels from `source`, run the backtest, return the report.
///
/// Only the series the spec references are loaded (plus `price_key`); a series
/// that is neither a known OHLCV field nor a fundamental field is skipped — if
/// it's actually needed, `run_backtest` surfaces a clear "unknown series" error.
pub fn handle_backtest<S: ObjectSource + Sync>(
    source: &S,
    req: &BacktestRequest,
    dirs: &DataDirs,
) -> Result<Report, String> {
    let mut names = BTreeSet::new();
    collect_series(&req.spec, &mut names);
    names.insert(req.price_key.clone()); // price panel must be present even if unreferenced
                                         // Always load high/low so per-trade MAE/MFE can be computed even when the
                                         // strategy never references them. They come from the same prices/{sym}.csv.gz
                                         // as close, so availability tracks close; a missing file yields a NaN column
                                         // and the engine degrades that trade's mae/mfe to None.
    names.insert("high".to_string());
    names.insert("low".to_string());
    // The liquidity cap needs dollar volume.
    if req.max_participation > 0.0 && req.initial_capital > 0.0 {
        names.insert("volume".to_string());
    }

    let mut panels: HashMap<String, _> = HashMap::new();
    for name in &names {
        let price_field_opt = price_field(name);
        if price_field_opt.is_none() && !is_fundamental_series(name) {
            continue; // unknown series — run_backtest surfaces it if actually needed
        }
        let p = cache::get_or_load(name, &req.symbols, req.from, req.to, || {
            // Combined file first (1 GET); fall back to per-symbol only when the
            // combined file is ABSENT (file-level, not symbol-level). A symbol not
            // in the combined header yields a NaN column until the next nightly
            // rebuild — e.g. a symbol added between its per-symbol archive build and
            // the panel rebuild. Bounded to ≤1 rebuild cycle; NaN (excluded), never
            // wrong data. Deliberate per the design's "fall back when absent".
            if let Some(panel) =
                load_combined_panel(source, name, &req.symbols, req.from, req.to, &dirs.panels)
                    .map_err(|e| e.to_string())?
            {
                return Ok(panel);
            }
            if let Some(field) = price_field_opt {
                load_panel(source, &req.symbols, field, req.from, req.to, &dirs.prices)
                    .map_err(|e| e.to_string())
            } else {
                load_fundamental_panel(
                    source,
                    &req.symbols,
                    name,
                    req.from,
                    req.to,
                    &dirs.fundamentals,
                )
                .map_err(|e| e.to_string())
            }
        })?;
        panels.insert(name.clone(), p);
    }

    // Per-request diagnostics (visible in container logs): symbol count, window,
    // and each loaded panel's dims + how many cells are non-NaN. An all-NaN
    // fundamental panel here means the data didn't line up with the price grid.
    eprintln!(
        "[yuzu] backtest: {} symbols, {}..{}, price_key={}",
        req.symbols.len(),
        req.from,
        req.to,
        req.price_key
    );
    for (name, p) in &panels {
        let non_nan = p.data.iter().filter(|v| !v.is_nan()).count();
        eprintln!(
            "[yuzu]   panel {name}: {}x{} (rows×cols), {} non-NaN",
            p.nrows(),
            p.ncols(),
            non_nan
        );
    }

    // Benchmark: one extra close panel for the named symbol, under the
    // reserved "benchmark" key (independent of the strategy universe).
    let mut benchmark_key = None;
    if let Some(sym) = &req.benchmark_symbol {
        let p = load_panel(
            source,
            std::slice::from_ref(sym),
            Field::AdjClose,
            req.from,
            req.to,
            &dirs.prices,
        )
        .map_err(|e| e.to_string())?;
        panels.insert("benchmark".to_string(), p);
        benchmark_key = Some("benchmark".to_string());
    }

    let ctx = EvalContext::new(panels);
    let cfg = BacktestConfig {
        fee_ratio: req.fee_ratio,
        position_limit: req.position_limit,
        slippage_ratio: req.slippage_ratio,
        initial_capital: req.initial_capital,
        max_participation: req.max_participation,
        delist_after: req.delist_after,
        delist_haircut: req.delist_haircut,
        benchmark_key,
        bootstrap_samples: req.bootstrap_samples,
        bootstrap_block: req.bootstrap_block,
        ..Default::default()
    };
    run_backtest(&req.spec.to_string(), &ctx, &req.price_key, &cfg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use yuzu_data::csv_io::{write_series, OhlcvRow};
    use yuzu_data::fundamentals::{write_fundamentals, FundamentalRow};
    use yuzu_data::{LocalSource, FUNDAMENTAL_FIELDS};

    fn ohlcv(day: i32, c: f64) -> OhlcvRow {
        OhlcvRow {
            day,
            adj_open: c,
            adj_high: c,
            adj_low: c,
            adj_close: c,
            volume: 1000.0,
        }
    }

    /// A fundamentals row with `pe` set (rest NaN) and an explicit report-event flag.
    fn frow(day: i32, pe: f64, report_event: f64) -> FundamentalRow {
        let mut values = vec![f64::NAN; FUNDAMENTAL_FIELDS.len()];
        values[0] = pe;
        FundamentalRow {
            day,
            values,
            report_event,
        }
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prices")).unwrap();
        std::fs::create_dir_all(dir.join("fundamentals")).unwrap();
        dir
    }

    fn write_symbol(dir: &Path, sym: &str, prices: &[OhlcvRow], funds: &[FundamentalRow]) {
        std::fs::write(
            dir.join(format!("prices/{sym}.csv.gz")),
            write_series(prices).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("fundamentals/{sym}.csv.gz")),
            write_fundamentals(funds).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn runs_backtest_from_local_source() {
        let dir = fixture("yuzu_server_handler_test");
        let days = [20240102, 20240103, 20240104];
        // AAA cheaper (low pe), BBB pricier; both have rising close.
        for (sym, base, pe) in [("AAA", 10.0, 8.0), ("BBB", 20.0, 15.0)] {
            let prices: Vec<_> = days
                .iter()
                .enumerate()
                .map(|(i, d)| ohlcv(*d, base + i as f64))
                .collect();
            let funds: Vec<_> = days.iter().map(|d| frow(*d, pe, 0.0)).collect();
            write_symbol(&dir, sym, &prices, &funds);
        }

        let source = LocalSource::new(&dir);
        // Hold the single lowest-pe name each day — references close (price) + pe (fundamental).
        let req = BacktestRequest {
            spec: serde_json::json!({ "op": "IsSmallest", "of": { "op": "Data", "name": "pe" }, "n": 1 }),
            symbols: vec!["AAA".into(), "BBB".into()],
            from: 20240102,
            to: 20240104,
            fee_ratio: 0.0,
            position_limit: 0.0,
            price_key: "close".into(),
            ..Default::default()
        };

        let report = handle_backtest(&source, &req, &DataDirs::default()).unwrap();
        assert_eq!(report.dates.len(), report.equity.len());
        assert!(
            !report.equity.is_empty(),
            "equity curve should be non-empty"
        );

        // Same request with a benchmark symbol: the report grows a rebased
        // benchmark curve + relative metrics (BBB rises 20→22 = +10%).
        let req_b = BacktestRequest {
            benchmark_symbol: Some("BBB".into()),
            spec: req.spec.clone(),
            symbols: req.symbols.clone(),
            from: req.from,
            to: req.to,
            price_key: "close".into(),
            ..Default::default()
        };
        let with_bench = handle_backtest(&source, &req_b, &DataDirs::default()).unwrap();
        let curve = with_bench.benchmark.as_ref().unwrap();
        assert_eq!(curve.len(), with_bench.dates.len());
        assert!((curve.last().unwrap() - 1.1).abs() < 1e-9);
        assert!(with_bench.metrics.beta.is_some());
    }

    #[test]
    fn force_loads_high_low_for_mae_mfe() {
        let dir = fixture("yuzu_server_mae_mfe_test");
        let days = [20240102, 20240103, 20240104];
        // Single symbol with DISTINCT intraday high/low; close-only spec below never
        // references high/low — so non-None mae/mfe proves they were force-loaded.
        let prices = vec![
            OhlcvRow {
                day: days[0],
                adj_open: 10.0,
                adj_high: 10.0,
                adj_low: 9.0,
                adj_close: 10.0,
                volume: 1000.0,
            },
            OhlcvRow {
                day: days[1],
                adj_open: 11.0,
                adj_high: 13.0,
                adj_low: 11.0,
                adj_close: 11.0,
                volume: 1000.0,
            },
            OhlcvRow {
                day: days[2],
                adj_open: 12.0,
                adj_high: 12.0,
                adj_low: 12.0,
                adj_close: 12.0,
                volume: 1000.0,
            },
        ];
        let funds: Vec<FundamentalRow> = days.iter().map(|d| frow(*d, 8.0, 0.0)).collect();
        write_symbol(&dir, "AAA", &prices, &funds);

        let source = LocalSource::new(&dir);
        let req = BacktestRequest {
            // close > 0 → always held; references only `close`, NOT high/low.
            spec: serde_json::json!({
                "op": "Gt",
                "l": { "op": "Data", "name": "close" },
                "r": { "op": "Const", "value": 0.0 }
            }),
            symbols: vec!["AAA".into()],
            from: 20240102,
            to: 20240104,
            fee_ratio: 0.0,
            position_limit: 0.0,
            price_key: "close".into(),
            ..Default::default()
        };

        let report = handle_backtest(&source, &req, &DataDirs::default()).unwrap();
        let t = report
            .trades
            .iter()
            .find(|t| t.symbol == "AAA")
            .expect("AAA trade");
        // ep = close day0 = 10; long, open trade over all 3 days.
        // MFE from high 13 (day1) → 0.3; MAE from low 9 (day0) → -0.1.
        assert!(
            (t.mfe.unwrap() - 0.3).abs() < 1e-9,
            "mfe from force-loaded high"
        );
        assert!(
            (t.mae.unwrap() - (-0.1)).abs() < 1e-9,
            "mae from force-loaded low"
        );
    }

    #[test]
    fn loads_report_event_series() {
        let dir = fixture("yuzu_server_report_event_test");
        let days = [20240102, 20240103, 20240104];
        // Distinct symbols from the other test — the panel cache is a global keyed
        // by (name, symbols, window), so reusing AAA/BBB would cross-contaminate.
        // RPTA files a report on day 3; RPTB never does.
        for (sym, base, pe, event_day) in [("RPTA", 10.0, 8.0, 20240103), ("RPTB", 20.0, 15.0, 0)] {
            let prices: Vec<_> = days
                .iter()
                .enumerate()
                .map(|(i, d)| ohlcv(*d, base + i as f64))
                .collect();
            let funds: Vec<_> = days
                .iter()
                .map(|d| frow(*d, pe, if *d == event_day { 1.0 } else { 0.0 }))
                .collect();
            write_symbol(&dir, sym, &prices, &funds);
        }

        let source = LocalSource::new(&dir);
        // Hold the lowest-pe name only on its report-event day — references pe AND
        // report_event. The point: report_event resolves (no "unknown series" error).
        let req = BacktestRequest {
            spec: serde_json::json!({
                "op": "And",
                "l": { "op": "IsSmallest", "of": { "op": "Data", "name": "pe" }, "n": 1 },
                "r": { "op": "Data", "name": "report_event" }
            }),
            symbols: vec!["RPTA".into(), "RPTB".into()],
            from: 20240102,
            to: 20240104,
            fee_ratio: 0.0,
            position_limit: 0.0,
            price_key: "close".into(),
            ..Default::default()
        };

        let report = handle_backtest(&source, &req, &DataDirs::default()).unwrap();
        assert_eq!(report.dates, vec![20240102, 20240103, 20240104]);
    }

    #[test]
    fn handle_rebuild_builds_then_backtest_uses_them() {
        let dir = fixture("yuzu_server_handle_rebuild_test");
        std::fs::create_dir_all(dir.join("panels")).unwrap();
        let days = [20240102, 20240103];
        for (sym, base, pe) in [("RBA", 10.0, 8.0), ("RBB", 20.0, 15.0)] {
            let prices: Vec<_> = days
                .iter()
                .enumerate()
                .map(|(i, d)| ohlcv(*d, base + i as f64))
                .collect();
            let funds: Vec<_> = days.iter().map(|d| frow(*d, pe, 0.0)).collect();
            write_symbol(&dir, sym, &prices, &funds);
        }
        let source = LocalSource::new(&dir);
        let req = RebuildRequest {
            symbols: vec!["RBA".into(), "RBB".into()],
        };
        let summary = handle_rebuild(&source, &req, &DataDirs::default()).unwrap();
        assert!(summary.fields >= 6); // 5 OHLCV + fundamentals
                                      // the combined close panel now exists and loads
        assert!(source.get("panels/close.csv.gz").unwrap().is_some());
    }

    #[test]
    fn runs_backtest_from_combined_panels() {
        let dir = fixture("yuzu_server_combined_test");
        std::fs::create_dir_all(dir.join("panels")).unwrap();
        let days = [20240102, 20240103, 20240104];
        for (sym, base, pe) in [("CMBA", 10.0, 8.0), ("CMBB", 20.0, 15.0)] {
            let prices: Vec<_> = days
                .iter()
                .enumerate()
                .map(|(i, d)| ohlcv(*d, base + i as f64))
                .collect();
            let funds: Vec<_> = days.iter().map(|d| frow(*d, pe, 0.0)).collect();
            write_symbol(&dir, sym, &prices, &funds);
        }
        let source = LocalSource::new(&dir);
        let syms = vec!["CMBA".to_string(), "CMBB".to_string()];
        // build the combined panels the loader will now prefer
        yuzu_data::rebuild_combined_panels(&source, &syms, "prices", "fundamentals", "panels")
            .unwrap();
        // delete per-symbol sources so only the combined files remain — forces the
        // combined-first path (fallback now has nothing to load).
        std::fs::remove_dir_all(dir.join("prices")).unwrap();
        std::fs::remove_dir_all(dir.join("fundamentals")).unwrap();

        let req = BacktestRequest {
            spec: serde_json::json!({ "op": "IsSmallest", "of": { "op": "Data", "name": "pe" }, "n": 1 }),
            symbols: syms,
            from: 20240102,
            to: 20240104,
            fee_ratio: 0.0,
            position_limit: 0.0,
            price_key: "close".into(),
            ..Default::default()
        };
        let report = handle_backtest(&source, &req, &DataDirs::default()).unwrap();
        assert!(
            !report.equity.is_empty(),
            "equity curve should be non-empty"
        );
    }

    #[test]
    fn loads_the_volume_panel_when_the_liquidity_cap_is_active() {
        let dir = fixture("yuzu_server_volume_test");
        let days = [20240102, 20240103, 20240104];
        for (sym, base, pe) in [("VOLA", 10.0, 8.0), ("VOLB", 20.0, 15.0)] {
            let prices: Vec<_> = days
                .iter()
                .enumerate()
                .map(|(i, d)| ohlcv(*d, base + i as f64))
                .collect();
            let funds: Vec<_> = days.iter().map(|d| frow(*d, pe, 0.0)).collect();
            write_symbol(&dir, sym, &prices, &funds);
        }
        let source = LocalSource::new(&dir);
        // initial_capital + max_participation both > 0 → the volume series is added
        // to the load set and the cap is applied.
        let req = BacktestRequest {
            spec: serde_json::json!({ "op": "IsSmallest", "of": { "op": "Data", "name": "pe" }, "n": 1 }),
            symbols: vec!["VOLA".into(), "VOLB".into()],
            from: 20240102,
            to: 20240104,
            price_key: "close".into(),
            initial_capital: 1_000_000.0,
            max_participation: 0.05,
            ..Default::default()
        };
        let report = handle_backtest(&source, &req, &DataDirs::default()).unwrap();
        assert!(!report.equity.is_empty());
    }

    #[test]
    fn collect_series_walks_objects_and_arrays() {
        // A spec value that nests Data nodes inside a JSON array exercises the
        // Array arm of the walker; duplicates collapse into the set.
        let spec = serde_json::json!({
            "op": "SomeListOp",
            "of": [
                { "op": "Data", "name": "close" },
                { "op": "Data", "name": "pe" },
                { "op": "Data", "name": "close" }
            ]
        });
        let mut names = BTreeSet::new();
        collect_series(&spec, &mut names);
        assert_eq!(
            names,
            ["close", "pe"].iter().map(|s| s.to_string()).collect()
        );
    }

    #[test]
    fn backtest_request_defaults_price_key_to_close() {
        // A request JSON without `price_key` deserializes with the serde default.
        let req: BacktestRequest = serde_json::from_str(
            r#"{"spec":{"op":"Data","name":"close"},"symbols":[],"from":0,"to":0}"#,
        )
        .unwrap();
        assert_eq!(req.price_key, "close");
    }

    #[test]
    fn data_dirs_from_env_overrides_then_defaults() {
        // These YUZU_* vars are read by no other test in this crate.
        for k in [
            "YUZU_PRICES_DIR",
            "YUZU_FUNDAMENTALS_DIR",
            "YUZU_PANELS_DIR",
        ] {
            std::env::remove_var(k);
        }
        // Unset → the constants.
        let d = DataDirs::from_env();
        assert_eq!(d.prices, PRICES_DIR);
        assert_eq!(d.fundamentals, FUNDAMENTALS_DIR);
        assert_eq!(d.panels, PANELS_DIR);

        // Set → the override wins.
        std::env::set_var("YUZU_PRICES_DIR", "custom/prices");
        std::env::set_var("YUZU_FUNDAMENTALS_DIR", "custom/funds");
        std::env::set_var("YUZU_PANELS_DIR", "custom/panels");
        let d = DataDirs::from_env();
        assert_eq!(d.prices, "custom/prices");
        assert_eq!(d.fundamentals, "custom/funds");
        assert_eq!(d.panels, "custom/panels");

        for k in [
            "YUZU_PRICES_DIR",
            "YUZU_FUNDAMENTALS_DIR",
            "YUZU_PANELS_DIR",
        ] {
            std::env::remove_var(k);
        }
    }
}
