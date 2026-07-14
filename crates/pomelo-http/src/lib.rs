//! Shared HTTP plumbing for the `pomelo-*` bring-your-own-key sync crates.
//!
//! Every vendor adapter (`pomelo-fmp`, `pomelo-eodhd`, `pomelo-alpha-vantage`,
//! `pomelo-finnhub`) needs the same primitives: a mockable [`HttpClient`], a
//! [`Fetcher`] that adds rate-limit throttle + bounded exponential-backoff
//! retry, token [`redact`]ion for logs, and a [`WriteMode`]. Those are identical
//! by copy across the adapters, so they live here once (conventions: citrusquant
//! issue [#211](https://github.com/citrusquant/citrusquant/issues/211)).
//!
//! **No vendor logic.** JSON field maps, densify formulas, symbol suffix rules,
//! and rating mappers stay in the vendor crate — forcing one of those here would
//! create false parity. (List-endpoint error envelopes are similar enough to
//! share: their key shapes are a common set and the label is cosmetic, so
//! [`Fetcher::get_rows`] handles them here rather than per vendor.)
//!
//! The [`Fetcher`] reads its throttle/retry knobs through the [`RetrySettings`]
//! trait, which each vendor's `SyncConfig` implements, so no vendor `SyncConfig`
//! type leaks in here.

use std::cell::Cell;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How an already-present symbol tree is treated by a sync run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteMode {
    /// Overwrite each symbol's files with the freshly fetched window (default).
    Overwrite,
    /// Merge fetched rows into existing files (extend an existing tree).
    Append,
    /// Skip any symbol that already has a `prices/{SYM}.csv.gz`.
    Resume,
}

/// A classified HTTP failure so the retry loop knows whether to back off.
#[derive(Debug, Clone)]
pub enum HttpError {
    /// A non-success HTTP status (e.g. 401, 404, 429, 503).
    Status(u16),
    /// A transport-level failure (DNS, TLS, connection reset, timeout, …).
    Transport(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Status(code) => write!(f, "HTTP {code}"),
            HttpError::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl HttpError {
    /// Whether retrying (after a backoff) could plausibly succeed.
    pub fn retryable(&self) -> bool {
        match self {
            HttpError::Transport(_) => true,
            HttpError::Status(code) => *code == 429 || (500..600).contains(code),
        }
    }
}

/// Minimal blocking HTTP GET, abstracted so sync logic is tested with a mock.
pub trait HttpClient {
    /// GET `url`, returning the response body on a 2xx status.
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
}

/// Throttle/retry knobs a [`Fetcher`] needs, supplied by each vendor `SyncConfig`.
pub trait RetrySettings {
    /// Max requests per minute (`0` = no throttle).
    fn rate_limit_per_min(&self) -> u32;
    /// Retries per request on a retryable error before giving up.
    fn max_retries(&self) -> u32;
    /// Base backoff; the Nth retry waits `base * 2^(N-1)`.
    fn backoff_base(&self) -> Duration;
}

/// The real ureq-backed client — only with the `ureq` feature.
#[cfg(feature = "ureq")]
pub struct UreqClient;

#[cfg(feature = "ureq")]
impl UreqClient {
    pub fn new() -> Self {
        UreqClient
    }
}

#[cfg(feature = "ureq")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "ureq")]
impl HttpClient for UreqClient {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        match ureq::get(url).call() {
            Ok(resp) => resp
                .into_body()
                .with_config()
                .limit(256 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| HttpError::Transport(e.to_string())),
            Err(ureq::Error::StatusCode(code)) => Err(HttpError::Status(code)),
            Err(e) => Err(HttpError::Transport(e.to_string())),
        }
    }
}

/// Redact vendor token query params for stderr logs (covers `token`, `apikey`,
/// `api_key`, `api_token`).
pub fn redact(url: &str) -> String {
    for key in ["api_token=", "token=", "apikey=", "api_key="] {
        if let Some(i) = url.find(key) {
            let start = i + key.len();
            let end = url[start..]
                .find('&')
                .map(|j| start + j)
                .unwrap_or(url.len());
            return format!("{}***{}", &url[..start], &url[end..]);
        }
    }
    url.to_string()
}

/// Wraps an [`HttpClient`] with a rate-limit throttle and retry/backoff loop.
///
/// `C` supplies the knobs via [`RetrySettings`] — typically a vendor `SyncConfig`.
pub struct Fetcher<'a, H: HttpClient, C: RetrySettings> {
    http: &'a H,
    cfg: &'a C,
    last_request: Cell<Option<Instant>>,
}

