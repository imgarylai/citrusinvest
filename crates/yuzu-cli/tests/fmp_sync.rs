//! End-to-end tests for `yuzu-cli fmp-sync` against a mock [`HttpClient`] — no
//! live key, no network (issue #52 acceptance: CI needs no key). The mock routes
//! by URL substring and can be scripted to fail before succeeding (retry path).

use std::cell::RefCell;
use std::time::Duration;

use pomelo_data::csv_io::parse_series;
use pomelo_data::fundamentals::parse_fundamentals;
use pomelo_data::{load_combined_panel, Field, LocalSource, ObjectSource, PANELS_DIR};
use yuzu_cli::fmp::{
    build_symbol_list, fetch_delisted, parse_symbols_list, sync, HttpClient, HttpError, Index,
    IndexMembership, SymbolFilter, SyncConfig, WriteMode, US_EXCHANGES,
};
use yuzu_cli::{run_single, write_index_membership};
use yuzu_core::backtest::BacktestConfig;

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
        include_snapshot_factors: false,
        skip_non_stocks: false, // most tests isolate prices; screening tests opt in
        min_market_cap: 0.0,
        rate_limit_per_min: 0, // no throttle in tests
        max_retries: 4,
        backoff_base: Duration::ZERO, // no sleeps in tests
        mode: WriteMode::Overwrite,
    }
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pomelo_fmp_{tag}"));
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
}

#[test]
fn delisted_universe_is_filtered_by_exchange_and_paged() {
    // Page 0 has one US name (kept) and one OTC name (filtered by US_EXCHANGES);
    // page 1 is empty, which stops the walk.
    let http = MockHttp::new().seq(
        "delisted-companies",
        vec![
            Ok(br#"[
                {"symbol":"DEAD","exchange":"NASDAQ","delistedDate":"2024-01-03"},
                {"symbol":"OTCJUNK","exchange":"OTC","delistedDate":"2019-06-01"}
            ]"#
            .to_vec()),
            Ok(b"[]".to_vec()),
        ],
    );
    let got = fetch_delisted(&http, "KEY", &cfg(), Some(US_EXCHANGES)).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].symbol, "DEAD");
    assert_eq!(got[0].delisted_date, Some(20240103));
    assert_eq!(http.hit_count("delisted-companies"), 2); // walked page 0 then the empty page 1
}

#[test]
fn delisted_symbol_price_file_ends_at_delist_date_and_engine_forces_exit() {
    // #124 acceptance: a delisted name's price file simply ends at its delisting
    // date, and the engine's delist_after forced-exit fires on the NaN gap.
    let dir = tmp("delist");
    // DEAD stops trading after 2024-01-03; the price feed returns nothing after.
    // LIVE trades the full window.
    let dead_prices = r#"[
        {"date":"2024-01-02","adjClose":10.0,"volume":100},
        {"date":"2024-01-03","adjClose":10.0,"volume":100}
    ]"#;
    let live_prices = r#"[
        {"date":"2024-01-02","adjClose":10.0,"volume":100},
        {"date":"2024-01-03","adjClose":10.0,"volume":100},
        {"date":"2024-01-04","adjClose":10.0,"volume":100}
    ]"#;
    let http = MockHttp::new()
        .seq(
            "delisted-companies",
            vec![
                Ok(
                    br#"[{"symbol":"DEAD","exchange":"NASDAQ","delistedDate":"2024-01-03"}]"#
                        .to_vec(),
                ),
                Ok(b"[]".to_vec()),
            ],
        )
        .ok("symbol=DEAD", dead_prices)
        .ok("symbol=LIVE", live_prices);

    // Union the delisted universe into the sync list, exactly like the CLI does.
    let delisted = fetch_delisted(&http, "KEY", &cfg(), Some(US_EXCHANGES)).unwrap();
    let mut syms = vec!["LIVE".to_string()];
    syms.extend(delisted.into_iter().map(|d| d.symbol));
    let summary = sync(&http, "KEY", &syms, &dir, &cfg()).unwrap();
    assert_eq!(summary.symbols_written, 2);

    // The dead name's file ends at the delisting date; the live one runs past it.
    let src = LocalSource::new(&dir);
    let dead = parse_series(
        &src.get("prices/DEAD.csv.gz").unwrap().unwrap(),
        Field::AdjClose,
    )
    .unwrap();
    assert_eq!(
        dead.last().unwrap().0,
        20240103,
        "DEAD must end at delist date"
    );
    let live = parse_series(
        &src.get("prices/LIVE.csv.gz").unwrap().unwrap(),
        Field::AdjClose,
    )
    .unwrap();
    assert_eq!(live.last().unwrap().0, 20240104);

    // Hold both names equally; on 2024-01-04 DEAD has a missing price. With
    // delist_after=1 + a full haircut the position is force-closed at a total
    // loss, so equity diverges from the survivorship-friendly delist_after=0
    // path — i.e. the forced exit fired on the synced tree.
    let spec =
        r#"{"op":"NormalizeRow","of":{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":2}}"#;
    let survivor = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &BacktestConfig::default(),
        "close",
        None,
    )
    .unwrap();
    let honest = run_single(
        &dir,
        spec,
        20240102,
        20240104,
        &BacktestConfig {
            delist_after: 1,
            delist_haircut: 1.0,
            ..Default::default()
        },
        "close",
        None,
    )
    .unwrap();
    assert!(
        (survivor.metrics.total_return - honest.metrics.total_return).abs() > 1e-9,
        "delist forced-exit did not change the result: survivor={} honest={}",
        survivor.metrics.total_return,
        honest.metrics.total_return
    );
}

