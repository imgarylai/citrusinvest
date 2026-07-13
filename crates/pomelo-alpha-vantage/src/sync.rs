//! Orchestrate multi-symbol Alpha Vantage → data-layout sync.

use std::path::Path;

use pomelo_data::csv_io::write_series;
use pomelo_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::http::{Fetcher, HttpClient};
use super::price::{parse_price_payload, price_url, read_existing_prices};
use super::symbol::split_symbol;

/// Alpha Vantage query API root (no trailing slash).
pub const ALPHA_VANTAGE_BASE: &str = "https://www.alphavantage.co/query";

/// Sync `symbols` from Alpha Vantage into the local `out` tree — convenience
/// wrapper over [`sync_into`] for the common on-disk case.
pub fn sync<H: HttpClient>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    out: &Path,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    sync_into(http, api_key, symbols, &LocalSource::new(out), cfg)
}

/// Storage-agnostic core: sync into any `store` (local disk or S3/R2).
///
/// Fetches `TIME_SERIES_DAILY_ADJUSTED` → `prices/{SYM}.csv.gz` with adj OHLC
/// scale. Fundamentals / industry / snapshot flags are reserved for later phases.
pub fn sync_into<H: HttpClient, S: ObjectSink + ObjectSource>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    store: &S,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    if symbols.is_empty() {
        return Err("no symbols requested".to_string());
    }
    if cfg.from > cfg.to {
        return Err(format!("from ({}) is after to ({})", cfg.from, cfg.to));
    }
    if cfg.default_exchange.trim().is_empty() {
        return Err("default_exchange is empty".to_string());
    }

    let fetcher = Fetcher::new(http, cfg);
    let mut summary = SyncSummary::default();
    let mut any_valid = false;

    if cfg.include_fundamentals {
        eprintln!("note: --include-fundamentals is not implemented yet (#216); ignoring");
    }
    if cfg.include_industry {
        eprintln!("note: --include-industry is not implemented yet (#215); ignoring");
    }
    if cfg.include_snapshot_factors {
        eprintln!("note: --include-snapshot-factors is not implemented yet (#218); ignoring");
    }

    for raw in symbols {
        let Some((layout, av)) = split_symbol(raw, &cfg.default_exchange) else {
            summary
                .failures
                .push((raw.clone(), "invalid symbol".into()));
            continue;
        };
        any_valid = true;

        let price_key = format!("{PRICES_DIR}/{layout}.csv.gz");
        if cfg.mode == WriteMode::Resume && store.get(&price_key).ok().flatten().is_some() {
            eprintln!("{layout}: already present, skipping (resume)");
            summary.symbols_skipped += 1;
            continue;
        }

        eprintln!("{layout}: fetching prices ({av})…");
        let fetched = match fetcher
            .get_json(&price_url(&av, api_key))
            .and_then(|v| parse_price_payload(&v, cfg))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{layout}: price fetch failed: {e}");
                summary.failures.push((layout, e));
                continue;
            }
        };
        if fetched.is_empty() {
            let msg = "no price rows in range".to_string();
            eprintln!("{layout}: {msg}");
            summary.failures.push((layout, msg));
            continue;
        }

        let rows = if cfg.mode == WriteMode::Append {
            let mut by_day = read_existing_prices(store, &layout);
            for r in fetched {
                by_day.insert(r.day, r);
            }
            by_day.into_values().collect()
        } else {
            fetched
        };

        match write_series(&rows).map_err(|e| e.to_string()) {
            Ok(bytes) => {
                if let Err(e) = store.put(&price_key, &bytes) {
                    let e = e.to_string();
                    eprintln!("{layout}: write failed: {e}");
                    summary.failures.push((layout, e));
                    continue;
                }
            }
            Err(e) => {
                eprintln!("{layout}: encode failed: {e}");
                summary.failures.push((layout, e));
                continue;
            }
        }

        summary.symbols_written += 1;
        summary.price_rows += rows.len();
        eprintln!("{layout}: wrote {} price rows", rows.len());
    }

    if !any_valid {
        return Err("no valid symbols after normalization".to_string());
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use pomelo_data::csv_io::parse_series;
    use pomelo_data::{Field, LocalSource};
    use std::time::Duration;

    struct MockHttp {
        body: Vec<u8>,
    }
    impl HttpClient for MockHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.body.clone())
        }
    }

    struct FailHttp;
    impl HttpClient for FailHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Status(500))
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig {
            from: 20240102,
            to: 20240104,
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    const IBM_TS: &str = r#"{
        "Time Series (Daily)": {
            "2024-01-04": {
                "1. open": "11.0", "2. high": "12.0", "3. low": "10.5",
                "4. close": "11.5", "5. adjusted close": "11.5", "6. volume": "1200"
            },
            "2024-01-03": {
                "1. open": "10.1", "2. high": "11.5", "3. low": "9.8",
                "4. close": "10.8", "5. adjusted close": "10.8", "6. volume": "1100"
            },
            "2024-01-02": {
                "1. open": "9.5", "2. high": "11.0", "3. low": "9.0",
                "4. close": "10.0", "5. adjusted close": "10.0", "6. volume": "1000"
            }
        }
    }"#;

    #[test]
    fn rejects_empty_key_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_av_rej");
        let http = MockHttp {
            body: IBM_TS.as_bytes().to_vec(),
        };
        assert!(sync(&http, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&http, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn syncs_prices_to_layout() {
        let dir = std::env::temp_dir().join("pomelo_av_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            body: IBM_TS.as_bytes().to_vec(),
        };
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 3);
        assert!(summary.failures.is_empty());

        let src = LocalSource::new(&dir);
        let bytes = src.get("prices/IBM.csv.gz").unwrap().unwrap();
        assert_eq!(
            parse_series(&bytes, Field::AdjClose).unwrap(),
            vec![(20240102, 10.0), (20240103, 10.8), (20240104, 11.5)]
        );
    }

    #[test]
    fn resume_skips_existing() {
        let dir = std::env::temp_dir().join("pomelo_av_resume");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            body: IBM_TS.as_bytes().to_vec(),
        };
        sync(&http, "demo", &["IBM".into()], &dir, &cfg()).unwrap();
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.symbols_skipped, 1);
    }

    #[test]
    fn append_merges_existing_history() {
        let dir = std::env::temp_dir().join("pomelo_av_append");
        let _ = std::fs::remove_dir_all(&dir);
        let day1 = r#"{"Time Series (Daily)":{
            "2024-01-02":{"1. open":"9.5","2. high":"11.0","3. low":"9.0","4. close":"10.0","5. adjusted close":"10.0","6. volume":"1000"}
        }}"#;
        let day2 = r#"{"Time Series (Daily)":{
            "2024-01-03":{"1. open":"10.1","2. high":"11.5","3. low":"9.8","4. close":"10.8","5. adjusted close":"10.8","6. volume":"1100"}
        }}"#;
        let http = MockHttp {
            body: day1.as_bytes().to_vec(),
        };
        let mut c = cfg();
        c.from = 20240102;
        c.to = 20240102;
        sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();

        let http2 = MockHttp {
            body: day2.as_bytes().to_vec(),
        };
        c.from = 20240103;
        c.to = 20240103;
        c.mode = WriteMode::Append;
        let summary = sync(&http2, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 2);
    }

    #[test]
    fn fetch_failure_recorded() {
        let dir = std::env::temp_dir().join("pomelo_av_fail");
        let _ = std::fs::remove_dir_all(&dir);
        let summary = sync(&FailHttp, "tok", &["IBM".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
    }

    #[test]
    fn rejects_inverted_dates() {
        let dir = std::env::temp_dir().join("pomelo_av_dates");
        let http = MockHttp {
            body: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20200101;
        assert!(sync(&http, "tok", &["IBM".into()], &dir, &c).is_err());
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_av_bad");
        let http = MockHttp {
            body: b"{}".to_vec(),
        };
        let err = sync(&http, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }
}
