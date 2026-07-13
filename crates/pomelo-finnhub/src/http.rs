//! HTTP client indirection, rate limiting, and retries.

use std::cell::Cell;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::config::SyncConfig;

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
    pub(crate) fn retryable(&self) -> bool {
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

/// The real ureq-backed client — only with the `finnhub-sync` feature.
#[cfg(feature = "finnhub-sync")]
pub struct UreqClient;

#[cfg(feature = "finnhub-sync")]
impl UreqClient {
    pub fn new() -> Self {
        UreqClient
    }
}

#[cfg(feature = "finnhub-sync")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "finnhub-sync")]
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

/// Redact Finnhub token query params for stderr logs.
pub(crate) fn redact(url: &str) -> String {
    for key in ["token=", "apikey=", "api_key="] {
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

// ---- fetcher (throttle + retry) --------------------------------------------

/// Wraps an [`HttpClient`] with rate-limit throttle and retry/backoff.
pub(crate) struct Fetcher<'a, H: HttpClient> {
    http: &'a H,
    cfg: &'a SyncConfig,
    last_request: Cell<Option<Instant>>,
}

impl<'a, H: HttpClient> Fetcher<'a, H> {
    pub(crate) fn new(http: &'a H, cfg: &'a SyncConfig) -> Self {
        Fetcher {
            http,
            cfg,
            last_request: Cell::new(None),
        }
    }

    fn throttle(&self) {
        if self.cfg.rate_limit_per_min == 0 {
            return;
        }
        let min_interval = Duration::from_secs_f64(60.0 / self.cfg.rate_limit_per_min as f64);
        if let Some(prev) = self.last_request.get() {
            let elapsed = prev.elapsed();
            if elapsed < min_interval {
                std::thread::sleep(min_interval - elapsed);
            }
        }
        self.last_request.set(Some(Instant::now()));
    }

    /// GET with throttle + bounded exponential backoff.
    pub(crate) fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut attempt = 0u32;
        loop {
            self.throttle();
            match self.http.get(url) {
                Ok(body) => return Ok(body),
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    let wait = self.cfg.backoff_base * 2u32.pow(attempt.min(16));
                    eprintln!(
                        "  retry {}/{} after {}: {} ({:?})",
                        attempt + 1,
                        self.cfg.max_retries,
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
    pub(crate) fn get_json(&self, url: &str) -> Result<Value, String> {
        let body = self.get(url)?;
        serde_json::from_slice(&body).map_err(|e| format!("bad JSON from {}: {e}", redact(url)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    #[test]
    fn redacts_token() {
        assert_eq!(
            redact("https://finnhub.io/api/v1/stock/candle?symbol=AAPL&token=SECRET"),
            "https://finnhub.io/api/v1/stock/candle?symbol=AAPL&token=***"
        );
        assert_eq!(
            redact("https://x?token=SECRET&resolution=D"),
            "https://x?token=***&resolution=D"
        );
    }

    #[test]
    fn retryable_classification() {
        assert!(HttpError::Status(429).retryable());
        assert!(HttpError::Status(503).retryable());
        assert!(!HttpError::Status(404).retryable());
        assert!(HttpError::Transport("x".into()).retryable());
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
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 2,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let body = fetcher
            .get("https://finnhub.io/api/v1/x?token=SECRET")
            .unwrap();
        assert_eq!(body, br#"{"ok":true}"#);
    }

    #[test]
    fn http_error_display() {
        assert_eq!(HttpError::Status(429).to_string(), "HTTP 429");
        assert!(HttpError::Transport("boom".into())
            .to_string()
            .contains("boom"));
    }

    #[test]
    fn throttle_with_rate_limit_runs() {
        let http = SeqHttp {
            calls: RefCell::new(vec![Ok(b"{}".to_vec()), Ok(b"{}".to_vec())]),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 6000,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        assert!(fetcher.get("https://x?token=k").is_ok());
        assert!(fetcher.get("https://x?token=k").is_ok());
    }

    #[test]
    fn fetcher_gives_up_after_retries() {
        let http = SeqHttp {
            calls: RefCell::new(vec![
                Err(HttpError::Status(503)),
                Err(HttpError::Status(503)),
            ]),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 1,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        assert!(fetcher.get("https://x").is_err());
    }

    #[test]
    fn fetcher_does_not_retry_client_errors() {
        let http = SeqHttp {
            calls: RefCell::new(vec![Err(HttpError::Status(404))]),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 5,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let err = fetcher.get("https://x").unwrap_err();
        assert!(err.contains("404"), "{err}");
    }
}
