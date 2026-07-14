use pomelo_data::csv_io::{write_series, OhlcvRow};
use std::fs;
use yuzu_cli::{list_symbols, run_single, run_sweep, SortKey};

fn fixture(tag: &str) -> std::path::PathBuf {
    // per-test temp dir (tests run in parallel) holding prices/<sym>.csv.gz for AAA, BBB.
    let dir = std::env::temp_dir().join(format!("yuzu_cli_fix_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    for (sym, closes) in [
        ("AAA", [10.0_f64, 11.0, 12.0]),
        ("BBB", [5.0_f64, 4.0, 6.0]),
    ] {
        let rows: Vec<OhlcvRow> = closes
            .iter()
            .enumerate()
            .map(|(i, &c)| OhlcvRow {
                day: 20240102 + i as i32,
                adj_open: c,
                adj_high: c,
                adj_low: c,
                adj_close: c,
                volume: 0.0,
            })
            .collect();
        let p = dir.join("prices");
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join(format!("{sym}.csv.gz")),
            write_series(&rows).unwrap(),
        )
        .unwrap();
    }
    dir
}

#[test]
fn sweeps_variants_and_ranks_them() {
    let dir = fixture("sweep");
    let variants = vec![
        (
            "hold_top1".to_string(),
            r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#.to_string(),
        ),
        (
            "hold_all".to_string(),
            r#"{"op":"Data","name":"close"}"#.to_string(),
        ),
        (
            "broken".to_string(),
            r#"{"op":"Data","name":"missing"}"#.to_string(),
        ), // unknown series -> error
    ];
    let board = run_sweep(
        &dir,
        &variants,
        20240102,
        20240104,
        &Default::default(),
        "close",
        SortKey::Sharpe,
    );

    assert_eq!(board.len(), 3);
    assert!(board[0].ok); // a successful run ranks first
    assert!(!board.last().unwrap().ok); // the broken variant sinks to the bottom
    assert_eq!(board.iter().filter(|e| e.ok).count(), 2);
}

#[test]
fn lists_symbols_and_runs_a_single_backtest() {
    let dir = fixture("run");
    assert_eq!(
        list_symbols(&dir).unwrap(),
        vec!["AAA".to_string(), "BBB".to_string()]
    );

    // is_largest(close, 1), rebalanced — always holds one name; just assert it runs + shapes.
    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    let report = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        None,
    )
    .unwrap();
    assert_eq!(report.equity.len(), 3);
    assert!(report.metrics.total_return.is_finite());
    assert!(report.live.is_none()); // no live block by default

    // live_performance_start attaches a post-live segment (rows from 20240103).
    let cfg = yuzu_core::backtest::BacktestConfig {
        live_performance_start: Some(20240103),
        ..Default::default()
    };
    let report = run_single(&dir, spec, 20240102, 20240104, &cfg, "close", None).unwrap();
    let seg = report.live.as_ref().unwrap();
    assert_eq!(seg.start, 20240103);
    assert_eq!(seg.days, 2);
    assert!(seg.total_return.is_finite());
}

#[test]
fn scopes_the_universe_to_an_explicit_symbol_list() {
    let dir = fixture("scoped");
    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    // Unscoped, is_largest holds AAA (10→11→12); scoped to BBB alone it must
    // ride BBB's 5→4→6 path — the dip to 0.8 proves the universe shrank.
    let bbb = ["BBB".to_string()];
    let scoped = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        Some(&bbb),
    )
    .unwrap();
    for (got, want) in scoped.equity.iter().zip([1.0, 0.8, 1.2]) {
        assert!((got - want).abs() < 1e-9, "{:?}", scoped.equity);
    }
    let full = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        None,
    )
    .unwrap();
    assert_ne!(full.equity, scoped.equity);

    // A requested symbol with no price file is an error, never a silent drop.
    let bad = ["BBB".to_string(), "ZZZ".to_string()];
    match run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        Some(&bad),
    ) {
        Err(e) => assert!(e.contains("ZZZ"), "{e}"),
        Ok(_) => panic!("expected an error for a symbol missing from the tree"),
    }
    // An empty list is a mistake, not "no filter".
    match run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        Some(&[]),
    ) {
        Err(e) => assert!(e.contains("empty"), "{e}"),
        Ok(_) => panic!("expected an error for an empty symbols list"),
    }
}

