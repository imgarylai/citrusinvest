use std::fs;
use yuzu_cli::{list_symbols, run_single, run_sweep, SortKey};
use yuzu_data::csv_io::{write_series, OhlcvRow};

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
    let report = run_single(&dir, spec, 20240102, 20240104, &Default::default()).unwrap();
    assert_eq!(report.equity.len(), 3);
    assert!(report.metrics.total_return.is_finite());
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
    let report = run_single(&dir, spec, 20240102, 20240104, &cfg).unwrap();
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
    let report = run_single(&dir, spec, 20240102, 20240104, &cfg).unwrap();
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
        SortKey::Sharpe,
    );
    assert_eq!(board.len(), 2);
    assert!(board.iter().all(|e| !e.ok && e.error.is_some()));
}
