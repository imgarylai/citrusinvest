//! Unit tests for the backtest module tree.

use super::*;
use crate::panel::Panel;
use std::collections::HashMap;

#[test]
fn cap_weights_clamps_each_to_limit_leaving_cash() {
    let mut a = [0.5, 0.5];
    cap_weights_row(&mut a, 0.3);
    assert_eq!(a, [0.3, 0.3]); // each capped; sum 0.6, rest cash
    let mut b = [0.2, 0.2];
    cap_weights_row(&mut b, 0.3);
    assert_eq!(b, [0.2, 0.2]); // under cap, unchanged
    let mut c = [0.5];
    cap_weights_row(&mut c, 0.0);
    assert_eq!(c, [0.5]); // 0 = off
}

#[test]
fn normalize_caps_at_one_but_leaves_small_books() {
    let mut a = [0.5, 0.5, 0.5]; // sum 1.5 -> divide by 1.5
    normalize_weights_row(&mut a);
    assert!((a[0] - 1.0 / 3.0).abs() < 1e-12);
    let mut b = [0.2, 0.3]; // sum 0.5 -> total clamped to 1.0 -> unchanged
    normalize_weights_row(&mut b);
    assert_eq!(b, [0.2, 0.3]);
}

#[test]
fn single_asset_full_weight_tracks_price() {
    use crate::panel::Panel;
    // 1 asset, weight 1.0 every day, no fees -> equity tracks price ratio.
    let pos = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![1.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![10.0], vec![11.0], vec![12.0]],
    )
    .unwrap();
    let run = run(&pos, &px, None, None, None, &BacktestConfig::default());
    assert_eq!(run.equity.len(), 3);
    assert!((run.equity[0] - 1.0).abs() < 1e-12);
    assert!((run.equity[1] - 1.1).abs() < 1e-12); // +10%
    assert!((run.equity[2] - 1.2).abs() < 1e-12); // 11->12 = +9.09% on 1.1
}

#[test]
fn slippage_charges_turnover_like_a_fee() {
    use crate::panel::Panel;
    // Enter day 0, exit day 2: two turnover events of 1.0 each.
    let pos = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![0.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![10.0], vec![10.0], vec![10.0]],
    )
    .unwrap();
    let slip = BacktestConfig {
        slippage_ratio: 0.001,
        ..Default::default()
    };
    let r = run(&pos, &px, None, None, None, &slip);
    // Flat price: equity = (1 - 0.001) entering * (1 - 0.001) exiting.
    let want = (1.0 - 0.001) * (1.0 - 0.001);
    assert!(
        (r.equity[2] - want).abs() < 1e-12,
        "equity {} want {want}",
        r.equity[2]
    );
    // The closed trade's net return carries slippage on both legs.
    let t = &r.trades[0];
    let want_ret = (1.0 - 0.001) * 1.0 * (1.0 - 0.001) - 1.0;
    assert!((t.ret - want_ret).abs() < 1e-12, "trade ret {}", t.ret);
    // Identical run with slippage folded into fee_ratio matches exactly.
    let fee = BacktestConfig {
        fee_ratio: 0.001,
        ..Default::default()
    };
    let r2 = run(&pos, &px, None, None, None, &fee);
    assert_eq!(r.equity, r2.equity);
}