#[test]
fn index_pit_membership_reconstructs_syncs_and_masks_a_backtest() {
    // #125 acceptance: reconstruct S&P 500 membership from the current snapshot +
    // change log, sync the ever-members, write the in_sp500 panel, and consume it
    // via mask(signal, in_sp500) in a backtest.
    let dir = tmp("pit");
    let from = 20240102;
    let to = 20240104;

    // Today's index = {AAA, BBB}. One change: on 2024-01-03 AAA joined and DDD
    // left. So before 01-03 the member set is {BBB, DDD}; on/after it is {AAA,BBB}.
    let current = r#"[{"symbol":"AAA"},{"symbol":"BBB"}]"#;
    let changes = r#"[{"date":"2024-01-03","symbol":"AAA","removedTicker":"DDD","reason":"x"}]"#;
    // All three names trade the full window (DDD keeps trading after leaving).
    let px = |a: f64, b: f64, c: f64| {
        format!(
            r#"[{{"date":"2024-01-02","adjClose":{a},"volume":100}},
                {{"date":"2024-01-03","adjClose":{b},"volume":100}},
                {{"date":"2024-01-04","adjClose":{c},"volume":100}}]"#
        )
    };
    let http = MockHttp::new()
        // historical-* must be routed before the current snapshot (substring match).
        .ok("historical-sp-500", changes)
        .ok("sp-500", current)
        .ok("symbol=AAA", &px(10.0, 11.0, 12.0))
        .ok("symbol=BBB", &px(10.0, 10.0, 10.0))
        .ok("symbol=DDD", &px(10.0, 9.0, 8.0));

    // Reconstruct + confirm the universe includes the name that left (DDD).
    let mut c = cfg();
    c.from = from;
    c.to = to;
    let membership = IndexMembership::fetch(&http, "KEY", Index::Sp500, &c).unwrap();
    let universe = membership.ever_members(from, to);
    assert_eq!(universe, vec!["AAA", "BBB", "DDD"]);

    // Sync those prices, then write the membership panel over the price calendar.
    sync(&http, "KEY", &universe, &dir, &c).unwrap();
    let (days, cols) = write_index_membership(&dir, &membership, from, to).unwrap();
    assert_eq!((days, cols), (3, 3));

    // The written panel is the expected per-day 0/1 grid.
    let src = LocalSource::new(&dir);
    let cols_v: Vec<String> = ["AAA", "BBB", "DDD"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let panel = load_combined_panel(&src, "in_sp500", &cols_v, from, to, PANELS_DIR)
        .unwrap()
        .unwrap();
    // 01-02: {BBB,DDD}; 01-03 & 01-04: {AAA,BBB}.
    assert_eq!(panel.data.row(0).to_vec(), vec![0.0, 1.0, 1.0]);
    assert_eq!(panel.data.row(1).to_vec(), vec![1.0, 1.0, 0.0]);
    assert_eq!(panel.data.row(2).to_vec(), vec![1.0, 1.0, 0.0]);

    // Consuming it: masking by in_sp500 gates holdings to members and changes the
    // result vs. the unmasked strategy (which keeps holding DDD after it left).
    let base = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":3}"#;
    let masked = format!(r#"{{"op":"Mask","of":{base},"by":{{"op":"Data","name":"in_sp500"}}}}"#);
    let unmasked = run_single(
        &dir,
        base,
        from,
        to,
        &BacktestConfig::default(),
        "close",
        None,
    )
    .unwrap();
    let gated = run_single(
        &dir,
        &masked,
        from,
        to,
        &BacktestConfig::default(),
        "close",
        None,
    )
    .unwrap();
    assert_eq!(gated.equity.len(), 3);
    assert!(gated.metrics.total_return.is_finite());
    assert!(
        (unmasked.metrics.total_return - gated.metrics.total_return).abs() > 1e-9,
        "membership gating had no effect: unmasked={} gated={}",
        unmasked.metrics.total_return,
        gated.metrics.total_return
    );
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
    // Trading days are 01-02, 01-03, 01-04. Fiscal period ends 2024-01-01 but the
    // report is only filed 2024-01-04, so the snapshot must become visible on the
    // filing day, NOT the (earlier) period-end — otherwise it would peek ahead.
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES) // matches any symbol's prices
        .ok(
            "ratios",
            r#"[{"date":"2024-01-01","priceToEarningsRatio":15.0,"netProfitMargin":0.2}]"#,
        )
        .ok(
            "key-metrics",
            r#"[{"date":"2024-01-01","marketCap":2.5e12}]"#,
        )
        .ok(
            "financial-growth",
            r#"[{"date":"2024-01-01","revenueGrowth":0.08}]"#,
        )
        .ok(
            "income-statement",
            r#"[{"date":"2024-01-01","filingDate":"2024-01-04","revenue":4.0e11}]"#,
        );
    let mut c = cfg();
    c.include_fundamentals = true;
    let summary = sync(&http, "KEY", &[String::from("AAPL")], &dir, &c).unwrap();
    assert_eq!(summary.fundamentals_written, 1);

    let bytes = LocalSource::new(&dir)
        .get("fundamentals/AAPL.csv.gz")
        .unwrap()
        .unwrap();
    // One row per trading day (3). The snapshot is visible from the filing day
    // (01-04), NOT the fiscal period-end (01-01) — no lookahead.
    let pe = parse_fundamentals(&bytes, "pe").unwrap();
    assert_eq!(pe.len(), 3);
    assert!(pe[0].1.is_nan()); // 01-02: period-end passed but not yet filed
    assert!(pe[1].1.is_nan()); // 01-03: still not filed
    assert_eq!(pe[2].1, 15.0); // 01-04: filed → visible
                               // Cross-endpoint fields merged in.
    assert_eq!(
        parse_fundamentals(&bytes, "market_cap").unwrap()[2].1,
        2.5e12
    );
    assert_eq!(
        parse_fundamentals(&bytes, "revenue_growth").unwrap()[2].1,
        0.08
    );
    // `revenue` is backfilled from the income statement.
    assert_eq!(parse_fundamentals(&bytes, "revenue").unwrap()[2].1, 4.0e11);
    // report_event flags the filing day, not the period-end.
    let ev = parse_fundamentals(&bytes, "report_event").unwrap();
    assert_eq!(ev[0].1, 0.0);
    assert_eq!(ev[1].1, 0.0);
    assert_eq!(ev[2].1, 1.0);
}

