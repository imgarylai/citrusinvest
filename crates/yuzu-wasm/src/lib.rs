//! JSON boundary over `yuzu-core`'s backtest. `run_backtest_json` is pure Rust
//! (unit-tested here); the `wasm_bindgen` export in this file re-exposes it to the
//! Worker. Input JSON: { spec, panels:{name:{dates,symbols,data}}, industry?,
//! price_key, config? }. `data` cells may be null (→ NaN). Output: Report JSON.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// WASM entry point — thin wrapper over `run_backtest_json`. Errors surface as a
/// thrown JS string.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn run_backtest(input_json: &str) -> Result<String, JsValue> {
    run_backtest_json(input_json).map_err(|e| JsValue::from_str(&e))
}

/// WASM entry point for the factor report — thin wrapper over `run_factor_json`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn run_factor(input_json: &str) -> Result<String, JsValue> {
    run_factor_json(input_json).map_err(|e| JsValue::from_str(&e))
}

/// WASM entry point for the event study — thin wrapper over `run_event_json`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn run_event(input_json: &str) -> Result<String, JsValue> {
    run_event_json(input_json).map_err(|e| JsValue::from_str(&e))
}

use ndarray::Array2;
use serde::Deserialize;
use std::collections::HashMap;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::panel::Panel;
use yuzu_core::{run_backtest as run_backtest_core, run_strategy, EvalContext};

#[derive(Deserialize)]
struct PanelJson {
    dates: Vec<i32>,
    symbols: Vec<String>,
    data: Vec<Vec<Option<f64>>>,
}

#[derive(Deserialize, Default)]
struct ConfigJson {
    #[serde(default)]
    fee_ratio: f64,
    #[serde(default)]
    tax_ratio: f64,
    #[serde(default)]
    position_limit: f64,
    #[serde(default)]
    slippage_ratio: f64,
    #[serde(default)]
    initial_capital: f64,
    #[serde(default)]
    max_participation: f64,
    #[serde(default)]
    impact_coef: f64,
    #[serde(default)]
    delist_after: usize,
    #[serde(default)]
    delist_haircut: f64,
    #[serde(default)]
    benchmark_key: Option<String>,
    #[serde(default)]
    bootstrap_samples: usize,
    #[serde(default)]
    bootstrap_block: usize,
    #[serde(default)]
    live_performance_start: Option<i32>,
    #[serde(default)]
    stops: StopConfigJson,
}

/// Execution-layer stop knobs. Optional levels (`null`/absent = off); `fill` is
/// `"touched"` (default) or `"close"`. Touched fills need `open`/`high`/`low`
/// panels in the request.
#[derive(Deserialize, Default)]
struct StopConfigJson {
    #[serde(default)]
    stop_loss: Option<f64>,
    #[serde(default)]
    take_profit: Option<f64>,
    #[serde(default)]
    trail_stop: Option<f64>,
    #[serde(default)]
    trail_stop_activation: f64,
    #[serde(default)]
    fill: StopFillJson,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum StopFillJson {
    #[default]
    Touched,
    Close,
}

impl From<StopConfigJson> for yuzu_core::backtest::StopConfig {
    fn from(s: StopConfigJson) -> Self {
        use yuzu_core::backtest::StopFill;
        let fill = match s.fill {
            StopFillJson::Touched => StopFill::Touched,
            StopFillJson::Close => StopFill::Close,
        };
        yuzu_core::backtest::StopConfig::from_options(
            s.stop_loss,
            s.take_profit,
            s.trail_stop,
            s.trail_stop_activation,
            fill,
        )
    }
}

#[derive(Deserialize)]
struct Input {
    spec: serde_json::Value,
    panels: HashMap<String, PanelJson>,
    #[serde(default)]
    industry: HashMap<String, String>,
    price_key: String,
    #[serde(default)]
    config: ConfigJson,
}

fn panel_from_json(p: PanelJson) -> Result<Panel, String> {
    let (nrows, ncols) = (p.dates.len(), p.symbols.len());
    let mut data = Array2::from_elem((nrows, ncols), f64::NAN);
    for (r, row) in p.data.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(x) = cell {
                data[[r, c]] = *x;
            }
        }
    }
    Panel::new(p.dates, p.symbols, data).map_err(|e| e.to_string())
}