#[test]
fn impact_cost_criteria() {
    use crate::panel::Panel;
    let dates = vec![20240102, 20240103];
    let syms = vec!["LIQ".to_string(), "ILQ".to_string()];
    let pos = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1.0, 1.0]; 2]).unwrap();
    let px = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![10.0, 10.0]; 2]).unwrap();
    // dollar volume: LIQ = 10 * 1e9 = 1e10; ILQ = 10 * 100 = 1_000.
    let vol = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e9, 100.0]; 2]).unwrap();
    let cfg = |coef: f64| BacktestConfig {
        impact_coef: coef,
        initial_capital: 1_000_000.0,
        ..Default::default()
    };

    // Day-0 entry: each cell trades |Δw| = 0.5.
    // LIQ participation = 0.5 * 1e6 / 1e10 = 5e-5 (dimensionless — #1).
    // ILQ participation = 0.5 * 1e6 / 1e3 = 500 → capped at 1 (#4).
    let coef = 0.01;
    let r = run(&pos, &px, None, None, Some(&vol), &cfg(coef));
    let liq_impact = 0.5 * coef * (5e-5_f64).sqrt();
    let ilq_impact = 0.5 * coef * 1.0_f64; // capped participation
    let want = 1.0 - (liq_impact + ilq_impact);
    assert!(
        (r.equity[0] - want).abs() < 1e-15,
        "equity {} want {want}",
        r.equity[0]
    );
    // #2 monotonicity: the illiquid cell pays strictly more.
    assert!(ilq_impact > liq_impact);

    // #5/#6 zero coefficient reproduces the legacy path bit-for-bit.
    let off = run(&pos, &px, None, None, Some(&vol), &cfg(0.0));
    let legacy = run(
        &pos,
        &px,
        None,
        None,
        Some(&vol),
        &BacktestConfig::default(),
    );
    assert_eq!(off.equity, legacy.equity);

    // #8 linearity: with zero flat components, total cost is linear in coef.
    let r2 = run(&pos, &px, None, None, Some(&vol), &cfg(2.0 * coef));
    assert!(((1.0 - r2.equity[0]) - 2.0 * (1.0 - r.equity[0])).abs() < 1e-15);

    // #3 zero/NaN dollar volume: those cells contribute NO impact (flat
    // path only) and nothing non-finite reaches the total.
    for bad in [0.0, f64::NAN] {
        let vol_bad =
            Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e9, bad]; 2]).unwrap();
        let rb = run(&pos, &px, None, None, Some(&vol_bad), &cfg(coef));
        let want_liq_only = 1.0 - liq_impact;
        assert!(
            (rb.equity[0] - want_liq_only).abs() < 1e-15,
            "bad dv {bad}: equity {}",
            rb.equity[0]
        );
        assert!(rb.equity.iter().all(|e| e.is_finite()));
    }

    // No volume panel at all -> impact silently off.
    let rn = run(&pos, &px, None, None, None, &cfg(coef));
    assert_eq!(rn.equity, legacy.equity);
}

#[test]
fn impact_cost_is_sign_symmetric() {
    use crate::panel::Panel;
    // #7: a buy of |Δw| = 1 and a later sell of |Δw| = 1 on a flat price
    // with identical dollar volume cost the same.
    let dates = vec![20240102, 20240103, 20240104];
    let syms = vec!["A".to_string()];
    let pos = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![vec![1.0], vec![1.0], vec![0.0]],
    )
    .unwrap();
    let px = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![10.0]; 3]).unwrap();
    let vol = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e6]; 3]).unwrap();
    let cfg = BacktestConfig {
        impact_coef: 0.01,
        initial_capital: 1_000_000.0,
        ..Default::default()
    };
    let r = run(&pos, &px, None, None, Some(&vol), &cfg);
    let entry_cost = 1.0 - r.equity[0];
    let exit_cost = 1.0 - r.equity[2] / r.equity[1];
    assert!(entry_cost > 0.0);
    assert!(
        (entry_cost - exit_cost).abs() < 1e-15,
        "entry {entry_cost} vs exit {exit_cost}"
    );
}

