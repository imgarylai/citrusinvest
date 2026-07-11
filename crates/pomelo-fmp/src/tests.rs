//! Unit tests for the FMP sync modules.

use super::config::SyncConfig;
use super::delisted::{exchange_filter, keep_exchange, parse_delisted_rows, DelistedSymbol};
use super::fundamentals::{densify_fundamentals, merge_fundamentals, Snapshot};
use super::http::{redact, HttpError};
use super::price::parse_price_rows;
use super::universe::parse_market_cap;
use super::util::{i32_to_iso, iso_to_i32};
use pomelo_data::fundamentals::FUNDAMENTAL_FIELDS;

#[test]
fn parse_market_cap_handles_suffixes_and_plain_numbers() {
    assert_eq!(parse_market_cap("1b").unwrap(), 1e9);
    assert_eq!(parse_market_cap("500M").unwrap(), 5e8);
    assert_eq!(parse_market_cap("2.5t").unwrap(), 2.5e12);
    assert_eq!(parse_market_cap("10k").unwrap(), 1e4);
    assert_eq!(parse_market_cap("1e9").unwrap(), 1e9);
    assert_eq!(parse_market_cap("0").unwrap(), 0.0);
    assert_eq!(parse_market_cap("1000000").unwrap(), 1e6);
    assert!(parse_market_cap("").is_err());
    assert!(parse_market_cap("1x").is_err());
    assert!(parse_market_cap("-1b").is_err());
}

#[test]
fn redacts_the_api_key() {
    assert_eq!(
        redact("https://x/stable/profile?symbol=AAPL&apikey=SECRET"),
        "https://x/stable/profile?symbol=AAPL&apikey=***"
    );
    assert_eq!(
        redact("https://x/y?apikey=SECRET&from=2020-01-01"),
        "https://x/y?apikey=***&from=2020-01-01"
    );
    // No key → untouched.
    assert_eq!(redact("https://x/y?symbol=AAPL"), "https://x/y?symbol=AAPL");
}

#[test]
fn iso_date_roundtrips_and_tolerates_time_suffix() {
    assert_eq!(iso_to_i32("2024-01-02"), Some(20240102));
    assert_eq!(iso_to_i32("2024-01-02 00:00:00"), Some(20240102));
    assert_eq!(i32_to_iso(20240102), "2024-01-02");
    assert_eq!(iso_to_i32("garbage"), None);
}

#[test]
fn retryable_classification() {
    assert!(HttpError::Status(429).retryable());
    assert!(HttpError::Status(503).retryable());
    assert!(HttpError::Transport("reset".into()).retryable());
    assert!(!HttpError::Status(401).retryable());
    assert!(!HttpError::Status(404).retryable());
}

fn snap(visible: i32, fill: f64) -> Snapshot {
    Snapshot {
        visible,
        values: vec![fill; FUNDAMENTAL_FIELDS.len()],
        fell_back: false,
    }
}

#[test]
fn densify_forward_fills_and_marks_events() {
    // Two snapshots (by visibility day); a 4-day trading calendar straddling both.
    let snaps = vec![snap(20240102, 10.0), snap(20240104, 20.0)];
    let days = [20240101, 20240102, 20240103, 20240104];
    let rows = densify_fundamentals(&snaps, &days);
    assert_eq!(rows.len(), 4);
    // Before the first snapshot: NaN factors, no event.
    assert!(rows[0].values[0].is_nan());
    assert_eq!(rows[0].report_event, 0.0);
    // Snapshot day: value applied, event flagged.
    assert_eq!(rows[1].values[0], 10.0);
    assert_eq!(rows[1].report_event, 1.0);
    // Between snapshots: carried forward, no event.
    assert_eq!(rows[2].values[0], 10.0);
    assert_eq!(rows[2].report_event, 0.0);
    // Second snapshot day.
    assert_eq!(rows[3].values[0], 20.0);
    assert_eq!(rows[3].report_event, 1.0);
}

#[test]
fn merge_fundamentals_spreads_fields_across_endpoints() {
    let ratios = vec![serde_json::json!({"date":"2024-01-02","priceToEarningsRatio":15.0})];
    let metrics = vec![serde_json::json!({"date":"2024-01-02","marketCap":1.0e12})];
    let growth = vec![serde_json::json!({"date":"2024-01-02","revenueGrowth":0.08})];
    let snaps = merge_fundamentals(&[ratios, metrics, growth]);
    assert_eq!(snaps.len(), 1);
    let s = &snaps[0];
    // No filing date in any body → visible falls back to the period-end date.
    assert_eq!(s.visible, 20240102);
    assert!(s.fell_back);
    assert_eq!(s.values[0], 15.0); // pe
    assert_eq!(s.values[6], 1.0e12); // market_cap
    assert_eq!(s.values[11], 0.08); // revenue_growth
}

