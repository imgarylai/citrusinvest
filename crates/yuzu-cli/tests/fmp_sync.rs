//! End-to-end tests for `yuzu-cli fmp-sync` against a mock [`HttpClient`] — no
//! live key, no network (issue #52 acceptance: CI needs no key). The mock routes
//! by URL substring and can be scripted to fail before succeeding (retry path).

use std::cell::RefCell;
use std::time::Duration;

use yuzu_cli::fmp::{
    build_symbol_list, parse_symbols_list, sync, HttpClient, HttpError, SymbolFilter, SyncConfig,
    WriteMode, US_EXCHANGES,
};
use yuzu_cli::run_single;
use yuzu_data::csv_io::parse_series;
use yuzu_data::fundamentals::parse_fundamentals;
use yuzu_data::{Field, LocalSource, ObjectSource};

/// A scripted mock HTTP client. Each entry maps a URL *substring* to a queue of
/// responses; each GET pops the next response for the first matching pattern
/// (the last response repeats once the queue is down to one). Unmatched URLs
/// return a 404.
struct MockHttp {
    routes: Vec<(String, RefCell<Vec<Result<Vec<u8>, HttpError>>>)>,
    hits: RefCell<Vec<String>>,
}

impl MockHttp {
    fn new() -> Self {
        MockHttp {
            routes: Vec::new(),
            hits: RefCell::new(Vec::new()),
        }
    }

    /// Route every URL containing `pat` to `body` (JSON), repeated as needed.
    fn ok(mut self, pat: &str, body: &str) -> Self {
        self.routes.push((
            pat.to_string(),
            RefCell::new(vec![Ok(body.as_bytes().to_vec())]),
        ));
        self
    }

    /// Route `pat` to a scripted sequence of responses (consumed in order; the
    /// final one repeats).
    fn seq(mut self, pat: &str, seq: Vec<Result<Vec<u8>, HttpError>>) -> Self {
        self.routes.push((pat.to_string(), RefCell::new(seq)));
        self
    }

    fn hit_count(&self, pat: &str) -> usize {
        self.hits
            .borrow()
            .iter()
            .filter(|u| u.contains(pat))
            .count()
    }
}

impl HttpClient for MockHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.hits.borrow_mut().push(url.to_string());
        for (pat, queue) in &self.routes {
            if url.contains(pat) {
                let mut q = queue.borrow_mut();
                return if q.len() > 1 {
                    q.remove(0)
                } else {
                    q[0].clone()
                };
            }
        }
        Err(HttpError::Status(404))
    }
}

fn cfg() -> SyncConfig {
    SyncConfig {
        from: 20240102,
        to: 20240104,
        include_fundamentals: false,
        include_industry: false,
        skip_non_stocks: false, // most tests isolate prices; screening tests opt in
        min_market_cap: 0.0,
        rate_limit_per_min: 0, // no throttle in tests
        max_retries: 4,
        backoff_base: Duration::ZERO, // no sleeps in tests
        mode: WriteMode::Overwrite,
    }
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yuzu_fmp_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const AAPL_PRICES: &str = r#"[
    {"date":"2024-01-04","adjOpen":11.0,"adjHigh":12.0,"adjLow":10.5,"adjClose":11.5,"volume":1200},
    {"date":"2024-01-03","adjOpen":10.1,"adjHigh":11.5,"adjLow":9.8,"adjClose":10.8,"volume":1100},
    {"date":"2024-01-02","adjOpen":9.5,"adjHigh":11.0,"adjLow":9.0,"adjClose":10.0,"volume":1000}
]"#;

const MSFT_PRICES: &str = r#"[
    {"date":"2024-01-02","adjOpen":20.0,"adjHigh":21.0,"adjLow":19.0,"adjClose":20.0,"volume":500},
    {"date":"2024-01-03","adjOpen":20.0,"adjHigh":21.0,"adjLow":19.0,"adjClose":21.0,"volume":600},
    {"date":"2024-01-04","adjOpen":21.0,"adjHigh":22.0,"adjLow":20.0,"adjClose":22.0,"volume":700}
]"#;