#[test]
fn liquidity_cap_limits_weight_to_volume_participation() {
    use crate::panel::Panel;
    let dates = vec![20240102, 20240103, 20240104];
    let syms = vec!["A".to_string()];
    let pos = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![vec![1.0], vec![1.0], vec![1.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![vec![10.0], vec![10.0], vec![10.0]],
    )
    .unwrap();
    // Day-0 dollar volume = 10 * 1000 = 10_000. With capital 1_000_000 and
    // 5% participation, the cap is 10_000 * 0.05 / 1_000_000 = 0.0005.
    let vol = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![vec![1000.0], vec![1000.0], vec![1000.0]],
    )
    .unwrap();
    let cfg = BacktestConfig {
        initial_capital: 1_000_000.0,
        max_participation: 0.05,
        ..Default::default()
    };
    let r = run(&pos, &px, None, None, Some(&vol), &cfg);
    assert!((r.exposure[0] - 0.0005).abs() < 1e-12, "capped weight");

    // Cap off (defaults) or volume missing -> full weight.
    let r2 = run(
        &pos,
        &px,
        None,
        None,
        Some(&vol),
        &BacktestConfig::default(),
    );
    assert!((r2.exposure[0] - 1.0).abs() < 1e-12);
    let r3 = run(&pos, &px, None, None, None, &cfg);
    assert!((r3.exposure[0] - 1.0).abs() < 1e-12);

    // NaN volume day: weight passes through uncapped.
    let vol_nan = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![vec![f64::NAN], vec![1000.0], vec![1000.0]],
    )
    .unwrap();
    let r4 = run(&pos, &px, None, None, Some(&vol_nan), &cfg);
    assert!((r4.exposure[0] - 1.0).abs() < 1e-12, "NaN dv -> no cap");
}

#[test]
fn delisting_forces_exit_with_haircut() {
    use crate::panel::Panel;
    let dates = vec![20240102, 20240103, 20240104, 20240105, 20240108];
    let syms = vec!["A".to_string(), "B".to_string()];
    // Both held from day 0. B's prices vanish from day 2 on (delisted).
    let pos = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1.0, 1.0]; 5]).unwrap();
    let px = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![
            vec![10.0, 10.0],
            vec![10.0, 10.0],
            vec![10.0, f64::NAN],
            vec![10.0, f64::NAN],
            vec![10.0, f64::NAN],
        ],
    )
    .unwrap();

    // Legacy (delist_after = 0): B freezes at its last value, equity flat.
    let r0 = run(&pos, &px, None, None, None, &BacktestConfig::default());
    assert!((r0.equity[4] - 1.0).abs() < 1e-12, "legacy freezes");
    assert!(r0
        .trades
        .iter()
        .all(|t| t.symbol != "B" || t.exit_date.is_none()));

    // delist_after = 2 confirms on day 3 (rows 2,3 NaN). Full haircut:
    // B was half the book -> equity halves; B's trade is a -100% loss.
    let cfg = BacktestConfig {
        delist_after: 2,
        delist_haircut: 1.0,
        ..Default::default()
    };
    let r = run(&pos, &px, None, None, None, &cfg);
    assert!((r.equity[2] - 1.0).abs() < 1e-12, "before confirmation");
    assert!((r.equity[3] - 0.5).abs() < 1e-12, "haircut hits equity");
    assert!((r.equity[4] - 0.5).abs() < 1e-12);
    let b = r
        .trades
        .iter()
        .find(|t| t.symbol == "B" && t.exit_date.is_some())
        .unwrap();
    assert_eq!(b.exit_date, Some(20240105));
    assert!((b.ret - (-1.0)).abs() < 1e-12, "total loss, ret {}", b.ret);
    // Surviving symbol A is now the whole book.
    assert!((r.exposure[3] - 1.0).abs() < 1e-12);

    // Haircut 0: forced exit at the last valid price -> no equity impact,
    // B's trade closes flat (entered and exited at 10).
    let cfg0 = BacktestConfig {
        delist_after: 2,
        delist_haircut: 0.0,
        ..Default::default()
    };
    let r2 = run(&pos, &px, None, None, None, &cfg0);
    assert!((r2.equity[4] - 1.0).abs() < 1e-12);
    let b2 = r2
        .trades
        .iter()
        .find(|t| t.symbol == "B" && t.exit_date.is_some())
        .unwrap();
    assert!(b2.ret.abs() < 1e-12, "flat exit, ret {}", b2.ret);
}

