mod golden_harness;
use golden_harness::{assert_panel_eq, load_golden, panel_from_json};

use yuzu_core::ops::rotation::HoldUntilOpts;
use yuzu_core::panel::Panel;

fn input(name: &str) -> Panel {
    panel_from_json(&load_golden(name), "input")
}
fn expected(name: &str) -> Panel {
    panel_from_json(&load_golden(name), "expected")
}

#[test]
fn average_2_matches_reference() {
    assert_panel_eq(&input("average_2").average(2), &expected("average_2"), 1e-9);
}

#[test]
fn rise_1_matches_reference() {
    assert_panel_eq(&input("rise_1").rise(1), &expected("rise_1"), 0.0);
}

#[test]
fn fall_1_matches_reference() {
    assert_panel_eq(&input("fall_1").fall(1), &expected("fall_1"), 0.0);
}

#[test]
fn is_largest_1_matches_reference() {
    assert_panel_eq(
        &input("is_largest_1").is_largest(1),
        &expected("is_largest_1"),
        0.0,
    );
}

#[test]
fn is_smallest_2_matches_reference() {
    assert_panel_eq(
        &input("is_smallest_2").is_smallest(2),
        &expected("is_smallest_2"),
        0.0,
    );
}

#[test]
fn sustain_2_matches_reference() {
    // fixture: expected = RAW.rise(1).sustain(2); reproduce from the RAW input.
    let inp = input("sustain_2");
    assert_panel_eq(&inp.rise(1).sustain(2, None), &expected("sustain_2"), 0.0);
}

#[test]
fn is_entry_matches_reference() {
    let inp = input("is_entry");
    let cond = inp.gt(&inp.average(2));
    assert_panel_eq(&cond.is_entry(), &expected("is_entry"), 0.0);
}

#[test]
fn is_exit_matches_reference() {
    let inp = input("is_exit");
    let cond = inp.gt(&inp.average(2));
    assert_panel_eq(&cond.is_exit(), &expected("is_exit"), 0.0);
}

#[test]
fn hold_until_n1_matches_reference() {
    let inp = input("hold_until_n1");
    let entries = inp.gt(&inp.average(2));
    let exits = inp.lt(&inp.average(2));
    let opts = HoldUntilOpts {
        nstocks_limit: Some(1),
        ..Default::default()
    };
    assert_panel_eq(
        &entries.hold_until(&exits, &opts),
        &expected("hold_until_n1"),
        0.0,
    );
}

#[test]
fn rebalance_w_matches_reference() {
    use yuzu_core::ops::rebalance::Freq;
    assert_panel_eq(
        &input("rebalance_W").rebalance_freq(Freq::Weekly),
        &expected("rebalance_W"),
        1e-9,
    );
}

#[test]
fn rebalance_me_matches_reference() {
    use yuzu_core::ops::rebalance::Freq;
    assert_panel_eq(
        &input("rebalance_ME").rebalance_freq(Freq::MonthEnd),
        &expected("rebalance_ME"),
        1e-9,
    );
}

#[test]
fn rebalance_qe_matches_reference() {
    use yuzu_core::ops::rebalance::Freq;
    assert_panel_eq(
        &input("rebalance_QE").rebalance_freq(Freq::QuarterEnd),
        &expected("rebalance_QE"),
        1e-9,
    );
}

#[test]
fn quantile_50_matches_reference() {
    assert_panel_eq(
        &input("quantile_50").quantile_row(0.5),
        &expected("quantile_50"),
        1e-9,
    );
}

#[test]
fn rolling_min_3_matches_reference() {
    assert_panel_eq(
        &input("rolling_min_3").rolling_min(3),
        &expected("rolling_min_3"),
        1e-9,
    );
}

#[test]
fn donchian_bands_match_reference() {
    assert_panel_eq(
        &input("donchian_high_3").donchian_high(3),
        &expected("donchian_high_3"),
        1e-9,
    );
    assert_panel_eq(
        &input("donchian_low_3").donchian_low(3),
        &expected("donchian_low_3"),
        1e-9,
    );
    assert_panel_eq(
        &input("donchian_mid_3").donchian_mid(3),
        &expected("donchian_mid_3"),
        1e-9,
    );
}

#[test]
fn bollinger_bands_match_reference() {
    assert_panel_eq(
        &input("bollinger_mid_2").bollinger_mid(2),
        &expected("bollinger_mid_2"),
        1e-9,
    );
    assert_panel_eq(
        &input("bollinger_upper_2").bollinger_upper(2, 2.0),
        &expected("bollinger_upper_2"),
        1e-9,
    );
    assert_panel_eq(
        &input("bollinger_lower_2").bollinger_lower(2, 2.0),
        &expected("bollinger_lower_2"),
        1e-9,
    );
}

#[test]
fn macd_line_signal_hist_match_reference() {
    assert_panel_eq(&input("macd_2_3").macd(2, 3), &expected("macd_2_3"), 1e-9);
    assert_panel_eq(
        &input("macd_signal_2_3_2").macd_signal(2, 3, 2),
        &expected("macd_signal_2_3_2"),
        1e-9,
    );
    assert_panel_eq(
        &input("macd_hist_2_3_2").macd_hist(2, 3, 2),
        &expected("macd_hist_2_3_2"),
        1e-9,
    );
}

#[test]
fn exit_when_matches_reference() {
    let inp = input("exit_when");
    let cond = inp.gt(&inp.average(2));
    let exit = inp.lt(&inp.average(2));
    assert_panel_eq(&cond.exit_when(&exit), &expected("exit_when"), 0.0);
}

// Price stops moved out of `hold_until` into the execution layer
// (`BacktestConfig::stops`); their behavior is covered by the stop tests in
// `backtest.rs` (touched/gap/close/trailing/short/re-entry). The old
// op-level `hold_until_stops` / `hold_until_trail` goldens are retired.

#[test]
fn vol_target_matches_reference() {
    let v = load_golden("vol_target_2");
    let weights = panel_from_json(&v, "weights");
    let prices = panel_from_json(&v, "prices");
    let target = v["target"].as_f64().unwrap();
    let n = v["n"].as_u64().unwrap() as usize;
    assert_panel_eq(
        &weights.vol_target(&prices, target, n),
        &panel_from_json(&v, "expected"),
        1e-12,
    );
}
