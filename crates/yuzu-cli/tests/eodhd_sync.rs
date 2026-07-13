//! End-to-end tests for `pomelo-eodhd` / `yuzu-cli eodhd` against a mock
//! [`HttpClient`] — no live token, no network.

use std::cell::RefCell;
use std::time::Duration;

use pomelo_data::csv_io::parse_series;
use pomelo_data::fundamentals::parse_fundamentals;
use pomelo_data::{Field, LocalSource, ObjectSource};
use yuzu_cli::eodhd::{
    fetch_delisted, sync, HttpClient, HttpError, SyncConfig, WriteMode, INDUSTRY_KEY,
};
use yuzu_cli::run_single;

struct MockHttp {
    routes: Vec<(String, RefCell<Vec<Result<Vec<u8>, HttpError>>>)>,
}

impl MockHttp {
    fn ok(pat: &str, body: &str) -> Self {
        MockHttp {
            routes: vec![(
                pat.to_string(),
                RefCell::new(vec![Ok(body.as_bytes().to_vec())]),
            )],
        }
    }

    fn multi(routes: Vec<(&str, &str)>) -> Self {
        MockHttp {
            routes: routes
                .into_iter()
                .map(|(pat, body)| {
                    (
                        pat.to_string(),
                        RefCell::new(vec![Ok(body.as_bytes().to_vec())]),
                    )
                })
                .collect(),
        }
    }
}

impl HttpClient for MockHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
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
        rate_limit_per_min: 0,
        max_retries: 2,
        backoff_base: Duration::ZERO,
        mode: WriteMode::Overwrite,
        ..SyncConfig::default()
    }
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yuzu_cli_eodhd_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const AAPL: &str = r#"[
    {"date":"2024-01-04","open":11.0,"high":12.0,"low":10.5,"close":11.5,"adjusted_close":11.5,"volume":1200},
    {"date":"2024-01-03","open":10.1,"high":11.5,"low":9.8,"close":10.8,"adjusted_close":10.8,"volume":1100},
    {"date":"2024-01-02","open":9.5,"high":11.0,"low":9.0,"close":10.0,"adjusted_close":10.0,"volume":1000}
]"#;

const MSFT: &str = r#"[
    {"date":"2024-01-02","open":20.0,"high":21.0,"low":19.0,"close":20.0,"adjusted_close":20.0,"volume":500},
    {"date":"2024-01-03","open":20.0,"high":21.0,"low":19.0,"close":21.0,"adjusted_close":21.0,"volume":600},
    {"date":"2024-01-04","open":21.0,"high":22.0,"low":20.0,"close":22.0,"adjusted_close":22.0,"volume":700}
]"#;

#[test]
fn syncs_prices_and_tree_backtests() {
    let dir = tmp("prices");
    let http = MockHttp::multi(vec![("AAPL.US", AAPL), ("MSFT.US", MSFT)]);
    let syms = vec!["AAPL".to_string(), "MSFT".to_string()];
    let summary = sync(&http, "KEY", &syms, &dir, &cfg()).unwrap();
    assert_eq!(summary.symbols_written, 2);
    assert_eq!(summary.price_rows, 6);

    let src = LocalSource::new(&dir);
    let bytes = src.get("prices/AAPL.csv.gz").unwrap().unwrap();
    assert_eq!(
        parse_series(&bytes, Field::AdjClose).unwrap(),
        vec![(20240102, 10.0), (20240103, 10.8), (20240104, 11.5)]
    );

    let spec = r#"{"op":"IsLargest","of":{"op":"Data","name":"close"},"n":1}"#;
    let report = run_single(&dir, spec, 20240102, 20240104, &Default::default(), "close").unwrap();
    assert_eq!(report.equity.len(), 3);
    assert!(report.metrics.total_return.is_finite());
}

#[test]
fn scales_split_adjusted_bars() {
    // 2:1 split-style: close=200, adj=100 → factor 0.5
    let body = r#"[{"date":"2024-01-02","open":202.0,"high":210.0,"low":198.0,"close":200.0,"adjusted_close":100.0,"volume":1000}]"#;
    let dir = tmp("scale");
    let http = MockHttp::ok("AAPL.US", body);
    let mut c = cfg();
    c.from = 20240102;
    c.to = 20240102;
    sync(&http, "KEY", &["AAPL".into()], &dir, &c).unwrap();
    let bytes = LocalSource::new(&dir)
        .get("prices/AAPL.csv.gz")
        .unwrap()
        .unwrap();
    let open = parse_series(&bytes, Field::AdjOpen).unwrap()[0].1;
    let close = parse_series(&bytes, Field::AdjClose).unwrap()[0].1;
    assert!((close - 100.0).abs() < 1e-9);
    assert!((open - 101.0).abs() < 1e-9);
}

