//! HTTP client indirection, rate limiting, and retries.

use std::cell::Cell;
use std::time::{Duration, Instant};

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
    #[allow(dead_code)] // used by Fetcher; exercised in unit tests
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
#[allow(dead_code)] // used by Fetcher (#194+); unit-tested here
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
// Wired for price/fundamentals fetch in #194+; kept compiled so the skeleton
// matches pomelo-fmp's HTTP pattern and unit tests cover retry/redact.

/// Wraps an [`HttpClient`] with rate-limit throttle and retry/backoff.
#[allow(dead_code)]
pub(crate) struct Fetcher<'a, H: HttpClient> {
    http: &'a H,
    cfg: &'a SyncConfig,
    /// Earliest instant the next request may fire (rate limit).
    next_ok: Cell<Instant>,
}

#[allow(dead_code)]
impl<'a, H: HttpClient> Fetcher<'a, H> {
    pub(crate) fn new(http: &'a H, cfg: &'a SyncConfig) -> Self {
        Fetcher {
            http,
            cfg,
            next_ok: Cell::new(Instant::now()),
        }
    }

    pub(crate) fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut attempt = 0u32;
        loop {
            self.throttle();
            match self.http.get(url) {
                Ok(body) => return Ok(body),
                Err(e) if e.retryable() && attempt < self.cfg.max_retries => {
                    attempt += 1;
                    let wait = self
                        .cfg
                        .backoff_base
                        .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
                    eprintln!(
                        "retry {attempt}/{} after {} ({wait:?}): {}",
                        self.cfg.max_retries,
                        e,
                        redact(url)
                    );
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                    }
                }
                Err(e) => {
                    return Err(format!("{}: {}", redact(url), e));
                }
            }
        }
    }
}

#[allow(dead_code)]
impl<H: HttpClient> Fetcher<'_, H> {
    fn throttle(&self) {
        let rpm = self.cfg.rate_limit_per_min;
        if rpm == 0 {
            return;
        }
        let min_gap = Duration::from_secs_f64(60.0 / f64::from(rpm));
        let now = Instant::now();
        let next = self.next_ok.get();
        if now < next {
            std::thread::sleep(next - now);
        }
        self.next_ok.set(Instant::now() + min_gap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