#[test]
fn merge_fundamentals_uses_filing_date_as_visibility() {
    // Fiscal year ends 2023-12-31 but the report is only filed 2024-02-01.
    // The snapshot must become visible on the *filing* day, not period-end.
    let ratios = vec![serde_json::json!({"date":"2023-12-31","priceToEarningsRatio":12.0})];
    let income = vec![serde_json::json!({
        "date":"2023-12-31","filingDate":"2024-02-01","revenue":5.0e11
    })];
    let snaps = merge_fundamentals(&[ratios, income]);
    assert_eq!(snaps.len(), 1);
    let s = &snaps[0];
    assert_eq!(s.visible, 20240201); // filing day, not 2023-12-31
    assert!(!s.fell_back);
    assert_eq!(s.values[0], 12.0); // pe (from ratios)
    assert_eq!(s.values[10], 5.0e11); // revenue backfilled from income-statement

    // And it must NOT appear in panel rows before the filing day.
    let days = [20240102, 20240201, 20240202];
    let rows = densify_fundamentals(&snaps, &days);
    assert!(rows[0].values[0].is_nan()); // period-end passed, but not yet filed
    assert_eq!(rows[0].report_event, 0.0);
    assert_eq!(rows[1].values[0], 12.0); // visible on filing day
    assert_eq!(rows[1].report_event, 1.0);
    assert_eq!(rows[2].values[0], 12.0); // carried forward
}

#[test]
fn merge_fundamentals_accepts_filing_date_aliases_and_sorts_by_visibility() {
    // Older misspelled `fillingDate` and `acceptedDate` (with a time suffix) are
    // both honored; snapshots come back ordered by visibility day.
    let a = vec![serde_json::json!({
        "date":"2023-12-31","fillingDate":"2024-03-01","priceToEarningsRatio":20.0
    })];
    let b = vec![serde_json::json!({
        "date":"2022-12-31","acceptedDate":"2023-02-15 17:04:00","priceToEarningsRatio":10.0
    })];
    let snaps = merge_fundamentals(&[a, b]);
    assert_eq!(snaps.len(), 2);
    // 2023-02-15 (older fiscal year, earlier filing) sorts first.
    assert_eq!(snaps[0].visible, 20230215);
    assert_eq!(snaps[0].values[0], 10.0);
    assert!(!snaps[0].fell_back);
    assert_eq!(snaps[1].visible, 20240301);
    assert_eq!(snaps[1].values[0], 20.0);
}

#[test]
fn parse_delisted_rows_extracts_symbol_exchange_and_date() {
    let rows = vec![
        serde_json::json!({
            "symbol":"DEAD","companyName":"Dead Co","exchange":"NASDAQ",
            "ipoDate":"2002-05-21","delistedDate":"2024-01-03"
        }),
        // Missing delistedDate → symbol still kept, date None.
        serde_json::json!({"symbol":"OLD","exchange":"NYSE"}),
        // Blank symbol → dropped.
        serde_json::json!({"symbol":"  ","exchange":"NASDAQ","delistedDate":"2020-01-01"}),
    ];
    let out = parse_delisted_rows(&rows);
    assert_eq!(
        out,
        vec![
            DelistedSymbol {
                symbol: "DEAD".into(),
                exchange: "NASDAQ".into(),
                delisted_date: Some(20240103),
            },
            DelistedSymbol {
                symbol: "OLD".into(),
                exchange: "NYSE".into(),
                delisted_date: None,
            },
        ]
    );
}

#[test]
fn exchange_filter_keeps_only_wanted_exchanges() {
    // None / empty / "all" → keep every exchange.
    assert!(exchange_filter(None).is_none());
    assert!(exchange_filter(Some("")).is_none());
    assert!(exchange_filter(Some("all")).is_none());

    let us = exchange_filter(Some("NASDAQ,NYSE,AMEX"));
    assert!(keep_exchange(&us, "nasdaq")); // case-insensitive
    assert!(keep_exchange(&us, "NYSE"));
    assert!(!keep_exchange(&us, "OTC")); // OTC filtered out
    assert!(!keep_exchange(&us, "CBOE"));
    // No filter keeps even the odd ones.
    assert!(keep_exchange(&None, "OTC"));
}

#[test]
fn parse_price_rows_uses_close_fallback_and_clamps_range() {
    let cfg = SyncConfig {
        from: 20240102,
        to: 20240103,
        ..Default::default()
    };
    let rows = vec![
        // out of range (dropped)
        serde_json::json!({"date":"2024-01-01","adjClose":9.0}),
        // adjusted OHLC present
        serde_json::json!({"date":"2024-01-02","adjOpen":9.5,"adjHigh":11.0,"adjLow":9.0,"adjClose":10.0,"volume":1000}),
        // close only → OHL fall back to close, volume 0
        serde_json::json!({"date":"2024-01-03","adjClose":11.0}),
    ];
    let out = parse_price_rows(&rows, &cfg);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].day, 20240102);
    assert_eq!(out[0].adj_high, 11.0);
    assert_eq!(out[0].volume, 1000.0);
    assert_eq!(out[1].day, 20240103);
    assert_eq!(out[1].adj_open, 11.0); // fallback to close
    assert_eq!(out[1].volume, 0.0);
}