#[test]
fn price_key_selects_the_execution_and_return_series() {
    // One symbol whose OPEN and CLOSE tell different stories: open rises
    // 10→12→14 (+40%), close falls 10→9→8 (−20%). A buy-and-hold's total
    // return must come off whichever series --price-key names.
    let dir = std::env::temp_dir().join("yuzu_cli_pricekey");
    let _ = fs::remove_dir_all(&dir);
    let opens = [10.0_f64, 12.0, 14.0];
    let closes = [10.0_f64, 9.0, 8.0];
    let rows: Vec<OhlcvRow> = (0..3)
        .map(|i| OhlcvRow {
            day: 20240102 + i as i32,
            adj_open: opens[i],
            adj_high: opens[i].max(closes[i]),
            adj_low: opens[i].min(closes[i]),
            adj_close: closes[i],
            volume: 0.0,
        })
        .collect();
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("ONE.csv.gz"), write_series(&rows).unwrap()).unwrap();

    // Hold the single name every day; signal references close but fills follow
    // the chosen price series.
    let spec = r#"{"op":"Data","name":"close"}"#;
    let on_close = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "close",
        None,
    )
    .unwrap();
    assert!((on_close.metrics.total_return - (-0.2)).abs() < 1e-9); // 8/10 - 1

    let on_open = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "open",
        None,
    )
    .unwrap();
    assert!((on_open.metrics.total_return - 0.4).abs() < 1e-9); // 14/10 - 1

    // An unknown price series fails fast with a clear message.
    match run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &Default::default(),
        "bogus",
        None,
    ) {
        Err(e) => assert!(e.contains("price-key must be one of"), "{e}"),
        Ok(_) => panic!("expected an error for an unknown price-key"),
    }
}

#[test]
fn grid_expands_cartesian_product_with_placeholders() {
    let grid: yuzu_cli::GridSpec = serde_json::from_str(
        r#"{
            "spec": {"op":"Average","of":{"op":"Data","name":"close"},"n":"$n","x":"$thresh"},
            "params": {"n": [10, 20], "thresh": [0.5]}
        }"#,
    )
    .unwrap();
    let variants = yuzu_cli::expand_grid(&grid);
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].0, "n=10,thresh=0.5");
    assert_eq!(variants[0].1["n"], 10);
    assert_eq!(variants[0].1["x"], 0.5);
    assert_eq!(variants[1].1["n"], 20);
    // non-placeholder strings and unknown placeholders pass through untouched
    assert_eq!(variants[0].1["of"]["name"], "close");

    // no params -> the spec itself
    let plain: yuzu_cli::GridSpec =
        serde_json::from_str(r#"{"spec": {"op":"Data","name":"close"}}"#).unwrap();
    assert_eq!(yuzu_cli::expand_grid(&plain).len(), 1);
}

/// A 12-day fixture where AAA always rises and BBB always falls — every
/// window's best variant is "hold AAA-style top-1 by momentum".
fn long_fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yuzu_cli_fix_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    for (sym, base, step) in [("AAA", 10.0_f64, 0.5_f64), ("BBB", 20.0, -0.5)] {
        let rows: Vec<OhlcvRow> = (0..12)
            .map(|i| {
                let c = base + step * i as f64;
                OhlcvRow {
                    day: 20240102 + i as i32, // 20240102..20240113, all valid dates
                    adj_open: c,
                    adj_high: c,
                    adj_low: c,
                    adj_close: c,
                    volume: 0.0,
                }
            })
            .collect();
        let p = dir.join("prices");
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join(format!("{sym}.csv.gz")),
            write_series(&rows).unwrap(),
        )
        .unwrap();
    }
    dir
}