#[test]
fn snapshot_factor_panels_are_written() {
    let dir = tmp("snap");
    // Trading days 01-02, 01-03, 01-04; last close = 11.5 (AAPL_PRICES).
    // income-statement filing date 01-03 anchors the report-derived factors;
    // analyst/consensus are current (last day 01-04).
    let http = MockHttp::new()
        .ok("historical-price-eod", AAPL_PRICES)
        .ok(
            "financial-scores",
            r#"[{"piotroskiScore":7,"altmanZScore":3.5}]"#,
        )
        .ok(
            "income-statement",
            r#"[{"date":"2024-01-01","filingDate":"2024-01-03"}]"#,
        )
        .ok("key-metrics-ttm", r#"[{"freeCashFlowYieldTTM":0.05}]"#)
        .ok("price-target-consensus", r#"[{"targetConsensus":13.0}]"#)
        .ok("grades-summary", r#"[{"consensus":"Buy"}]"#);
    let mut c = cfg();
    c.include_snapshot_factors = true;
    let summary = sync(&http, "KEY", &[String::from("AAPL")], &dir, &c).unwrap();
    assert_eq!(summary.snapshot_factor_panels, 5);

    let src = LocalSource::new(&dir);
    let syms = vec!["AAPL".to_string()];
    let load = |name: &str| {
        load_combined_panel(&src, name, &syms, 20240101, 20240104, PANELS_DIR)
            .unwrap()
            .unwrap()
    };

    // piotroski_score: visible from the filing day (01-03) onward, NOT before.
    let p = load("piotroski_score");
    assert_eq!(p.dates, vec![20240103, 20240104]);
    assert_eq!(p.data[[0, 0]], 7.0);
    assert_eq!(p.data[[1, 0]], 7.0);
    // altman_z + fcf_yield share the filing-day anchor.
    assert_eq!(load("altman_z").data[[0, 0]], 3.5);
    assert_eq!(load("fcf_yield").data[[0, 0]], 0.05);

    // analyst_upside_pct is current: only the last day, (13-11.5)/11.5*100.
    let up = load("analyst_upside_pct");
    assert_eq!(up.dates, vec![20240104]);
    assert!((up.data[[0, 0]] - 13.043_478_260_869_565).abs() < 1e-9);
    // consensus_rating: "Buy" → 2, on the last day.
    let cr = load("consensus_rating");
    assert_eq!(cr.dates, vec![20240104]);
    assert_eq!(cr.data[[0, 0]], 2.0);
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
    let map = pomelo_data::industry::parse_industry_csv(&decode(&bytes));
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