#[test]
fn syncs_prices_and_the_tree_backtests() {
    let dir = tmp("prices");
    let http = MockHttp::new()
        .ok("symbol=AAPL", AAPL_PRICES)
        .ok("symbol=MSFT", MSFT_PRICES);
    let syms = vec!["AAPL".to_string(), "MSFT".to_string()];

    let summary = sync(&http, "KEY", &syms, &dir, &cfg()).unwrap();
    assert_eq!(summary.symbols_written, 2);
    assert_eq!(summary.price_rows, 6);
    assert!(summary.failures.is_empty());

    // The written file parses back, oldest-first with full OHLCV.
    let src = LocalSource::new(&dir);
    let bytes = src.get("prices/AAPL.csv.gz").unwrap().unwrap();
    assert_eq!(
        parse_series(&bytes, Field::AdjClose).unwrap(),
        vec![(20240102, 10.0), (20240103, 10.8), (20240104, 11.5)]
    );
    assert_eq!(
        parse_series(&bytes, Field::AdjHigh).unwrap()[0],
        (20240102, 11.0)
    );
    assert_eq!(
        parse_series(&bytes, Field::Volume).unwrap()[2],
        (20240104, 1200.0)
    );

    // Acceptance: yuzu-cli run can backtest a pure price strategy over the tree.
    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    let report = run_single(&dir, spec, 20240102, 20240104, &Default::default()).unwrap();
    assert_eq!(report.equity.len(), 3);
    assert!(report.metrics.total_return.is_finite());
}

#[test]
fn close_only_response_fills_ohl_from_close() {
    let dir = tmp("close_only");
    let http = MockHttp::new().ok(
        "symbol=AAA",
        r#"[{"date":"2024-01-02","adjClose":10.0},{"date":"2024-01-03","adjClose":11.0}]"#,
    );
    sync(&http, "KEY", &[String::from("AAA")], &dir, &cfg()).unwrap();
    let src = LocalSource::new(&dir);
    let bytes = src.get("prices/AAA.csv.gz").unwrap().unwrap();
    // high/low/open fall back to close so single-series strategies still run.
    assert_eq!(
        parse_series(&bytes, Field::AdjHigh).unwrap()[0],
        (20240102, 10.0)
    );
    assert_eq!(
        parse_series(&bytes, Field::AdjLow).unwrap()[1],
        (20240103, 11.0)
    );
    assert_eq!(
        parse_series(&bytes, Field::Volume).unwrap()[0],
        (20240102, 0.0)
    );
}

#[test]
fn retries_on_429_then_succeeds() {
    let dir = tmp("retry");
    let http = MockHttp::new().seq(
        "symbol=AAA",
        vec![
            Err(HttpError::Status(429)),
            Err(HttpError::Status(503)),
            Ok(AAPL_PRICES.as_bytes().to_vec()),
        ],
    );
    let summary = sync(&http, "KEY", &[String::from("AAA")], &dir, &cfg()).unwrap();
    assert_eq!(summary.symbols_written, 1);
    assert_eq!(http.hit_count("symbol=AAA"), 3); // two failures then success
}

#[test]
fn terminal_error_records_failure_and_continues_batch() {
    let dir = tmp("terminal");
    // AAA gets a hard 401 (bad key) — not retryable; BBB succeeds.
    let http = MockHttp::new()
        .seq("symbol=AAA", vec![Err(HttpError::Status(401))])
        .ok("symbol=BBB", MSFT_PRICES);
    let syms = vec!["AAA".to_string(), "BBB".to_string()];
    let summary = sync(&http, "KEY", &syms, &dir, &cfg()).unwrap();

    assert_eq!(summary.symbols_written, 1); // BBB
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(summary.failures[0].0, "AAA");
    assert_eq!(http.hit_count("symbol=AAA"), 1); // 401 is not retried
                                                 // The successful symbol still landed.
    assert!(LocalSource::new(&dir)
        .get("prices/BBB.csv.gz")
        .unwrap()
        .is_some());
}