#[test]
fn densifies_fundamentals_from_statements() {
    let eod = r#"[
        {"date":"2024-01-02","open":10.0,"high":10.0,"low":10.0,"close":10.0,"adjusted_close":10.0,"volume":1},
        {"date":"2024-01-03","open":10.0,"high":10.0,"low":10.0,"close":10.0,"adjusted_close":10.0,"volume":1},
        {"date":"2024-01-04","open":10.0,"high":10.0,"low":10.0,"close":10.0,"adjusted_close":10.0,"volume":1}
    ]"#;
    let fund = r#"{
      "Financials::Income_Statement::yearly": {
        "2023-09-30": {
          "date":"2023-09-30","filing_date":"2023-11-03",
          "totalRevenue":100.0,"grossProfit":40.0,"netIncome":20.0,"operatingIncome":30.0
        },
        "2024-09-30": {
          "date":"2024-09-30","filing_date":"2024-01-03",
          "totalRevenue":110.0,"grossProfit":44.0,"netIncome":22.0,"operatingIncome":33.0
        }
      },
      "Financials::Balance_Sheet::yearly": {
        "2023-09-30": {
          "date":"2023-09-30","filing_date":"2023-11-03",
          "totalAssets":200.0,"totalLiab":80.0,"totalStockholderEquity":120.0,"netReceivables":10.0,"shortLongTermDebtTotal":50.0
        },
        "2024-09-30": {
          "date":"2024-09-30","filing_date":"2024-01-03",
          "totalAssets":220.0,"totalLiab":88.0,"totalStockholderEquity":132.0,"netReceivables":11.0,"shortLongTermDebtTotal":55.0
        }
      }
    }"#;
    let http = MockHttp::multi(vec![("/eod/AAPL.US", eod), ("fundamentals/AAPL.US", fund)]);
    let dir = tmp("fundies");
    let mut c = cfg();
    c.include_fundamentals = true;
    let summary = sync(&http, "KEY", &["AAPL".into()], &dir, &c).unwrap();
    assert_eq!(summary.fundamentals_written, 1);
    let bytes = LocalSource::new(&dir)
        .get("fundamentals/AAPL.csv.gz")
        .unwrap()
        .unwrap();
    let rev = parse_fundamentals(&bytes, "revenue").unwrap();
    // 2023 filing (2023-11-03) is before the price window → day1 already has rev 100
    assert!((rev[0].1 - 100.0).abs() < 1e-6);
    // On 2024-01-03 second filing becomes visible → rev 110
    assert!((rev[1].1 - 110.0).abs() < 1e-6);
    assert!((rev[2].1 - 110.0).abs() < 1e-6);
    let events = parse_fundamentals(&bytes, "report_event").unwrap();
    assert_eq!(events[0].1, 1.0); // first day applies outstanding 2023 snapshot
    assert_eq!(events[1].1, 1.0); // 2024 snapshot lands
    assert_eq!(events[2].1, 0.0);
}

#[test]
fn industry_map_and_delisted_fetch() {
    let eod = r#"[{"date":"2024-01-02","open":10.0,"high":10.0,"low":10.0,"close":10.0,"adjusted_close":10.0,"volume":1}]"#;
    let prof = r#"{"General::Sector":"Technology","General::Industry":"Software","Highlights::MarketCapitalization":9e9}"#;
    let delisted = r#"[
        {"Code":"DEAD","Exchange":"US","Type":"Common Stock"},
        {"Code":"AAPL","Exchange":"US","Type":"Common Stock"}
    ]"#;
    let http = MockHttp::multi(vec![
        ("/eod/AAPL.US", eod),
        ("fundamentals/AAPL.US", prof),
        ("exchange-symbol-list/US", delisted),
    ]);
    let dir = tmp("industry");
    let mut c = cfg();
    c.from = 20240102;
    c.to = 20240102;
    c.include_industry = true;
    let summary = sync(&http, "KEY", &["AAPL".into()], &dir, &c).unwrap();
    assert!(summary.industry_written);
    let gz = LocalSource::new(&dir).get(INDUSTRY_KEY).unwrap().unwrap();
    // gunzip via flate2 not required — content is gzip; just ensure object exists
    assert!(!gz.is_empty());

    let d = fetch_delisted(&http, "KEY", &c, "US").unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].symbol, "AAPL");
    assert_eq!(d[1].symbol, "DEAD");
}
