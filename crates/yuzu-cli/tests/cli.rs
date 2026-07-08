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