#[test]
fn error_messages_redact_the_api_key() {
    let dir = tmp("redact");
    let http = MockHttp::new().seq("symbol=AAA", vec![Err(HttpError::Status(401))]);
    let summary = sync(&http, "SUPERSECRET", &[String::from("AAA")], &dir, &cfg()).unwrap();
    let msg = &summary.failures[0].1;
    assert!(!msg.contains("SUPERSECRET"), "leaked key: {msg}");
    assert!(msg.contains("***"), "expected redaction marker in: {msg}");
}

#[test]
fn append_merges_with_existing_history() {
    let dir = tmp("append");
    // First sync: only 2024-01-02.
    let http1 = MockHttp::new().ok(
        "symbol=AAA",
        r#"[{"date":"2024-01-02","adjClose":10.0,"volume":100}]"#,
    );
    let mut c = cfg();
    c.from = 20240102;
    c.to = 20240102;
    sync(&http1, "KEY", &[String::from("AAA")], &dir, &c).unwrap();

    // Second sync in append mode brings a new day + revises the old one.
    let http2 = MockHttp::new().ok(
        "symbol=AAA",
        r#"[{"date":"2024-01-02","adjClose":10.5,"volume":150},{"date":"2024-01-03","adjClose":11.0,"volume":200}]"#,
    );
    let mut c2 = cfg();
    c2.from = 20240102;
    c2.to = 20240103;
    c2.mode = WriteMode::Append;
    sync(&http2, "KEY", &[String::from("AAA")], &dir, &c2).unwrap();

    let bytes = LocalSource::new(&dir)
        .get("prices/AAA.csv.gz")
        .unwrap()
        .unwrap();
    // Union of days; fetched row wins on the collision (10.0 -> 10.5).
    assert_eq!(
        parse_series(&bytes, Field::AdjClose).unwrap(),
        vec![(20240102, 10.5), (20240103, 11.0)]
    );
}

#[test]
fn resume_skips_symbols_that_already_exist() {
    let dir = tmp("resume");
    let http1 = MockHttp::new().ok("symbol=AAA", AAPL_PRICES);
    sync(&http1, "KEY", &[String::from("AAA")], &dir, &cfg()).unwrap();

    // Resume: AAA already present (skipped without a fetch); BBB is fetched.
    let http2 = MockHttp::new()
        .ok("symbol=AAA", AAPL_PRICES)
        .ok("symbol=BBB", MSFT_PRICES);
    let mut c = cfg();
    c.mode = WriteMode::Resume;
    let summary = sync(&http2, "KEY", &["AAA".into(), "BBB".into()], &dir, &c).unwrap();

    assert_eq!(summary.symbols_skipped, 1);
    assert_eq!(summary.symbols_written, 1);
    assert_eq!(http2.hit_count("symbol=AAA"), 0); // never refetched
    assert_eq!(http2.hit_count("symbol=BBB"), 1);
}

#[test]
fn fundamentals_are_densified_onto_the_price_calendar() {
    let dir = tmp("fund");
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES) // matches any symbol's prices
        .ok(
            "ratios",
            r#"[{"date":"2024-01-03","priceToEarningsRatio":15.0,"netProfitMargin":0.2}]"#,
        )
        .ok(
            "key-metrics",
            r#"[{"date":"2024-01-03","marketCap":2.5e12}]"#,
        )
        .ok(
            "financial-growth",
            r#"[{"date":"2024-01-03","revenueGrowth":0.08}]"#,
        );
    let mut c = cfg();
    c.include_fundamentals = true;
    let summary = sync(&http, "KEY", &[String::from("AAPL")], &dir, &c).unwrap();
    assert_eq!(summary.fundamentals_written, 1);

    let bytes = LocalSource::new(&dir)
        .get("fundamentals/AAPL.csv.gz")
        .unwrap()
        .unwrap();
    // One row per trading day (3), forward-filled from the single annual snapshot.
    let pe = parse_fundamentals(&bytes, "pe").unwrap();
    assert_eq!(pe.len(), 3);
    assert!(pe[0].1.is_nan()); // before the 01-03 snapshot
    assert_eq!(pe[1].1, 15.0); // snapshot day
    assert_eq!(pe[2].1, 15.0); // carried forward
                               // Cross-endpoint fields merged in.
    assert_eq!(
        parse_fundamentals(&bytes, "market_cap").unwrap()[1].1,
        2.5e12
    );
    assert_eq!(
        parse_fundamentals(&bytes, "revenue_growth").unwrap()[1].1,
        0.08
    );
    // report_event flags the snapshot's first effective day.
    let ev = parse_fundamentals(&bytes, "report_event").unwrap();
    assert_eq!(ev[0].1, 0.0);
    assert_eq!(ev[1].1, 1.0);
    assert_eq!(ev[2].1, 0.0);
}

