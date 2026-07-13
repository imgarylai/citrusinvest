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

/// The real ureq-backed client — only with the `eodhd-sync` feature.
#[cfg(feature = "eodhd-sync")]
pub struct UreqClient;

#[cfg(feature = "eodhd-sync")]
impl UreqClient {
    pub fn new() -> Self {
        UreqClient
    }
}

#[cfg(feature = "eodhd-sync")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "eodhd-sync")]
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

/// Redact common EODHD token query params for stderr logs.
pub(crate) fn redact(url: &str) -> String {
    for key in ["api_token=", "api_key="] {
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

    /// GET and parse a JSON array of row objects (EOD list endpoints).
    pub(crate) fn get_rows(&self, url: &str) -> Result<Vec<Value>, String> {
        let body = self.get(url)?;
        let value: Value = serde_json::from_slice(&body)
            .map_err(|e| format!("bad JSON from {}: {e}", redact(url)))?;
        match value {
            Value::Array(rows) => Ok(rows),
            Value::Object(map) => {
                if let Some(msg) = map
                    .get("message")
                    .or_else(|| map.get("error"))
                    .or_else(|| map.get("Error Message"))
                    .and_then(|v| v.as_str())
                {
                    Err(format!("EODHD error: {msg}"))
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
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn redacts_api_token() {
        assert_eq!(
            redact("https://eodhd.com/api/eod/AAPL.US?api_token=SECRET&fmt=json"),
            "https://eodhd.com/api/eod/AAPL.US?api_token=***&fmt=json"
        );
        assert_eq!(
            redact("https://eodhd.com/api/eod/AAPL.US?api_token=SECRET"),
            "https://eodhd.com/api/eod/AAPL.US?api_token=***"
        );
        assert_eq!(
            redact("https://x?api_key=SECRET&fmt=json"),
            "https://x?api_key=***&fmt=json"
        );
        assert_eq!(
            redact("https://eodhd.com/api/eod/AAPL.US"),
            "https://eodhd.com/api/eod/AAPL.US"
        );
    }

    #[test]
    fn retryable_classification() {
        assert!(HttpError::Status(429).retryable());
        assert!(HttpError::Status(503).retryable());
        assert!(HttpError::Transport("reset".into()).retryable());
        assert!(!HttpError::Status(401).retryable());
        assert!(!HttpError::Status(404).retryable());
    }

    #[test]
    fn http_error_display() {
        assert_eq!(HttpError::Status(404).to_string(), "HTTP 404");
        assert_eq!(
            HttpError::Transport("boom".into()).to_string(),
            "transport error: boom"
        );
    }

    struct SeqHttp {
        /// Shared queue of responses consumed in order.
        seq: Cell<Vec<Result<Vec<u8>, HttpError>>>,
    }

    impl SeqHttp {
        fn new(seq: Vec<Result<Vec<u8>, HttpError>>) -> Self {
            SeqHttp {
                seq: Cell::new(seq),
            }
        }
    }

    impl HttpClient for SeqHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            let mut q = self.seq.take();
            let r = if q.is_empty() {
                Err(HttpError::Status(404))
            } else {
                q.remove(0)
            };
            self.seq.set(q);
            r
        }
    }

    fn cfg_retry() -> SyncConfig {
        SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 2,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    #[test]
    fn fetcher_retries_then_succeeds() {
        let http = SeqHttp::new(vec![
            Err(HttpError::Status(429)),
            Err(HttpError::Transport("blip".into())),
            Ok(br#"[{"ok":true}]"#.to_vec()),
        ]);
        let cfg = cfg_retry();
        let f = Fetcher::new(&http, &cfg);
        let body = f.get("https://x?api_token=SECRET").unwrap();
        assert_eq!(body, br#"[{"ok":true}]"#);
    }

    #[test]
    fn fetcher_gives_up_after_retries() {
        let http = SeqHttp::new(vec![
            Err(HttpError::Status(503)),
            Err(HttpError::Status(503)),
            Err(HttpError::Status(503)),
        ]);
        let cfg = cfg_retry();
        let f = Fetcher::new(&http, &cfg);
        let err = f.get("https://x").unwrap_err();
        assert!(err.contains("HTTP 503"), "{err}");
    }

    #[test]
    fn fetcher_does_not_retry_client_errors() {
        let http = SeqHttp::new(vec![Err(HttpError::Status(401))]);
        let cfg = cfg_retry();
        let f = Fetcher::new(&http, &cfg);
        let err = f.get("https://x").unwrap_err();
        assert!(err.contains("HTTP 401"), "{err}");
    }

    #[test]
    fn get_rows_parses_array_and_errors() {
        let cfg = cfg_retry();

        let ok = SeqHttp::new(vec![Ok(br#"[{"a":1},{"a":2}]"#.to_vec())]);
        let rows = Fetcher::new(&ok, &cfg).get_rows("https://x").unwrap();
        assert_eq!(rows.len(), 2);

        let msg = SeqHttp::new(vec![Ok(br#"{"message":"nope"}"#.to_vec())]);
        let err = Fetcher::new(&msg, &cfg).get_rows("https://x").unwrap_err();
        assert!(err.contains("EODHD error: nope"), "{err}");

        let obj = SeqHttp::new(vec![Ok(br#"{"foo":1}"#.to_vec())]);
        let err = Fetcher::new(&obj, &cfg).get_rows("https://x").unwrap_err();
        assert!(err.contains("expected a JSON array"), "{err}");

        let bad = SeqHttp::new(vec![Ok(br#"not-json"#.to_vec())]);
        let err = Fetcher::new(&bad, &cfg).get_rows("https://x").unwrap_err();
        assert!(err.contains("bad JSON"), "{err}");

        let num = SeqHttp::new(vec![Ok(br#"42"#.to_vec())]);
        let err = Fetcher::new(&num, &cfg).get_rows("https://x").unwrap_err();
        assert!(err.contains("expected a JSON array"), "{err}");
    }

    #[test]
    fn throttle_with_rate_limit_runs() {
        // Smoke: rate_limit > 0 exercises throttle without asserting sleep.
        let http = SeqHttp::new(vec![Ok(b"[]".to_vec()), Ok(b"[]".to_vec())]);
        let cfg = SyncConfig {
            rate_limit_per_min: 6000, // tiny interval
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let f = Fetcher::new(&http, &cfg);
        assert!(f.get("https://a").is_ok());
        assert!(f.get("https://b").is_ok());
    }
}
