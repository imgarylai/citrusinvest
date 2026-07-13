//! Orchestrate multi-symbol Finnhub → data-layout sync.
//!
//! Skeleton (#225): validate inputs only. Price writes land in #226.

use std::path::Path;

use pomelo_data::{LocalSource, ObjectSink, ObjectSource};

use super::config::{SyncConfig, SyncSummary};
use super::http::{Fetcher, HttpClient};
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
/// **Skeleton (#225):** validates API key, date window, and symbols. Does not
/// fetch or write prices yet — each valid symbol is recorded as a structured
/// failure so the CLI can surface “not implemented” honestly.
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

    // Wire Fetcher now so throttle/retry paths stay compiled; #226 will fetch.
    let _fetcher = Fetcher::new(http, cfg);
    let _store = store;

    let mut summary = SyncSummary::default();
    let mut any_valid = false;
    for raw in symbols {
        let Some((layout, _fh)) = split_symbol(raw, &cfg.default_exchange) else {
            summary
                .failures
                .push((raw.clone(), "invalid symbol".into()));
            continue;
        };
        any_valid = true;
        summary.failures.push((
            layout,
            "price sync not implemented yet (pomelo-finnhub skeleton; see #226)".into(),
        ));
    }

    if !any_valid {
        return Err("no valid symbols after normalization".to_string());
    }

    eprintln!(
        "pomelo-finnhub skeleton: validated {} symbol(s); price/fundies/index \
         phases land under epic #210",
        symbols.len()
    );
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use std::time::Duration;

    struct NoHttp;
    impl HttpClient for NoHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Transport("skeleton".into()))
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig {
            from: 20200101,
            to: 20241231,
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    #[test]
    fn rejects_empty_key_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_fh_rej");
        assert!(sync(&NoHttp, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&NoHttp, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn rejects_inverted_dates() {
        let dir = std::env::temp_dir().join("pomelo_fh_dates");
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20200101;
        assert!(sync(&NoHttp, "tok", &["AAPL".into()], &dir, &c).is_err());
    }

    #[test]
    fn skeleton_records_not_implemented_per_symbol() {
        let dir = std::env::temp_dir().join("pomelo_fh_skel");
        let summary = sync(
            &NoHttp,
            "demo",
            &["AAPL".into(), "MSFT".into()],
            &dir,
            &cfg(),
        )
        .unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 2);
        assert!(summary.failures[0].1.contains("not implemented"));
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_fh_bad");
        let err = sync(&NoHttp, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }
}