#[test]
fn industry_snapshot_is_written_from_profiles() {
    let dir = tmp("industry");
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES)
        .seq(
            "profile?symbol=AAPL",
            vec![Ok(
                br#"[{"symbol":"AAPL","sector":"Technology","marketCap":2.5e12}]"#.to_vec(),
            )],
        )
        .seq(
            "profile?symbol=XOM",
            vec![Ok(
                br#"[{"symbol":"XOM","sector":"Energy","marketCap":4.7e11}]"#.to_vec(),
            )],
        );
    let mut c = cfg();
    c.include_industry = true;
    let summary = sync(&http, "KEY", &["AAPL".into(), "XOM".into()], &dir, &c).unwrap();
    assert!(summary.industry_written);

    let bytes = LocalSource::new(&dir)
        .get("tracked/universe.csv.gz")
        .unwrap()
        .unwrap();
    let map = yuzu_data::industry::parse_industry_csv(&decode(&bytes));
    assert_eq!(map.get("AAPL").map(String::as_str), Some("Technology"));
    assert_eq!(map.get("XOM").map(String::as_str), Some("Energy"));
}

/// gunzip helper for reading back the written industry file.
fn decode(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut out = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut out)
        .unwrap();
    out
}

#[test]
fn skips_etfs_and_funds_by_default() {
    let dir = tmp("etf");
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES)
        .ok(
            "profile?symbol=SPY",
            r#"[{"symbol":"SPY","isEtf":true,"isFund":false,"sector":""}]"#,
        )
        .ok(
            "profile?symbol=VFIAX",
            r#"[{"symbol":"VFIAX","isEtf":false,"isFund":true,"sector":""}]"#,
        )
        .ok(
            "profile?symbol=AAPL",
            r#"[{"symbol":"AAPL","isEtf":false,"isFund":false,"sector":"Technology"}]"#,
        );
    let mut c = cfg();
    c.skip_non_stocks = true;
    let syms = vec!["SPY".to_string(), "VFIAX".to_string(), "AAPL".to_string()];
    let summary = sync(&http, "KEY", &syms, &dir, &c).unwrap();

    assert_eq!(summary.symbols_filtered, 2); // SPY (ETF) + VFIAX (fund)
    assert_eq!(summary.symbols_written, 1); // AAPL only
                                            // Screened before prices: the ETF/fund never cost a price request.
    assert_eq!(http.hit_count("dividend-adjusted?symbol=SPY"), 0);
    let src = LocalSource::new(&dir);
    assert!(src.get("prices/SPY.csv.gz").unwrap().is_none());
    assert!(src.get("prices/AAPL.csv.gz").unwrap().is_some());
}

#[test]
fn include_etf_keeps_them_and_skips_the_profile_fetch() {
    let dir = tmp("etf_keep");
    // Only prices are mocked; with screening off no profile GET should happen.
    let http = MockHttp::new().ok("historical-price-eod", AAPL_PRICES);
    let mut c = cfg();
    c.skip_non_stocks = false; // == --include-etf
    let summary = sync(&http, "KEY", &[String::from("SPY")], &dir, &c).unwrap();
    assert_eq!(summary.symbols_written, 1);
    assert_eq!(summary.symbols_filtered, 0);
    assert_eq!(http.hit_count("profile"), 0);
}

