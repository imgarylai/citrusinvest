//! HTTP client indirection, rate limiting, and retries.

use std::cell::Cell;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::config::SyncConfig;

/// A classified HTTP failure so the retry loop knows whether to back off.
#[derive(Debug, Clone)]
pub enum HttpError {
    /// A non-success HTTP status (e.g. 401 bad key, 404, 429 rate-limited, 503).
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
    /// Whether retrying (after a backoff) could plausibly succeed: rate limits,
    /// server-side 5xx, and transport blips. A 4xx (bad key, bad symbol) is
    /// terminal — retrying just burns the rate budget.
    pub(crate) fn retryable(&self) -> bool {
        match self {
            HttpError::Transport(_) => true,
            HttpError::Status(code) => *code == 429 || (500..600).contains(code),
        }
    }
}

/// Minimal blocking HTTP GET, abstracted so the sync logic is exercised with a
/// mock in CI (no live key, no network). The real implementation is
/// [`UreqClient`], compiled only with the `fmp-sync` feature.
pub trait HttpClient {
    /// GET `url`, returning the response body on a 2xx status.
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
}

/// The real ureq-backed client. `default-features=false` on ureq means no
/// transparent gzip decode; FMP JSON is read verbatim.
#[cfg(feature = "fmp-sync")]
pub struct UreqClient;

#[cfg(feature = "fmp-sync")]
impl UreqClient {
    pub fn new() -> Self {
        UreqClient
    }
}

#[cfg(feature = "fmp-sync")]
impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "fmp-sync")]
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

pub(crate) fn redact(url: &str) -> String {
    match url.find("apikey=") {
        Some(i) => {
            let start = i + "apikey=".len();
            let end = url[start..]
                .find('&')
                .map(|j| start + j)
                .unwrap_or(url.len());
            format!("{}***{}", &url[..start], &url[end..])
        }
        None => url.to_string(),
    }
}

// ---- fetcher (throttle + retry) --------------------------------------------

/// Wraps an [`HttpClient`] with the rate-limit throttle and retry/backoff loop.
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

    /// Sleep enough to keep under `rate_limit_per_min`.
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

    /// GET with throttle + bounded exponential backoff. On success returns the
    /// body; on terminal failure returns a message with the key redacted.
    pub(crate) fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut attempt = 0u32;
        loop {
            self.throttle();
            match self.http.get(url) {
                Ok(body) => return Ok(body),
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    let wait = self.cfg.backoff_base * 2u32.pow(attempt.min(16));
                    eprintln!(
                        "  retry {}/{} after {}: {} ({})",
                        attempt + 1,
                        self.cfg.max_retries,
                        e,
                        redact(url),
                        wait.as_secs_f64()
                    );
                    std::thread::sleep(wait);
                    attempt += 1;
                }
                Err(e) => return Err(format!("{e} for {}", redact(url))),
            }
        }
    }

    /// GET and parse the body as a JSON array of row objects. FMP error payloads
    /// come back as a JSON object (`{"Error Message": ...}`) rather than an
    /// array — surface that as an error instead of silently yielding no rows.
    pub(crate) fn get_rows(&self, url: &str) -> Result<Vec<Value>, String> {
        let body = self.get(url)?;
        let value: Value = serde_json::from_slice(&body)
            .map_err(|e| format!("bad JSON from {}: {e}", redact(url)))?;
        match value {
            Value::Array(rows) => Ok(rows),
            Value::Object(map) => {
                // Any of the FMP error shapes: {"Error Message": "..."} etc.
                if let Some(msg) = map
                    .get("Error Message")
                    .or_else(|| map.get("error"))
                    .and_then(|v| v.as_str())
                {
                    Err(format!("FMP error: {msg}"))
                } else {
                    // A lone object is unexpected for these list endpoints.
                    Err(format!("expected a JSON array from {}", redact(url)))
                }
            }
            _ => Err(format!("expected a JSON array from {}", redact(url))),
        }
    }
}
