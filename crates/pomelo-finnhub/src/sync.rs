//! Orchestrate multi-symbol Finnhub → data-layout sync.
//!
//! Prices (#226): `/stock/candle` (resolution=D, adjusted) → `prices/{SYM}.csv.gz`
//! with resume/append modes. Industry / fundamentals / index / snapshot land in
//! later epic #210 phases; their config flags are accepted but inert for now.

use std::path::Path;

use pomelo_data::csv_io::write_series;
use pomelo_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::http::{Fetcher, HttpClient};
use super::price::{parse_price_payload, price_url, read_existing_prices};
use super::symbol::split_symbol;

/// Finnhub API root (no trailing slash).
pub const FINNHUB_BASE: &str = "https://finnhub.io/api/v1";

/// Sync `symbols` from Finnhub into the local `out` tree — convenience wrapper
/// over [`sync_into`] for the common on-disk case.
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
/// Fetches adjusted daily candles → `prices/{SYM}.csv.gz` for each symbol.
/// `WriteMode::Resume` skips symbols that already have a price file;
/// `WriteMode::Append` merges the fetched window into the existing file. The
/// `include_*` flags are reserved for later #210 phases and have no effect yet.
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

    for raw in symbols {
        let Some((layout, fh)) = split_symbol(raw, &cfg.default_exchange) else {
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

        eprintln!("{layout}: fetching candles ({fh})…");
        let fetched = match fetcher
            .get_json(&price_url(&fh, cfg, api_key))
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

    // 2024-01-02..04 at 00:00 UTC.
    const T2: i64 = 1_704_153_600;
    const T3: i64 = T2 + 86_400;
    const T4: i64 = T3 + 86_400;

    struct OkHttp {
        body: Vec<u8>,
    }
    impl HttpClient for OkHttp {
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

    fn candle(ts: &[i64]) -> Vec<u8> {
        let n = ts.len();
        let arr = |base: f64| -> String {
            (0..n)
                .map(|i| (base + i as f64).to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let t = ts
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"s":"ok","t":[{t}],"o":[{}],"h":[{}],"l":[{}],"c":[{}],"v":[{}]}}"#,
            arr(9.0),
            arr(11.0),
            arr(8.0),
            arr(10.0),
            arr(1000.0),
        )
        .into_bytes()
    }

    #[test]
    fn rejects_empty_key_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_fh_rej");
        let http = OkHttp {
            body: candle(&[T2]),
        };
        assert!(sync(&http, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&http, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn syncs_prices_to_layout() {
        let dir = std::env::temp_dir().join("pomelo_fh_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: candle(&[T2, T3, T4]),
        };
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 3);
        assert!(summary.failures.is_empty());

        let src = LocalSource::new(&dir);
        let bytes = src.get("prices/AAPL.csv.gz").unwrap().unwrap();
        assert_eq!(
            parse_series(&bytes, Field::AdjClose).unwrap(),
            vec![(20240102, 10.0), (20240103, 11.0), (20240104, 12.0)]
        );
    }

    #[test]
    fn resume_skips_existing() {
        let dir = std::env::temp_dir().join("pomelo_fh_resume");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: candle(&[T2, T3, T4]),
        };
        sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.symbols_skipped, 1);
    }

    #[test]
    fn append_merges_existing_history() {
        let dir = std::env::temp_dir().join("pomelo_fh_append");
        let _ = std::fs::remove_dir_all(&dir);

        let mut c = cfg();
        c.from = 20240102;
        c.to = 20240102;
        let http = OkHttp {
            body: candle(&[T2]),
        };
        sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();

        c.from = 20240103;
        c.to = 20240103;
        c.mode = WriteMode::Append;
        let http2 = OkHttp {
            body: candle(&[T3]),
        };
        let summary = sync(&http2, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 2);
    }

    #[test]
    fn fetch_failure_recorded() {
        let dir = std::env::temp_dir().join("pomelo_fh_fail");
        let _ = std::fs::remove_dir_all(&dir);
        let summary = sync(&FailHttp, "tok", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
    }

    #[test]
    fn no_data_recorded_as_failure() {
        let dir = std::env::temp_dir().join("pomelo_fh_nodata");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: br#"{"s":"no_data"}"#.to_vec(),
        };
        let summary = sync(&http, "tok", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.failures[0].1.contains("no price rows"));
    }

    #[test]
    fn rejects_inverted_dates() {
        let dir = std::env::temp_dir().join("pomelo_fh_dates");
        let http = OkHttp {
            body: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20200101;
        assert!(sync(&http, "tok", &["AAPL".into()], &dir, &c).is_err());
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_fh_bad");
        let http = OkHttp {
            body: b"{}".to_vec(),
        };
        let err = sync(&http, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }
}