#[test]
fn run_reports_per_day_gross_exposure() {
    use crate::panel::Panel;
    // 1 asset held every day at weight 1.0 -> exposure 1.0 each row.
    let pos = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![1.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        vec![20240102, 20240103, 20240104],
        vec!["A".into()],
        vec![vec![10.0], vec![11.0], vec![12.0]],
    )
    .unwrap();
    let run = run(&pos, &px, None, None, None, &BacktestConfig::default());
    assert_eq!(run.exposure.len(), 3);
    for e in &run.exposure {
        assert!((e - 1.0).abs() < 1e-12);
    }
}

#[test]
fn computes_direction_aware_mae_mfe() {
    use crate::panel::Panel;
    let dates = vec![20240102, 20240103, 20240104, 20240105];
    let syms = vec!["LONG".to_string(), "SHORT".to_string()];
    // LONG: held days 0-2, exits day 3 (closed). SHORT: held all days (open).
    let pos = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![
            vec![1.0, -1.0],
            vec![1.0, -1.0],
            vec![1.0, -1.0],
            vec![0.0, -1.0],
        ],
    )
    .unwrap();
    let close = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![
            vec![10.0, 10.0],
            vec![11.0, 9.0],
            vec![12.0, 8.0],
            vec![11.0, 9.0],
        ],
    )
    .unwrap();
    let high = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![
            vec![10.0, 10.0],
            vec![13.0, 11.0],
            vec![12.0, 12.0],
            vec![11.0, 9.0],
        ],
    )
    .unwrap();
    let low = Panel::from_rows(
        dates.clone(),
        syms.clone(),
        vec![
            vec![9.0, 10.0],
            vec![11.0, 8.0],
            vec![12.0, 7.0],
            vec![10.0, 9.0],
        ],
    )
    .unwrap();

    let r = run(
        &pos,
        &close,
        Some(&high),
        Some(&low),
        None,
        &BacktestConfig::default(),
    );
    let long = r.trades.iter().find(|t| t.symbol == "LONG").unwrap();
    let short = r.trades.iter().find(|t| t.symbol == "SHORT").unwrap();

    // LONG ep=10, dir=+1, window days 0..=3: MFE from high 13 → 0.3; MAE from low 9 → -0.1
    assert!((long.mfe.unwrap() - 0.3).abs() < 1e-9, "long mfe");
    assert!((long.mae.unwrap() - (-0.1)).abs() < 1e-9, "long mae");
    // SHORT ep=10, dir=-1, open, window days 0..=3: MFE from low 7 → 0.3; MAE from high 12 → -0.2
    assert!((short.mfe.unwrap() - 0.3).abs() < 1e-9, "short mfe");
    assert!((short.mae.unwrap() - (-0.2)).abs() < 1e-9, "short mae");

    // No high/low → None.
    let r2 = run(&pos, &close, None, None, None, &BacktestConfig::default());
    assert!(r2.trades.iter().all(|t| t.mae.is_none() && t.mfe.is_none()));

    // Fill prices and side come off the same panel cells that drive returns.
    assert_eq!(long.side, TradeSide::Long);
    assert!((long.entry_price - 10.0).abs() < 1e-12); // close on entry day
    assert!((long.exit_price.unwrap() - 11.0).abs() < 1e-12); // close on exit day
    assert_eq!(short.side, TradeSide::Short);
    assert!((short.entry_price - 10.0).abs() < 1e-12);
    assert!(short.exit_price.is_none()); // open trade: no realized exit
}

