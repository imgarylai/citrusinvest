//! Sync configuration and result summary.

use std::time::Duration;

pub use pomelo_http::WriteMode;

/// Knobs for one [`crate::sync`] run.
pub struct SyncConfig {
    /// Inclusive date bounds, packed `YYYYMMDD`.
    pub from: i32,
    pub to: i32,
    /// Default exchange hint for bare tickers (US equities stay bare on AV).
    pub default_exchange: String,
    /// Also fetch fundamentals → `fundamentals/{SYM}.csv.gz` (phase #216).
    pub include_fundamentals: bool,
    /// Also fetch sector map → `tracked/universe.csv.gz` (phase #215).
    pub include_industry: bool,
    /// Also compute best-effort snapshot panels (phase #218).
    pub include_snapshot_factors: bool,
    /// Max requests per minute (`0` = no throttle).
    pub rate_limit_per_min: u32,
    /// Retries per request on a retryable error before giving up on the symbol.
    pub max_retries: u32,
    /// Base backoff duration; the Nth retry waits `base * 2^(N-1)`.
    pub backoff_base: Duration,
    /// How to treat an already-present tree.
    pub mode: WriteMode,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            from: 20000101,
            to: 99991231,
            default_exchange: "US".to_string(),
            include_fundamentals: false,
            include_industry: false,
            include_snapshot_factors: false,
            rate_limit_per_min: 0,
            max_retries: 4,
            backoff_base: Duration::from_secs(2),
            mode: WriteMode::Overwrite,
        }
    }
}

impl pomelo_http::RetrySettings for SyncConfig {
    fn rate_limit_per_min(&self) -> u32 {
        self.rate_limit_per_min
    }
    fn max_retries(&self) -> u32 {
        self.max_retries
    }
    fn backoff_base(&self) -> Duration {
        self.backoff_base
    }
}

/// What a [`crate::sync`] run produced.
#[derive(Debug, Default)]
pub struct SyncSummary {
    /// Symbols whose price file was (re)written.
    pub symbols_written: usize,
    /// Symbols skipped because they already existed (`--resume`).
    pub symbols_skipped: usize,
    /// Symbols screened out by filters (reserved for later phases).
    pub symbols_filtered: usize,
    /// Total adjusted price rows written across symbols.
    pub price_rows: usize,
    /// Symbols that got a fundamentals file.
    pub fundamentals_written: usize,
    /// Whether `tracked/universe.csv.gz` was written this run.
    pub industry_written: bool,
    /// Number of snapshot-factor panels written under `panels/`.
    pub snapshot_factor_panels: usize,
    /// Per-symbol failures (symbol, message); batch continues after a failure.
    pub failures: Vec<(String, String)>,
}