#[test]
fn min_market_cap_filters_small_caps() {
    let dir = tmp("mcap");
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES)
        .ok(
            "profile?symbol=BIG",
            r#"[{"symbol":"BIG","isEtf":false,"marketCap":5.0e12,"sector":"Tech"}]"#,
        )
        .ok(
            "profile?symbol=SMALL",
            r#"[{"symbol":"SMALL","isEtf":false,"marketCap":1.0e8,"sector":"Tech"}]"#,
        );
    let mut c = cfg();
    c.min_market_cap = 1.0e9; // 1B floor
    let summary = sync(&http, "KEY", &["BIG".into(), "SMALL".into()], &dir, &c).unwrap();

    assert_eq!(summary.symbols_written, 1); // BIG
    assert_eq!(summary.symbols_filtered, 1); // SMALL
    let src = LocalSource::new(&dir);
    assert!(src.get("prices/BIG.csv.gz").unwrap().is_some());
    assert!(src.get("prices/SMALL.csv.gz").unwrap().is_none());
}

#[test]
fn profile_error_fails_open_and_still_syncs_prices() {
    let dir = tmp("failopen");
    // Profile endpoint hard-fails, but prices are fine: the symbol must still land.
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES)
        .seq("profile?symbol=AAA", vec![Err(HttpError::Status(403))]);
    let mut c = cfg();
    c.skip_non_stocks = true;
    let summary = sync(&http, "KEY", &[String::from("AAA")], &dir, &c).unwrap();
    assert_eq!(summary.symbols_written, 1);
    assert_eq!(summary.symbols_filtered, 0);
}

#[test]
fn build_symbol_list_defaults_to_us_exchanges_and_supports_all() {
    // The US-majors default is pushed to the screener as an exchange param…
    let http = MockHttp::new().ok("company-screener", r#"[{"symbol":"AAA","isEtf":false}]"#);
    let filter = SymbolFilter {
        exchange: Some(US_EXCHANGES.to_string()),
        ..Default::default()
    };
    build_symbol_list(&http, "KEY", &cfg(), &filter).unwrap();
    assert_eq!(http.hit_count("exchange=NASDAQ,NYSE,AMEX"), 1);

    // …while `all` (and empty) is the escape hatch: no exchange filter at all.
    let http2 = MockHttp::new().ok("company-screener", r#"[{"symbol":"AAA","isEtf":false}]"#);
    let filter2 = SymbolFilter {
        exchange: Some("all".to_string()),
        ..Default::default()
    };
    build_symbol_list(&http2, "KEY", &cfg(), &filter2).unwrap();
    assert_eq!(http2.hit_count("exchange="), 0);
}

#[test]
fn build_symbol_list_screens_via_the_company_screener() {
    // Screener returns four rows; stocks-only + a 1B cap should keep BIG & MID.
    let http = MockHttp::new().ok(
        "company-screener",
        r#"[
            {"symbol":"BIG","marketCap":5.0e12,"isEtf":false},
            {"symbol":"SPY","marketCap":6.0e11,"isEtf":true},
            {"symbol":"MID","marketCap":2.0e9,"isEtf":false},
            {"symbol":"TINY","marketCap":1.0e8,"isEtf":false}
        ]"#,
    );
    let filter = SymbolFilter {
        min_market_cap: 1.0e9,
        include_etf: false,
        ..Default::default()
    };
    let syms = build_symbol_list(&http, "KEY", &cfg(), &filter).unwrap();
    assert_eq!(syms, vec!["BIG".to_string(), "MID".to_string()]);
}

#[test]
fn parse_symbols_list_handles_plain_lists_csv_and_comments() {
    let text = "# my universe\nAAPL\nMSFT\n\nsymbol,name\nGOOGL,Alphabet\n";
    assert_eq!(
        parse_symbols_list(text),
        vec!["AAPL".to_string(), "MSFT".to_string(), "GOOGL".to_string()]
    );
}

#[test]
fn empty_symbol_list_and_bad_range_error_out() {
    let dir = tmp("guards");
    let http = MockHttp::new();
    assert!(sync(&http, "KEY", &[], &dir, &cfg()).is_err());
    assert!(sync(&http, "", &[String::from("AAA")], &dir, &cfg()).is_err());
    let mut bad = cfg();
    bad.from = 20250101;
    bad.to = 20240101;
    assert!(sync(&http, "KEY", &[String::from("AAA")], &dir, &bad).is_err());
}