#[test]
fn walkforward_picks_in_sample_winner_and_chains_oos_equity() {
    let dir = long_fixture("wf");
    // Two variants: hold the 1-day riser (picks AAA) vs the 1-day faller (picks BBB).
    let variants = vec![
        (
            "riser".to_string(),
            r#"{"op":"Rise","of":{"op":"Data","name":"close"},"n":1}"#.to_string(),
        ),
        (
            "faller".to_string(),
            r#"{"op":"Fall","of":{"op":"Data","name":"close"},"n":1}"#.to_string(),
        ),
    ];
    let report = yuzu_cli::run_walkforward(
        &dir,
        &variants,
        &yuzu_cli::WalkForwardParams {
            from: 20240102,
            to: 20240113,
            train_days: 4,
            test_days: 3,
            sort_by: SortKey::TotalReturn,
            warmup_days: None,
        },
        &Default::default(),
    )
    .unwrap();

    // 12 days: windows at rows [0..4)+[4..7), [7..11)+[11..12) -> 2 windows.
    assert_eq!(report.windows.len(), 2);
    // AAA rises all the way: the riser wins in-sample and gains out-of-sample.
    for w in &report.windows {
        assert_eq!(w.chosen, "riser", "window {}..{}", w.train_from, w.test_to);
    }
    assert!(report.total_return > 0.0);
    // stitched OOS curve covers only the test rows (3 + 1)
    assert_eq!(report.equity.len(), 4);
    assert_eq!(report.dates.len(), 4);
    // errors on impossible windows
    assert!(yuzu_cli::run_walkforward(
        &dir,
        &variants,
        &yuzu_cli::WalkForwardParams {
            from: 20240102,
            to: 20240113,
            train_days: 50,
            test_days: 3,
            sort_by: SortKey::Sharpe,
            warmup_days: None,
        },
        &Default::default(),
    )
    .is_err());
}

#[test]
fn max_lookback_finds_largest_window_arg() {
    let spec: serde_json::Value = serde_json::from_str(
        r#"{"op":"And",
            "l":{"op":"Gt","l":{"op":"Data","name":"close"},
                 "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":4}},
            "r":{"op":"Sustain","of":{"op":"Data","name":"sig"},"nwindow":20,"nsatisfy":3}}"#,
    )
    .unwrap();
    assert_eq!(yuzu_cli::max_lookback(&spec), 20);
    assert_eq!(
        yuzu_cli::max_lookback(&serde_json::json!({"op":"Data","name":"x"})),
        0
    );
}

#[test]
fn walkforward_warmup_captures_returns_cold_start_misses() {
    let dir = long_fixture("wf_warmup");
    // close > sma(close, 4): on ever-rising AAA this is true wherever the SMA
    // exists. Cold-started windows lose the first rows to SMA warmup.
    let variants = vec![(
        "sma4".to_string(),
        r#"{"op":"Gt","l":{"op":"Data","name":"close"},
            "r":{"op":"Average","of":{"op":"Data","name":"close"},"n":4}}"#
            .to_string(),
    )];
    let run = |warmup_days| {
        yuzu_cli::run_walkforward(
            &dir,
            &variants,
            &yuzu_cli::WalkForwardParams {
                from: 20240102,
                to: 20240113,
                train_days: 4,
                test_days: 3,
                sort_by: SortKey::TotalReturn,
                warmup_days,
            },
            &Default::default(),
        )
        .unwrap()
    };
    let cold = run(Some(0));
    let warm = run(None); // auto -> max_lookback = 4

    // same OOS date axis either way, strictly increasing (no duplicated
    // boundary rows)
    assert_eq!(warm.dates, cold.dates);
    assert!(warm.dates.windows(2).all(|w| w[0] < w[1]));
    // warmup means the signal is live from the first test day AND the
    // boundary-day return is priced -> strictly more OOS return on a riser.
    assert!(
        warm.total_return > cold.total_return + 1e-9,
        "warm {} vs cold {}",
        warm.total_return,
        cold.total_return
    );
}