impl<'a, H: HttpClient, C: RetrySettings> Fetcher<'a, H, C> {
    pub fn new(http: &'a H, cfg: &'a C) -> Self {
        Fetcher {
            http,
            cfg,
            last_request: Cell::new(None),
        }
    }

    fn throttle(&self) {
        let rpm = self.cfg.rate_limit_per_min();
        if rpm == 0 {
            return;
        }
        let min_interval = Duration::from_secs_f64(60.0 / rpm as f64);
        if let Some(prev) = self.last_request.get() {
            let elapsed = prev.elapsed();
            if elapsed < min_interval {
                std::thread::sleep(min_interval - elapsed);
            }
        }
        self.last_request.set(Some(Instant::now()));
    }

    /// GET with throttle + bounded exponential backoff. On success returns the
    /// body; on terminal failure a message with the token redacted.
    pub fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let max_retries = self.cfg.max_retries();
        let mut attempt = 0u32;
        loop {
            self.throttle();
            match self.http.get(url) {
                Ok(body) => return Ok(body),
                Err(e) if e.retryable() && attempt < max_retries => {
                    let wait = self.cfg.backoff_base() * 2u32.pow(attempt.min(16));
                    eprintln!(
                        "  retry {}/{} after {}: {} ({:?})",
                        attempt + 1,
                        max_retries,
                        e,
                        redact(url),
                        wait
                    );
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                    }
                    attempt += 1;
                }
                Err(e) => return Err(format!("{e} for {}", redact(url))),
            }
        }
    }

    /// GET and parse any JSON value.
    pub fn get_json(&self, url: &str) -> Result<Value, String> {
        let body = self.get(url)?;
        serde_json::from_slice(&body).map_err(|e| format!("bad JSON from {}: {e}", redact(url)))
    }

    /// GET and parse a JSON array of row objects (list / EOD endpoints).
    ///
    /// Vendor list endpoints return an array on success and an error **object**
    /// on failure; surface that object's message (checking the common key shapes
    /// across vendors — `message` / `error` / `Error Message`) instead of
    /// silently yielding no rows.
    pub fn get_rows(&self, url: &str) -> Result<Vec<Value>, String> {
        match self.get_json(url)? {
            Value::Array(rows) => Ok(rows),
            Value::Object(map) => {
                if let Some(msg) = ["message", "error", "Error Message"]
                    .iter()
                    .find_map(|k| map.get(*k).and_then(Value::as_str))
                {
                    Err(format!("API error: {msg}"))
                } else {
                    Err(format!("expected a JSON array from {}", redact(url)))
                }
            }
            _ => Err(format!("expected a JSON array from {}", redact(url))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Minimal `RetrySettings` for tests.
    struct Policy {
        rpm: u32,
        retries: u32,
        backoff: Duration,
    }
    impl RetrySettings for Policy {
        fn rate_limit_per_min(&self) -> u32 {
            self.rpm
        }
        fn max_retries(&self) -> u32 {
            self.retries
        }
        fn backoff_base(&self) -> Duration {
            self.backoff
        }
    }
    fn policy(retries: u32) -> Policy {
        Policy {
            rpm: 0,
            retries,
            backoff: Duration::ZERO,
        }
    }

    #[test]
    fn redacts_all_token_shapes() {
        assert_eq!(
            redact("https://x/candle?symbol=AAPL&token=SECRET"),
            "https://x/candle?symbol=AAPL&token=***"
        );
        assert_eq!(
            redact("https://x?apikey=SECRET&r=D"),
            "https://x?apikey=***&r=D"
        );
        assert_eq!(
            redact("https://eodhd.com/eod/AAPL?api_token=SECRET&fmt=json"),
            "https://eodhd.com/eod/AAPL?api_token=***&fmt=json"
        );
        assert_eq!(redact("https://x/no-token"), "https://x/no-token");
    }

    #[test]
    fn retryable_classification() {
        assert!(HttpError::Status(429).retryable());
        assert!(HttpError::Status(503).retryable());
        assert!(!HttpError::Status(404).retryable());
        assert!(HttpError::Transport("x".into()).retryable());
    }

    #[test]
    fn http_error_display() {
        assert_eq!(HttpError::Status(429).to_string(), "HTTP 429");
        assert!(HttpError::Transport("boom".into())
            .to_string()
            .contains("boom"));
    }

    struct SeqHttp {
        calls: RefCell<Vec<Result<Vec<u8>, HttpError>>>,
    }
    impl HttpClient for SeqHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            let mut q = self.calls.borrow_mut();
            if q.is_empty() {
                Err(HttpError::Status(500))
            } else {
                q.remove(0)
            }
        }
    }

    #[test]
    fn fetcher_retries_then_succeeds() {
        let http = SeqHttp {
            calls: RefCell::new(vec![
                Err(HttpError::Status(503)),
                Ok(br#"{"ok":true}"#.to_vec()),
            ]),
        };
        let p = policy(2);
        let f = Fetcher::new(&http, &p);
        assert_eq!(f.get("https://x?token=S").unwrap(), br#"{"ok":true}"#);
    }

    #[test]
    fn fetcher_gives_up_after_retries() {
        let http = SeqHttp {
            calls: RefCell::new(vec![
                Err(HttpError::Status(503)),
                Err(HttpError::Status(503)),
            ]),
        };
        let p = policy(1);
        let f = Fetcher::new(&http, &p);
        assert!(f.get("https://x").is_err());
    }

    #[test]
    fn fetcher_does_not_retry_client_errors() {
        let http = SeqHttp {
            calls: RefCell::new(vec![Err(HttpError::Status(404))]),
        };
        let p = policy(5);
        let f = Fetcher::new(&http, &p);
        assert!(f.get("https://x").unwrap_err().contains("404"));
    }

    #[test]
    fn get_json_parses_and_reports_bad_json() {
        let http = SeqHttp {
            calls: RefCell::new(vec![Ok(b"{\"a\":1}".to_vec()), Ok(b"not json".to_vec())]),
        };
        let p = policy(0);
        let f = Fetcher::new(&http, &p);
        assert_eq!(f.get_json("https://x").unwrap()["a"], 1);
        assert!(f.get_json("https://x").unwrap_err().contains("bad JSON"));
    }

    #[test]
    fn throttle_with_rate_limit_runs() {
        let http = SeqHttp {
            calls: RefCell::new(vec![Ok(b"{}".to_vec()), Ok(b"{}".to_vec())]),
        };
        let p = Policy {
            rpm: 6000,
            retries: 0,
            backoff: Duration::ZERO,
        };
        let f = Fetcher::new(&http, &p);
        assert!(f.get("https://x?token=k").is_ok());
        assert!(f.get("https://x?token=k").is_ok());
    }

    #[test]
    fn get_rows_array_object_error_and_non_array() {
        let http = SeqHttp {
            calls: RefCell::new(vec![
                Ok(br#"[{"a":1}]"#.to_vec()),
                Ok(br#"{"Error Message":"bad key"}"#.to_vec()),
                Ok(br#"{"message":"denied"}"#.to_vec()),
                Ok(b"42".to_vec()),
            ]),
        };
        let p = policy(0);
        let f = Fetcher::new(&http, &p);
        assert_eq!(f.get_rows("https://x").unwrap().len(), 1);
        assert!(f
            .get_rows("https://x?apikey=S")
            .unwrap_err()
            .contains("API error: bad key"));
        assert!(f
            .get_rows("https://x")
            .unwrap_err()
            .contains("API error: denied"));
        assert!(f
            .get_rows("https://x")
            .unwrap_err()
            .contains("expected a JSON array"));
    }

    #[test]
    fn write_mode_variants() {
        assert_ne!(WriteMode::Overwrite, WriteMode::Append);
        assert_eq!(WriteMode::Resume, WriteMode::Resume);
    }
}