pub fn run_backtest_json(input_json: &str) -> Result<String, String> {
    let input: Input = serde_json::from_str(input_json).map_err(|e| e.to_string())?;
    let mut panels = HashMap::new();
    for (name, p) in input.panels {
        panels.insert(name, panel_from_json(p)?);
    }
    let ctx = EvalContext {
        panels,
        industry: input.industry,
    };
    let spec_str = serde_json::to_string(&input.spec).map_err(|e| e.to_string())?;
    let cfg = BacktestConfig {
        fee_ratio: input.config.fee_ratio,
        tax_ratio: input.config.tax_ratio,
        position_limit: input.config.position_limit,
        slippage_ratio: input.config.slippage_ratio,
        initial_capital: input.config.initial_capital,
        max_participation: input.config.max_participation,
        impact_coef: input.config.impact_coef,
        delist_after: input.config.delist_after,
        delist_haircut: input.config.delist_haircut,
        benchmark_key: input.config.benchmark_key,
        bootstrap_samples: input.config.bootstrap_samples,
        bootstrap_block: input.config.bootstrap_block,
        live_performance_start: input.config.live_performance_start,
        stops: input.config.stops.into(),
    };
    let report =
        run_backtest_core(&spec_str, &ctx, &input.price_key, &cfg).map_err(|e| e.to_string())?;
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

/// Build an `EvalContext` from the request's `panels` + `industry`.
fn ctx_from(
    panels_json: HashMap<String, PanelJson>,
    industry: HashMap<String, String>,
) -> Result<EvalContext, String> {
    let mut panels = HashMap::new();
    for (name, p) in panels_json {
        panels.insert(name, panel_from_json(p)?);
    }
    Ok(EvalContext { panels, industry })
}

fn default_horizon() -> usize {
    1
}
fn default_quantiles() -> usize {
    5
}
fn default_window() -> usize {
    5
}

#[derive(Deserialize)]
struct FactorInput {
    spec: serde_json::Value,
    panels: HashMap<String, PanelJson>,
    #[serde(default)]
    industry: HashMap<String, String>,
    /// Forward-return horizon in trading days (default 1).
    #[serde(default = "default_horizon")]
    horizon: usize,
    /// Number of factor quantile buckets (default 5).
    #[serde(default = "default_quantiles")]
    quantiles: usize,
    /// Demean the factor within each sector before ranking (default false).
    #[serde(default)]
    neutralize_industry: bool,
}

/// Factor report (#45) at the WASM boundary. Input: `{ spec, panels, industry?,
/// horizon?, quantiles?, neutralize_industry? }`; the factor comes from the spec,
/// forward returns from the `close` panel. Output: `FactorReport` JSON.
pub fn run_factor_json(input_json: &str) -> Result<String, String> {
    let input: FactorInput = serde_json::from_str(input_json).map_err(|e| e.to_string())?;
    let ctx = ctx_from(input.panels, input.industry)?;
    let spec_str = serde_json::to_string(&input.spec).map_err(|e| e.to_string())?;
    let mut factor = run_strategy(&spec_str, &ctx).map_err(|e| e.to_string())?;
    if input.neutralize_industry {
        factor = factor.neutralize_industry(&ctx.industry, true);
    }
    let close = ctx.panels.get("close").ok_or("no close panel")?;
    let fwd = yuzu_core::research::forward_returns(close, input.horizon);
    let report = yuzu_core::research::factor_report(&factor, &fwd, input.quantiles);
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct EventInput {
    spec: serde_json::Value,
    panels: HashMap<String, PanelJson>,
    #[serde(default)]
    industry: HashMap<String, String>,
    /// Rows before each event to include (default 5).
    #[serde(default = "default_window")]
    pre: usize,
    /// Rows after each event to include (default 5).
    #[serde(default = "default_window")]
    post: usize,
}

/// Event study (#45) at the WASM boundary. Input: `{ spec, panels, pre?, post? }`;
/// the 0/1 event panel comes from the spec, daily returns from the `close` panel.
/// Output: `EventStudy` JSON.
pub fn run_event_json(input_json: &str) -> Result<String, String> {
    let input: EventInput = serde_json::from_str(input_json).map_err(|e| e.to_string())?;
    let ctx = ctx_from(input.panels, input.industry)?;
    let spec_str = serde_json::to_string(&input.spec).map_err(|e| e.to_string())?;
    let events = run_strategy(&spec_str, &ctx).map_err(|e| e.to_string())?;
    let close = ctx.panels.get("close").ok_or("no close panel")?;
    let rets = yuzu_core::research::daily_returns(close);
    let es = yuzu_core::research::event_study(&events, &rets, input.pre, input.post);
    serde_json::to_string(&es).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn factor_report_from_json() {
        // 4 symbols, 2 dates; factor = close level. Forward returns (close[t+1]/
        // close[t] − 1) rise with the factor: 0.1, 0.2, 0.3, 0.4 → rank IC +1,
        // positive long-short spread. Only row 0 has a defined forward return.
        let input = r#"{
            "spec": { "op": "Data", "name": "close" },
            "panels": {
                "close": { "dates": [20240102,20240103], "symbols": ["A","B","C","D"],
                           "data": [[1.0,2.0,3.0,4.0],[1.1,2.4,3.9,5.6]] }
            },
            "horizon": 1, "quantiles": 2
        }"#;
        let out: Value = serde_json::from_str(&run_factor_json(input).unwrap()).unwrap();
        assert_eq!(out["quantiles"], 2);
        assert!((out["mean_ic"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert!(out["long_short"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn event_study_from_json() {
        // Event for A on date 3; average the daily-return path over [-1, +1].
        let input = r#"{
            "spec": { "op": "Data", "name": "signal" },
            "panels": {
                "signal": { "dates": [20240102,20240103,20240104,20240105], "symbols": ["A"],
                            "data": [[0.0],[0.0],[1.0],[0.0]] },
                "close":  { "dates": [20240102,20240103,20240104,20240105], "symbols": ["A"],
                            "data": [[100.0],[110.0],[121.0],[121.0]] }
            },
            "pre": 1, "post": 1
        }"#;
        let out: Value = serde_json::from_str(&run_event_json(input).unwrap()).unwrap();
        assert_eq!(out["event_count"], 1);
        assert_eq!(out["lags"].as_array().unwrap().len(), 3);
        // lag 0 = the +10% move into the event day (121/110 - 1 = 0.1).
        let avg = out["avg_return"].as_array().unwrap();
        assert!((avg[1].as_f64().unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn runs_a_buy_and_hold_backtest_from_json() {
        // One symbol, 3 days, always-in position; close 10→11→12 ⇒ +20% total.
        let input = r#"{
            "spec": { "op": "Data", "name": "signal" },
            "panels": {
                "signal": { "dates": [20240102,20240103,20240104], "symbols": ["A"], "data": [[1.0],[1.0],[1.0]] },
                "close":  { "dates": [20240102,20240103,20240104], "symbols": ["A"], "data": [[10.0],[11.0],[12.0]] }
            },
            "price_key": "close",
            "config": { "fee_ratio": 0.0, "tax_ratio": 0.0 }
        }"#;
        let out = run_backtest_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["equity"].as_array().unwrap().len(), 3);
        let total = v["metrics"]["total_return"].as_f64().unwrap();
        assert!((total - 0.2).abs() < 1e-9, "total_return {total}");
    }

    #[test]
    fn stops_flow_through_the_config() {
        // Entry 100; day1 low 90 touches the 8% stop (level 92), open 98 above →
        // touched fill at 92 → −8% (vs close-to-close −5%).
        let input = r#"{
            "spec": { "op": "Data", "name": "signal" },
            "panels": {
                "signal": { "dates": [20240102,20240103], "symbols": ["A"], "data": [[1.0],[1.0]] },
                "close":  { "dates": [20240102,20240103], "symbols": ["A"], "data": [[100.0],[95.0]] },
                "open":   { "dates": [20240102,20240103], "symbols": ["A"], "data": [[100.0],[98.0]] },
                "high":   { "dates": [20240102,20240103], "symbols": ["A"], "data": [[100.0],[99.0]] },
                "low":    { "dates": [20240102,20240103], "symbols": ["A"], "data": [[100.0],[90.0]] }
            },
            "price_key": "close",
            "config": { "stops": { "stop_loss": 0.08 } }
        }"#;
        let out = run_backtest_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let total = v["metrics"]["total_return"].as_f64().unwrap();
        assert!((total - (-0.08)).abs() < 1e-9, "stop fill at 92: {total}");
        assert!((v["trades"][0]["exit_price"].as_f64().unwrap() - 92.0).abs() < 1e-9);
    }

    #[test]
    fn new_config_fields_flow_through_to_the_report() {
        // slippage shows up in equity; benchmark_key adds the benchmark block.
        let input = r#"{
            "spec": { "op": "Data", "name": "signal" },
            "panels": {
                "signal": { "dates": [20240102,20240103,20240104], "symbols": ["A"], "data": [[1.0],[1.0],[1.0]] },
                "close":  { "dates": [20240102,20240103,20240104], "symbols": ["A"], "data": [[10.0],[11.0],[12.0]] },
                "spy":    { "dates": [20240102,20240103,20240104], "symbols": ["SPY"], "data": [[100.0],[101.0],[102.0]] }
            },
            "price_key": "close",
            "config": { "slippage_ratio": 0.001, "benchmark_key": "spy" }
        }"#;
        let out = run_backtest_json(input).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["benchmark"].as_array().unwrap().len(), 3);
        assert!(v["metrics"]["alpha"].is_number());
        assert!(v["metrics"]["information_ratio"].is_number());
        // day-0 entry pays slippage on turnover 1.0
        let eq0 = v["equity"][0].as_f64().unwrap();
        assert!((eq0 - 0.999).abs() < 1e-12, "slippage applied: {eq0}");
    }

    #[test]
    fn null_cells_become_nan_and_bad_input_errors() {
        assert!(run_backtest_json("{ not json").is_err());
        // null in a panel cell parses (becomes NaN) — unknown price key errors.
        let input = r#"{"spec":{"op":"Data","name":"x"},
            "panels":{"x":{"dates":[20240102],"symbols":["A"],"data":[[null]]}},
            "price_key":"missing"}"#;
        assert!(run_backtest_json(input).is_err());
    }
}