#[test]
fn factor_and_event_report_over_the_tree() {
    let dir = fixture("research"); // AAA rises 10→12, BBB dips then recovers
                                   // Factor = close level; 1-day forward returns; 2 buckets.
    let spec = r#"{"op":"Data","name":"close"}"#;
    let fr = yuzu_cli::run_factor(&dir, spec, 20240102, 20240104, 1, 2, false).unwrap();
    assert_eq!(fr.quantiles, 2);
    assert_eq!(fr.quantile_returns.len(), 2);
    assert!(fr.mean_ic.is_finite());

    // Event = day close rose; average the return path around it.
    let ev_spec = r#"{"op":"Rise","of":{"op":"Data","name":"close"},"n":1}"#;
    let es = yuzu_cli::run_event(&dir, ev_spec, 20240102, 20240104, 1, 1).unwrap();
    assert_eq!(es.lags, vec![-1, 0, 1]);
    assert!(es.event_count >= 1); // AAA rises every day
}

#[test]
fn walkforward_carries_holdings_across_seams() {
    // Flat prices → nothing drifts, so the winner's book is a constant {AAA:1.0}
    // every window. `close > 0` holds AAA with zero lookback, so the boundary
    // rows are live too. With a 2% fee, the FIRST window pays a full entry cost;
    // the SECOND enters from the carried book (identical target) and pays ZERO
    // seam turnover — the crux of #21.
    let dir = std::env::temp_dir().join("yuzu_cli_wf_carry");
    let _ = fs::remove_dir_all(&dir);
    let rows: Vec<OhlcvRow> = (0..12)
        .map(|i| OhlcvRow {
            day: 20240102 + i as i32,
            adj_open: 10.0,
            adj_high: 10.0,
            adj_low: 10.0,
            adj_close: 10.0,
            volume: 0.0,
        })
        .collect();
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("AAA.csv.gz"), write_series(&rows).unwrap()).unwrap();

    let variants = vec![(
        "always".to_string(),
        r#"{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":0.0}}"#
            .to_string(),
    )];
    let cfg = yuzu_core::backtest::BacktestConfig {
        fee_ratio: 0.02,
        ..Default::default()
    };
    let report = yuzu_cli::run_walkforward(
        &dir,
        &variants,
        &yuzu_cli::WalkForwardParams {
            from: 20240102,
            to: 20240113,
            train_days: 4,
            test_days: 3,
            sort_by: SortKey::TotalReturn, // flat curve → total_return 0, not NaN
            warmup_days: None,
        },
        &cfg,
    )
    .unwrap();

    assert_eq!(report.windows.len(), 2);
    // Window 0 enters flat: a full 2% entry cost on flat prices.
    assert!(
        (report.windows[0].oos_return - (-0.02)).abs() < 1e-12,
        "first seam should pay the entry fee, got {}",
        report.windows[0].oos_return
    );
    // Window 1 carries the identical book → zero seam turnover, no fee.
    assert!(
        report.windows[1].oos_return.abs() < 1e-12,
        "carried seam should pay no turnover, got {}",
        report.windows[1].oos_return
    );
}

#[test]
fn list_symbols_is_empty_when_prices_dir_is_absent() {
    let dir = std::env::temp_dir().join("yuzu_cli_no_prices");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // No prices/ subdir at all → empty list, not an error.
    assert!(list_symbols(&dir).unwrap().is_empty());
}

