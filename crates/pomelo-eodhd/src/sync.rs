//! Orchestrate multi-symbol EODHD → data-layout sync.

use std::path::Path;

use pomelo_data::csv_io::write_series;
use pomelo_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::http::{Fetcher, HttpClient};
use super::price::{parse_price_rows, price_url, read_existing_prices};
use super::symbol::split_symbol;

/// EODHD API root (no trailing slash).
pub const EODHD_BASE: &str = "https://eodhd.com/api";

/// Sync `symbols` from EODHD into the local `out` tree — convenience wrapper
/// over [`sync_into`] for the common on-disk case.
pub fn sync<H: HttpClient>(
    http: &H,
    api_token: &str,
    symbols: &[String],
    out: &Path,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    sync_into(http, api_token, symbols, &LocalSource::new(out), cfg)
}

/// Storage-agnostic core: sync into any `store` (local disk or S3/R2).
///
/// Always fetches adjusted EOD prices → `prices/{SYM}.csv.gz`. Fundamentals /
/// industry flags are reserved for later phases (#195 / #196).
pub fn sync_into<H: HttpClient, S: ObjectSink + ObjectSource>(
    http: &H,
    api_token: &str,
    symbols: &[String],
    store: &S,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    if api_token.trim().is_empty() {
        return Err("empty API token".to_string());
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

    for raw in symbols {
        let Some((layout, eodhd)) = split_symbol(raw, &cfg.default_exchange) else {
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

        eprintln!("{layout}: fetching prices ({eodhd})…");
        let fetched = match fetcher
            .get_rows(&price_url(&eodhd, cfg, api_token))
            .map(|rows| parse_price_rows(&rows, cfg))
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

        if cfg.include_fundamentals {
            eprintln!("{layout}: fundamentals requested but not implemented yet (#196)");
        }
        if cfg.include_industry {
            eprintln!("{layout}: industry map requested but not implemented yet (#195)");
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
    use crate::http::HttpError;
    use pomelo_data::csv_io::parse_series;
    use pomelo_data::{Field, LocalSource};
    use std::cell::RefCell;
    use std::time::Duration;

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

        fn seq(pat: &str, bodies: Vec<Result<Vec<u8>, HttpError>>) -> Self {
            MockHttp {
                routes: vec![(pat.to_string(), RefCell::new(bodies))],
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

    const AAPL_EOD: &str = r#"[
        {"date":"2024-01-04","open":11.0,"high":12.0,"low":10.5,"close":11.5,"adjusted_close":11.5,"volume":1200},
        {"date":"2024-01-03","open":10.1,"high":11.5,"low":9.8,"close":10.8,"adjusted_close":10.8,"volume":1100},
        {"date":"2024-01-02","open":9.5,"high":11.0,"low":9.0,"close":10.0,"adjusted_close":10.0,"volume":1000}
    ]"#;

    #[test]
    fn rejects_empty_token_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_rej");
        let http = MockHttp::ok("unused", "[]");
        assert!(sync(&http, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&http, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn syncs_prices_to_layout() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp::ok("AAPL.US", AAPL_EOD);
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 3);
        assert!(summary.failures.is_empty());

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
    }

    #[test]
    fn resume_skips_existing() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_resume");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp::ok("AAPL.US", AAPL_EOD);
        sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.symbols_skipped, 1);
    }

    #[test]
    fn rejects_inverted_dates_and_empty_exchange() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_dates");
        let http = MockHttp::ok("unused", "[]");
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20240101;
        assert!(sync(&http, "tok", &["AAPL".into()], &dir, &c).is_err());
        c = cfg();
        c.default_exchange = String::new();
        assert!(sync(&http, "tok", &["AAPL".into()], &dir, &c).is_err());
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_badsym");
        let http = MockHttp::ok("unused", "[]");
        let err = sync(&http, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }

    #[test]
    fn fetch_failure_and_empty_rows_recorded() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_fail");
        let _ = std::fs::remove_dir_all(&dir);
        // MSFT 404; AAPL empty array
        let http = MockHttp {
            routes: vec![("AAPL.US".into(), RefCell::new(vec![Ok(b"[]".to_vec())]))],
        };
        let summary = sync(&http, "tok", &["AAPL".into(), "MSFT".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 2);
    }

    #[test]
    fn append_merges_existing_history() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_append");
        let _ = std::fs::remove_dir_all(&dir);
        let day1 = r#"[{"date":"2024-01-02","open":9.5,"high":11.0,"low":9.0,"close":10.0,"adjusted_close":10.0,"volume":1000}]"#;
        let day2 = r#"[{"date":"2024-01-03","open":10.1,"high":11.5,"low":9.8,"close":10.8,"adjusted_close":10.8,"volume":1100}]"#;
        let http = MockHttp::ok("AAPL.US", day1);
        let mut c = cfg();
        c.from = 20240102;
        c.to = 20240102;
        sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();

        let http2 = MockHttp::ok("AAPL.US", day2);
        c.from = 20240103;
        c.to = 20240103;
        c.mode = WriteMode::Append;
        let summary = sync(&http2, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 2);

        let bytes = LocalSource::new(&dir)
            .get("prices/AAPL.csv.gz")
            .unwrap()
            .unwrap();
        assert_eq!(
            parse_series(&bytes, Field::AdjClose).unwrap(),
            vec![(20240102, 10.0), (20240103, 10.8)]
        );
    }

    #[test]
    fn include_flags_log_but_still_write_prices() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_flags");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp::ok("AAPL.US", AAPL_EOD);
        let mut c = cfg();
        c.include_fundamentals = true;
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.fundamentals_written, 0);
    }

    #[test]
    fn mixed_valid_and_invalid_symbols() {
        let dir = std::env::temp_dir().join("pomelo_eodhd_mixed");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp::ok("AAPL.US", AAPL_EOD);
        let summary = sync(&http, "demo", &[".".into(), "AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.failures.len(), 1);
    }

    #[test]
    fn seq_helper_unused_path_covered_via_404() {
        // Document MockHttp::seq exists for future retry tests at sync layer.
        let _ = MockHttp::seq("X", vec![Err(HttpError::Status(500))]);
    }
}
