//! Integration tests for `yuzu-cli data-audit` (#133): a seeded data-layout tree
//! with an un-adjusted split, a mid-series gap, an all-NaN factor panel, and a
//! zero-lag (lookahead) fundamentals fixture — each must be flagged.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pomelo_data::csv_io::{write_series, OhlcvRow};
use pomelo_data::fundamentals::{write_fundamentals, FundamentalRow};
use pomelo_data::FUNDAMENTAL_FIELDS;
use yuzu_cli::data_audit::{Check, DataAuditReport, Status};
use yuzu_cli::run_data_audit;

fn ohlcv(day: i32, close: f64) -> OhlcvRow {
    OhlcvRow {
        day,
        adj_open: close,
        adj_high: close,
        adj_low: close,
        adj_close: close,
        volume: 0.0,
    }
}

fn write_prices(dir: &Path, sym: &str, bars: &[(i32, f64)]) {
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    let rows: Vec<OhlcvRow> = bars.iter().map(|&(d, c)| ohlcv(d, c)).collect();
    fs::write(
        p.join(format!("{sym}.csv.gz")),
        write_series(&rows).unwrap(),
    )
    .unwrap();
}

/// A tree that trips one specific check each:
/// - `GAP` skips 2024-01-04 (a mid-series hole vs the union calendar)
/// - `SPLIT` doubles overnight (candidate un-adjusted split)
/// - `DEAD` ends early (a delisted tail → survivorship OK)
/// - `GOOD` fundamentals file files on a month-end (lookahead smell)
/// - `panels/piotroski_score.csv` is all-NaN (the #132 empty-panel smell)
/// - `MISSING` is in the universe but has no prices (coverage warn)
fn build_tree(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yuzu_cli_audit_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    let d = [20240102, 20240103, 20240104, 20240105];

    write_prices(
        &dir,
        "GOOD",
        &[(d[0], 100.0), (d[1], 101.0), (d[2], 102.0), (d[3], 103.0)],
    );
    write_prices(&dir, "GAP", &[(d[0], 50.0), (d[1], 51.0), (d[3], 52.0)]); // 2024-01-04 missing
    write_prices(
        &dir,
        "SPLIT",
        &[(d[0], 100.0), (d[1], 100.0), (d[2], 200.0), (d[3], 201.0)],
    );
    write_prices(&dir, "DEAD", &[(d[0], 100.0), (d[1], 101.0)]); // ends before the last day

    // Fundamentals for GOOD: `pe` populated; a filing (report_event) on 2023-12-31,
    // a calendar month-end → the #131 lookahead smell.
    let fdir = dir.join("fundamentals");
    fs::create_dir_all(&fdir).unwrap();
    let mut vals = vec![f64::NAN; FUNDAMENTAL_FIELDS.len()];
    vals[0] = 15.0; // pe
    let rows = vec![
        FundamentalRow {
            day: 20231229,
            values: vals.clone(),
            report_event: 0.0,
        },
        FundamentalRow {
            day: 20231231,
            values: vals.clone(),
            report_event: 1.0,
        },
    ];
    fs::write(fdir.join("GOOD.csv.gz"), write_fundamentals(&rows).unwrap()).unwrap();

    // An all-NaN snapshot-factor panel (plain .csv — the loader probes .csv).
    let pdir = dir.join("panels");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(
        pdir.join("piotroski_score.csv"),
        "day,GOOD\n2024-01-02,\n2024-01-03,\n",
    )
    .unwrap();

    // Universe map with an extra name that has no price file.
    let tdir = dir.join("tracked");
    fs::create_dir_all(&tdir).unwrap();
    fs::write(
        tdir.join("universe.csv"),
        "symbol,sector,market_cap\nGOOD,Tech,1e12\nGAP,Tech,1e11\nSPLIT,Tech,1e11\nDEAD,Tech,1e10\nMISSING,Tech,1e9\n",
    )
    .unwrap();
    dir
}

fn check<'a>(report: &'a DataAuditReport, name: &str) -> &'a Check {
    report
        .checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check named {name}"))
}

#[test]
fn audit_flags_gap_split_lookahead_coverage_and_all_nan_panel() {
    let dir = build_tree("flags");
    let report = run_data_audit(&dir, 20000101, 99991231).unwrap();

    assert_eq!(report.symbol_count, 4);
    assert_eq!(report.overall, Status::Warn); // warnings, no hard FAIL

    // Coverage: MISSING is in the universe but unpriced.
    let cov = check(&report, "coverage");
    assert_eq!(cov.status, Status::Warn);
    assert!(cov.details["in_universe_missing_prices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "MISSING"));

    // Calendar gaps: GAP has one interior hole (2024-01-04).
    let gaps = check(&report, "calendar_gaps");
    assert_eq!(gaps.status, Status::Warn);
    assert_eq!(gaps.details["total_holes"], 1);

    // Adjustment: SPLIT's overnight doubling is flagged.
    let adj = check(&report, "adjustment");
    assert_eq!(adj.status, Status::Warn);
    assert_eq!(adj.details["flagged"], 1);

    // Survivorship: DEAD ends early, so the tree is NOT survivors-only.
    let surv = check(&report, "survivorship");
    assert_eq!(surv.status, Status::Ok);
    assert_eq!(surv.details["ended_early"], 1);

    // NaN density: the piotroski_score panel is all-NaN.
    let nan = check(&report, "nan_density");
    assert_eq!(nan.status, Status::Warn);
    assert!(nan.details["all_nan_factor_panels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "piotroski_score"));

    // PIT lag: the only filing day lands on a month-end → lookahead smell.
    let pit = check(&report, "pit_lag");
    assert_eq!(pit.status, Status::Warn);
    assert_eq!(pit.details["report_events"], 1);
    assert_eq!(pit.details["on_month_end"], 1);

    // No index membership panels present.
    assert_eq!(check(&report, "index_membership").status, Status::Ok);
}

#[test]
fn audit_empty_tree_fails_and_exits_nonzero() {
    let dir = std::env::temp_dir().join("yuzu_cli_audit_empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_yuzu-cli"))
        .args(["data-audit", "--data", dir.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(2), "empty tree must FAIL → exit 2");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("FAIL"),
        "table should show a FAIL: {stdout}"
    );
}

#[test]
fn audit_json_flag_emits_machine_report() {
    let dir = build_tree("json");
    let out = Command::new(env!("CARGO_BIN_EXE_yuzu-cli"))
        .args(["data-audit", "--data", dir.to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "warnings must still exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["overall"], "WARN");
    assert_eq!(v["symbol_count"], 4);
    assert!(v["checks"].as_array().unwrap().len() >= 7);
}
