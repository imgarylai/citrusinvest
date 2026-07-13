//! Orchestrate multi-symbol EODHD → data-layout sync.
//!
//! Skeleton (#193): validate inputs and return a summary. Price/fundamentals
//! writers land in later epic phases (#194+).

use std::path::Path;

use pomelo_data::{LocalSource, ObjectSink, ObjectSource};

use super::config::{SyncConfig, SyncSummary};
use super::http::HttpClient;
use super::symbol::split_symbol;

/// EODHD API root (no trailing slash). Public for docs/tests; used by fetchers in #194+.
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
/// **Skeleton:** validates token / dates / symbols, normalizes tickers, and
/// returns an empty success summary. Does **not** call the network yet — price
/// fetch lands in #194. Callers (CLI) should treat `symbols_written == 0` as
/// expected for this phase.
pub fn sync_into<H: HttpClient, S: ObjectSink + ObjectSource>(
    _http: &H,
    api_token: &str,
    symbols: &[String],
    _store: &S,
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

    let mut summary = SyncSummary::default();
    let mut resolved = 0usize;
    for raw in symbols {
        match split_symbol(raw, &cfg.default_exchange) {
            Some((layout, eodhd)) => {
                resolved += 1;
                eprintln!(
                    "{layout}: skeleton only — would fetch {eodhd} \
                     (prices → #194; fundies/industry later)"
                );
            }
            None => {
                summary
                    .failures
                    .push((raw.clone(), "invalid symbol".into()));
            }
        }
    }
    if resolved == 0 {
        return Err("no valid symbols after normalization".to_string());
    }

    eprintln!(
        "pomelo-eodhd skeleton: validated {resolved} symbol(s), \
         from={} to={} — no files written yet (epic #192 / phase #194)",
        cfg.from, cfg.to
    );
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpError;
    use std::path::PathBuf;

    struct NoHttp;
    impl HttpClient for NoHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Transport("no network in tests".into()))
        }
    }

    #[test]
    fn rejects_empty_token_and_symbols() {
        let cfg = SyncConfig::default();
        let dir = PathBuf::from("/tmp/pomelo-eodhd-unused");
        assert!(sync(&NoHttp, "", &["AAPL".into()], &dir, &cfg).is_err());
        assert!(sync(&NoHttp, "tok", &[], &dir, &cfg).is_err());
    }

    #[test]
    fn rejects_inverted_dates() {
        let mut cfg = SyncConfig::default();
        cfg.from = 20250101;
        cfg.to = 20240101;
        let dir = PathBuf::from("/tmp/pomelo-eodhd-unused");
        assert!(sync(&NoHttp, "tok", &["AAPL".into()], &dir, &cfg).is_err());
    }

    #[test]
    fn skeleton_ok_writes_nothing() {
        let cfg = SyncConfig {
            from: 20200101,
            to: 20201231,
            ..SyncConfig::default()
        };
        let dir = std::env::temp_dir().join("pomelo_eodhd_skeleton_ok");
        let _ = std::fs::create_dir_all(&dir);
        let summary = sync(&NoHttp, "demo", &["AAPL".into(), "MSFT.US".into()], &dir, &cfg)
            .expect("skeleton ok");
        assert_eq!(summary.symbols_written, 0);
        assert!(summary.failures.is_empty());
    }
}