#[test]
fn list_symbols_skips_non_files_and_non_matching_names() {
    let dir = fixture("mixed");
    let prices = dir.join("prices");
    // A subdirectory under prices/ (not a file) and a file without the .csv.gz
    // suffix are both ignored; only AAA and BBB remain.
    fs::create_dir_all(prices.join("nested_dir")).unwrap();
    fs::write(prices.join("README.txt"), b"not a price file").unwrap();
    assert_eq!(
        list_symbols(&dir).unwrap(),
        vec!["AAA".to_string(), "BBB".to_string()]
    );
}

#[test]
fn run_single_loads_volume_panel_for_the_liquidity_cap() {
    let dir = fixture("liquidity");
    // A non-zero cap loads the volume panel in addition to close.
    let cfg = yuzu_core::backtest::BacktestConfig {
        initial_capital: 1_000_000.0,
        max_participation: 0.1,
        ..Default::default()
    };
    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    let report = run_single(&dir, spec, 20240102, 20240104, &cfg, "close", None).unwrap();
    assert_eq!(report.equity.len(), 3);
}

#[test]
fn run_single_loads_a_benchmark_symbol_panel() {
    let dir = fixture("bench");
    // benchmark_key names a symbol (AAA) that isn't itself a panel key ("close"),
    // so its closes are loaded into a dedicated "AAA" panel.
    let cfg = yuzu_core::backtest::BacktestConfig {
        benchmark_key: Some("AAA".to_string()),
        ..Default::default()
    };
    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    let report = run_single(&dir, spec, 20240102, 20240104, &cfg, "close", None).unwrap();
    assert!(report.benchmark.is_some());
}

#[test]
fn sweep_ranks_by_every_sort_key() {
    let dir = fixture("sortkeys");
    let variants = vec![
        (
            "top1".to_string(),
            r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#.to_string(),
        ),
        (
            "all".to_string(),
            r#"{"op":"Data","name":"close"}"#.to_string(),
        ),
    ];
    for key in [
        SortKey::Sharpe,
        SortKey::TotalReturn,
        SortKey::Cagr,
        SortKey::Calmar,
    ] {
        let board = run_sweep(
            &dir,
            &variants,
            20240102,
            20240104,
            &Default::default(),
            "close",
            key,
        );
        assert_eq!(board.len(), 2);
        assert!(board.iter().all(|e| e.ok));
    }
}

#[test]
fn sweep_marks_every_variant_failed_when_the_panel_cannot_load() {
    // `prices` is a regular file, not a directory: listing symbols errors, so the
    // shared panel never loads and every variant reports that same load error
    // instead of a per-variant result.
    let dir = std::env::temp_dir().join("yuzu_cli_unreadable");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("prices"), b"i am a file, not a directory").unwrap();

    let variants = vec![
        (
            "a".to_string(),
            r#"{"op":"Data","name":"close"}"#.to_string(),
        ),
        (
            "b".to_string(),
            r#"{"op":"Data","name":"close"}"#.to_string(),
        ),
    ];
    let board = run_sweep(
        &dir,
        &variants,
        20240102,
        20240104,
        &Default::default(),
        "close",
        SortKey::Sharpe,
    );
    assert_eq!(board.len(), 2);
    assert!(board.iter().all(|e| !e.ok && e.error.is_some()));
}