#[test]
fn delisting_exit_price_is_haircut_last_valid() {
    use crate::panel::Panel;
    // Held from day 0; price goes missing from day 2 on -> delisted after 1
    // missing row. Exit fills at the last valid price (20) less a 10% haircut.
    let dates = vec![20240102, 20240103, 20240104, 20240105];
    let pos = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![1.0], vec![1.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        vec![vec![10.0], vec![20.0], vec![f64::NAN], vec![f64::NAN]],
    )
    .unwrap();
    let cfg = BacktestConfig {
        delist_after: 1,
        delist_haircut: 0.1,
        ..Default::default()
    };
    let r = run(&pos, &px, None, None, None, &cfg);
    let t = &r.trades[0];
    assert_eq!(t.side, TradeSide::Long);
    assert!((t.entry_price - 10.0).abs() < 1e-12);
    assert!(t.exit_date.is_some(), "delisting force-closes the trade");
    assert!(
        (t.exit_price.unwrap() - 18.0).abs() < 1e-12,
        "exit fills at 20 * (1 - 0.1) = 18, got {:?}",
        t.exit_price
    );
}

#[test]
fn terminal_weights_report_the_final_book() {
    use crate::panel::Panel;
    // Hold A every day; end drifted book is A at weight 1.0 (single name).
    let dates = vec![20240102, 20240103];
    let pos =
        Panel::from_rows(dates.clone(), vec!["A".into()], vec![vec![1.0], vec![1.0]]).unwrap();
    let px = Panel::from_rows(dates, vec!["A".into()], vec![vec![10.0], vec![11.0]]).unwrap();
    let r = run(&pos, &px, None, None, None, &BacktestConfig::default());
    assert_eq!(r.terminal_weights.len(), 1);
    assert!((r.terminal_weights["A"] - 1.0).abs() < 1e-12);
}

#[test]
fn initial_weights_pay_seam_turnover_only_on_the_difference() {
    use crate::panel::Panel;
    // One name held every day, flat prices so nothing drifts; a 1% fee makes
    // the day-0 turnover visible in equity.
    let dates = vec![20240102, 20240103, 20240104];
    let pos = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![1.0]],
    )
    .unwrap();
    let px = Panel::from_rows(
        dates,
        vec!["A".into()],
        vec![vec![10.0], vec![10.0], vec![10.0]],
    )
    .unwrap();
    let cfg = BacktestConfig {
        fee_ratio: 0.01,
        ..Default::default()
    };

    // Flat start: day-0 buys the whole book, paying 1% on turnover 1.0.
    let flat = run(&pos, &px, None, None, None, &cfg);
    assert!((flat.equity[0] - 0.99).abs() < 1e-12);
    assert!((flat.terminal_weights["A"] - 1.0).abs() < 1e-12);

    // Carrying the identical book -> zero seam turnover, no day-0 cost.
    let carried = HashMap::from([("A".to_string(), 1.0)]);
    let warm = run_with_initial(&pos, &px, None, None, None, None, &cfg, Some(&carried));
    assert!((warm.equity[0] - 1.0).abs() < 1e-12, "no seam cost");
    assert!((warm.equity[2] - 1.0).abs() < 1e-12);

    // Carrying half the target -> turnover 0.5, so only half the entry fee.
    let half = HashMap::from([("A".to_string(), 0.5)]);
    let partial = run_with_initial(&pos, &px, None, None, None, None, &cfg, Some(&half));
    assert!((partial.equity[0] - (1.0 - 0.01 * 0.5)).abs() < 1e-12);

    // A carried symbol that isn't in this segment's target still costs to
    // unwind: hold B, carry A -> turnover |0-1| (sell A) + |1-0| (buy B) = 2.
    let posb = Panel::from_rows(
        vec![20240102, 20240103],
        vec!["A".into(), "B".into()],
        vec![vec![0.0, 1.0], vec![0.0, 1.0]],
    )
    .unwrap();
    let pxb = Panel::from_rows(
        vec![20240102, 20240103],
        vec!["A".into(), "B".into()],
        vec![vec![10.0, 10.0], vec![10.0, 10.0]],
    )
    .unwrap();
    let cross = run_with_initial(
        &posb,
        &pxb,
        None,
        None,
        None,
        None,
        &cfg,
        Some(&HashMap::from([("A".to_string(), 1.0)])),
    );
    assert!((cross.equity[0] - (1.0 - 0.01 * 2.0)).abs() < 1e-12);
}

