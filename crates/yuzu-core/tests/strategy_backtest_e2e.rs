mod golden_harness;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::panel::Panel;
use yuzu_core::run_backtest;
use yuzu_core::EvalContext;
use golden_harness::{load_golden, panel_from_json};
use std::collections::HashMap;

#[test]
fn spec_to_report_end_to_end() {
    // reuse the DSL e2e fixture's close prices as both signal source and mark price.
    let v = load_golden("strategy_e2e");
    let close: Panel = panel_from_json(&v, "input");
    let mut ctx_panels = HashMap::new();
    ctx_panels.insert("close".to_string(), close);
    let ctx = EvalContext::new(ctx_panels);

    let spec = r#"{ "op": "Gt",
        "l": { "op": "Data", "name": "close" },
        "r": { "op": "Average", "of": { "op": "Data", "name": "close" }, "n": 2 } }"#;

    let report = run_backtest(spec, &ctx, "close", &BacktestConfig::default()).unwrap();
    assert!(!report.equity.is_empty());
    assert_eq!(report.equity.len(), report.dates.len());
    assert!(report.metrics.sharpe.is_finite() || report.metrics.sharpe.is_nan());
}