#[test]
fn lookahead_flags_strategies_that_collapse_under_execution_lag() {
    // Price alternates: odd rows fall ~1%, even rows rise ~5%. The strategy
    // "hold after a down day" (fall(close, 1)) enters at each fall-day close
    // and captures every +5% day — but ONLY with same-close execution. Lagged
    // one day it holds through every fall instead.
    let dir = std::env::temp_dir().join("yuzu_cli_fix_lookahead");
    let _ = fs::remove_dir_all(&dir);
    let mut c = 100.0_f64;
    let rows: Vec<OhlcvRow> = (0..20)
        .map(|i| {
            if i > 0 {
                c *= if i % 2 == 1 { 0.99 } else { 1.05 };
            }
            OhlcvRow {
                day: 20240102 + i as i32,
                adj_open: c,
                adj_high: c,
                adj_low: c,
                adj_close: c,
                volume: 0.0,
            }
        })
        .collect();
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("PAT.csv.gz"), write_series(&rows).unwrap()).unwrap();

    let spec = r#"{"op":"Fall","of":{"op":"Data","name":"close"},"n":1}"#;
    let report =
        yuzu_cli::run_lookahead(&dir, spec, 20240102, 20240131, 1, &Default::default()).unwrap();

    assert!(
        report.baseline.total_return > 0.5,
        "baseline captures the rises: {}",
        report.baseline.total_return
    );
    assert!(
        report.lagged.total_return < 0.0,
        "lagged holds the falls: {}",
        report.lagged.total_return
    );
    assert!(report.sharpe_drop > 0.0);
    assert!(report.suspicious, "collapse under lag must be flagged");

    // A lag-robust strategy (always in the market) is NOT flagged.
    let robust = r#"{"op":"Gt","l":{"op":"Data","name":"close"},"r":{"op":"Const","value":0.0}}"#;
    let r2 =
        yuzu_cli::run_lookahead(&dir, robust, 20240102, 20240131, 1, &Default::default()).unwrap();
    assert!(!r2.suspicious, "buy-and-hold survives a 1-day lag");

    // shift_days = 0 is rejected.
    assert!(
        yuzu_cli::run_lookahead(&dir, spec, 20240102, 20240131, 0, &Default::default()).is_err()
    );
}

#[test]
fn lookahead_profile_shows_the_decay_curve() {
    // Same alternating fixture as the single-shift test: the edge lives
    // entirely in same-close execution, so the curve cliffs at shift 1.
    let dir = std::env::temp_dir().join("yuzu_cli_fix_lookahead_profile");
    let _ = fs::remove_dir_all(&dir);
    let mut c = 100.0_f64;
    let rows: Vec<OhlcvRow> = (0..40)
        .map(|i| {
            if i > 0 {
                c *= if i % 2 == 1 { 0.99 } else { 1.05 };
            }
            OhlcvRow {
                day: 20240101 + (i / 28) as i32 * 100 + (i % 28) as i32 + 1, // valid dates
                adj_open: c,
                adj_high: c,
                adj_low: c,
                adj_close: c,
                volume: 0.0,
            }
        })
        .collect();
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("PAT.csv.gz"), write_series(&rows).unwrap()).unwrap();

    let spec = r#"{"op":"Fall","of":{"op":"Data","name":"close"},"n":1}"#;
    let profile = yuzu_cli::run_lookahead_profile(
        &dir,
        spec,
        20240101,
        20240301,
        &[1, 2, 5],
        &Default::default(),
    )
    .unwrap();

    assert!(profile.baseline.total_return > 0.5);
    assert_eq!(profile.points.len(), 3);
    assert_eq!(profile.points[0].shift_days, 1);
    // Cliff at shift 1: retention collapses immediately.
    assert!(profile.points[0].sharpe < 0.0);
    assert!(profile.points[0].sharpe_retention < 0.5);
    assert!(profile.suspicious);
    // Even shifts re-align with the period-2 pattern: shift 2 recovers most
    // of the baseline (that's the shape telling you the period), odd shifts
    // stay inverted.
    assert!(profile.points[1].shift_days == 2 && profile.points[1].sharpe > 0.0);

    // Guards: zero shift and empty ladder are rejected.
    assert!(yuzu_cli::run_lookahead_profile(
        &dir,
        spec,
        20240101,
        20240301,
        &[0],
        &Default::default()
    )
    .is_err());
    assert!(yuzu_cli::run_lookahead_profile(
        &dir,
        spec,
        20240101,
        20240301,
        &[],
        &Default::default()
    )
    .is_err());
}
