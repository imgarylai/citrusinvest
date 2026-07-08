mod golden_harness;
use golden_harness::{assert_panel_eq, load_golden, panel_from_json};
use std::collections::HashMap;
use yuzu_core::panel::Panel;
use yuzu_core::run_strategy;
use yuzu_core::EvalContext;

#[test]
fn ma_crossover_hold_until_matches_reference() {
    let v = load_golden("strategy_e2e");
    let close: Panel = panel_from_json(&v, "input");
    let mut ctx_panels = HashMap::new();
    ctx_panels.insert("close".to_string(), close);
    let ctx = EvalContext::new(ctx_panels);

    let spec = r#"
    {
      "op": "HoldUntil",
      "entry": { "op": "Gt",
        "l": { "op": "Data", "name": "close" },
        "r": { "op": "Average", "of": { "op": "Data", "name": "close" }, "n": 2 } },
      "exit": { "op": "Lt",
        "l": { "op": "Data", "name": "close" },
        "r": { "op": "Average", "of": { "op": "Data", "name": "close" }, "n": 2 } },
      "nstocks_limit": 1,
      "rank": null
    }"#;

    let got = run_strategy(spec, &ctx).unwrap();
    assert_panel_eq(&got, &panel_from_json(&v, "expected"), 0.0);
}
