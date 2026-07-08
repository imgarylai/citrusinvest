mod golden_harness;
use yuzu_core::backtest::{run, BacktestConfig};
use yuzu_core::panel::Panel;
use golden_harness::{load_golden, panel_from_json};

fn cfg(v: &serde_json::Value) -> BacktestConfig {
    BacktestConfig {
        fee_ratio: v["fee_ratio"].as_f64().unwrap(),
        tax_ratio: v["tax_ratio"].as_f64().unwrap(),
        position_limit: 0.0,
    }
}

fn check(name: &str) {
    let v = load_golden(name);
    let pos: Panel = panel_from_json(&v, "positions");
    let px: Panel = panel_from_json(&v, "prices");
    let r = run(&pos, &px, None, None, &cfg(&v));
    let want = v["equity"].as_array().unwrap();
    assert_eq!(r.equity.len(), want.len());
    for (i, w) in want.iter().enumerate() {
        assert!((r.equity[i] - w.as_f64().unwrap()).abs() < 1e-9, "equity[{i}]");
    }
}

#[test]
fn nav_matches_reference_no_fee() {
    check("backtest_nofee");
}

#[test]
fn nav_matches_reference_with_fee() {
    check("backtest_fee");
}

#[test]
fn trades_recorded() {
    // AAA: held days 0-1 then exits day 2; re-enters day 4 (still open at end).
    let v = load_golden("backtest_nofee");
    let pos: Panel = panel_from_json(&v, "positions");
    let px: Panel = panel_from_json(&v, "prices");
    let r = run(&pos, &px, None, None, &BacktestConfig::default());
    let aaa: Vec<_> = r.trades.iter().filter(|t| t.symbol == "AAA").collect();
    assert_eq!(aaa.len(), 2);
    assert_eq!(aaa[0].entry_date, 20240102);
    assert_eq!(aaa[0].exit_date, Some(20240104));
    assert_eq!(aaa[1].exit_date, None); // re-entered day 4, never closed
    assert_eq!(aaa[1].entry_date, 20240108); // re-entry day 4
    // first trade closed AAA 10 -> 12 with zero fees: ret = 0.2
    assert!((aaa[0].ret - 0.2).abs() < 1e-9);
    assert_eq!(aaa[0].period, 2);
}