// ---- execution-layer stops (#20) ---------------------------------------

/// Build (pos, close, open, high, low) panels from row-wise OHLC for one
/// symbol held long every day.
fn stop_fixture(
    ohlc: &[(f64, f64, f64, f64)], // (open, high, low, close)
) -> (Panel, Panel, Panel, Panel, Panel) {
    use crate::panel::Panel;
    let dates: Vec<i32> = (0..ohlc.len() as i32).map(|i| 20240102 + i).collect();
    let col = |f: fn(&(f64, f64, f64, f64)) -> f64| {
        Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            ohlc.iter().map(|x| vec![f(x)]).collect(),
        )
        .unwrap()
    };
    let pos = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        ohlc.iter().map(|_| vec![1.0]).collect(),
    )
    .unwrap();
    (pos, col(|x| x.3), col(|x| x.0), col(|x| x.1), col(|x| x.2))
}

#[test]
fn touched_stop_loss_fills_at_the_level_not_the_close() {
    // Entry close 100; day1 low 90 touches the 8% stop (level 92) while the
    // open (98) is above it -> fill at 92, not the close (95).
    let (pos, close, open, high, low) =
        stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 95.0)]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            stop_loss: 0.08,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    assert!(
        (r.equity[1] - 0.92).abs() < 1e-12,
        "fill at 92, got {}",
        r.equity[1]
    );
    let t = &r.trades[0];
    assert!((t.exit_price.unwrap() - 92.0).abs() < 1e-12);
}

#[test]
fn gapped_stop_fills_at_the_open() {
    // Day1 gaps fully below the 92 stop (open 88) -> can't fill at 92; fills
    // at the open 88 (worse than the stop).
    let (pos, close, open, high, low) =
        stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (88.0, 89.0, 87.0, 88.0)]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            stop_loss: 0.08,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    assert!(
        (r.equity[1] - 0.88).abs() < 1e-12,
        "gap fill at open 88, got {}",
        r.equity[1]
    );
}

#[test]
fn close_fill_mode_triggers_and_fills_on_the_close() {
    // Touched mode would fill at 92; Close mode instead needs the close to
    // breach and fills there. Close 91 -> −9% ≤ −8% -> fill at 91.
    let (pos, close, open, high, low) =
        stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 91.0)]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            stop_loss: 0.08,
            fill: StopFill::Close,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    assert!(
        (r.equity[1] - 0.91).abs() < 1e-12,
        "close fill at 91, got {}",
        r.equity[1]
    );
}

#[test]
fn take_profit_fills_at_the_level() {
    // Day1 high 115 hits the +10% take-profit (level 110); open 108 below it
    // -> fill at 110.
    let (pos, close, open, high, low) =
        stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (108.0, 115.0, 107.0, 112.0)]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            take_profit: 0.10,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    assert!(
        (r.equity[1] - 1.10).abs() < 1e-12,
        "take-profit at 110, got {}",
        r.equity[1]
    );
}

#[test]
fn stopped_name_stays_flat_until_the_signal_resets() {
    // Held days 0-1 (stops day1). Signal still on day2 → must stay flat
    // (the +200 spike must NOT be earned). Signal drops day3, re-asserts
    // day4 on a calm bar → a fresh trade opens.
    use crate::panel::Panel;
    let dates: Vec<i32> = (0..5).map(|i| 20240102 + i).collect();
    let pos = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        vec![vec![1.0], vec![1.0], vec![1.0], vec![0.0], vec![1.0]],
    )
    .unwrap();
    let mk = |v: Vec<f64>| {
        Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            v.into_iter().map(|x| vec![x]).collect(),
        )
        .unwrap()
    };
    let close = mk(vec![100.0, 95.0, 200.0, 100.0, 100.0]);
    let open = mk(vec![100.0, 98.0, 190.0, 100.0, 100.0]);
    let high = mk(vec![100.0, 99.0, 205.0, 100.0, 100.0]);
    let low = mk(vec![100.0, 90.0, 190.0, 100.0, 100.0]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            stop_loss: 0.08,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    // day1: 92/100 = 0.92; day2-4: flat (the +200 spike is NOT earned).
    assert!((r.equity[1] - 0.92).abs() < 1e-12);
    assert!(
        (r.equity[2] - 0.92).abs() < 1e-12,
        "must be flat day2, got {}",
        r.equity[2]
    );
    assert!((r.equity[4] - 0.92).abs() < 1e-12);
    // Two trades: the stopped exit (day1) and the fresh re-entry on day4.
    assert_eq!(r.trades.len(), 2, "expected a stopped exit + a re-entry");
    let reentry = r.trades.iter().find(|t| t.entry_date == 20240106).unwrap();
    assert!(reentry.exit_date.is_none(), "day4 re-entry is still open");
}

#[test]
fn trailing_stop_arms_and_ratchets() {
    // Rise to +20% (arms the 10% trail once activation 5% passed), then a
    // pullback whose low crosses trail level = peak(1.20) − 0.10 = 1.10 →
    // level price 110. Day2 low 108 < 110, open 118 above → fill at 110.
    let (pos, close, open, high, low) = stop_fixture(&[
        (100.0, 100.0, 100.0, 100.0),
        (105.0, 120.0, 104.0, 118.0), // peak 1.20
        (118.0, 118.0, 108.0, 109.0), // pulls back through 110
    ]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            trail_stop: 0.10,
            trail_stop_activation: 0.05,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    // day1 close 118 -> 1.18; day2 exits at 110 -> ×(110/118).
    assert!(
        (r.equity[2] - 1.10).abs() < 1e-9,
        "trail exit at 110, got {}",
        r.equity[2]
    );
}

#[test]
fn short_position_stop_loss_triggers_on_a_rise() {
    // Short entry at 100; stop_loss 8% for a short triggers when price RISES
    // to 108. Day1 high 110 touches; open 102 below -> fill at 108; a short
    // loses as price rises, so equity < 1.
    use crate::panel::Panel;
    let dates: Vec<i32> = (0..2).map(|i| 20240102 + i).collect();
    let pos = Panel::from_rows(
        dates.clone(),
        vec!["A".into()],
        vec![vec![-1.0], vec![-1.0]],
    )
    .unwrap();
    let mk = |v: Vec<f64>| {
        Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            v.into_iter().map(|x| vec![x]).collect(),
        )
        .unwrap()
    };
    let close = mk(vec![100.0, 106.0]);
    let open = mk(vec![100.0, 102.0]);
    let high = mk(vec![100.0, 110.0]);
    let low = mk(vec![100.0, 101.0]);
    let cfg = BacktestConfig {
        stops: StopConfig {
            stop_loss: 0.08,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &cfg,
        None,
    );
    // short return day1 = w(−1)·(108/100 − 1) = −0.08 -> equity 0.92.
    assert!(
        (r.equity[1] - 0.92).abs() < 1e-12,
        "short stop at 108, got {}",
        r.equity[1]
    );
}

#[test]
fn stops_off_by_default_leaves_the_curve_unchanged() {
    // A drop that would trip an 8% stop earns the full close-to-close move
    // when stops are off (default) — proving the feature is opt-in.
    let (pos, close, open, high, low) =
        stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 95.0)]);
    let r = run_with_initial(
        &pos,
        &close,
        Some(&open),
        Some(&high),
        Some(&low),
        None,
        &BacktestConfig::default(),
        None,
    );
    assert!(
        (r.equity[1] - 0.95).abs() < 1e-12,
        "no stop -> close 95, got {}",
        r.equity[1]
    );
}
