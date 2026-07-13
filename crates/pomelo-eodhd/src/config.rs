//! Sync configuration and result summary.

use std::time::Duration;

/// How an already-present symbol tree is treated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteMode {
    /// Overwrite each symbol's files with the freshly fetched window (default).
    Overwrite,
    /// Merge fetched rows into existing files (extend an existing tree).
    Append,
    /// Skip any symbol that already has a `prices/{SYM}.csv.gz`.
    Resume,
}

/// Knobs for one [`crate::sync`] run.
pub struct SyncConfig {
    /// Inclusive date bounds, packed `YYYYMMDD`.
    pub from: i32,
    pub to: i32,
    /// Default exchange suffix when a bare ticker is given (`AAPL` → `AAPL.US`).
    pub default_exchange: String,
    /// Also fetch fundamentals → `fundamentals/{SYM}.csv.gz` (phase #196).
    pub include_fundamentals: bool,
    /// Also fetch sector map → `tracked/universe.csv.gz`.
    pub include_industry: bool,
    /// Max requests per minute (`0` = no throttle). Tune to your EODHD plan.
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
            rate_limit_per_min: 0,
            max_retries: 4,
            backoff_base: Duration::from_secs(2),
            mode: WriteMode::Overwrite,
        }
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
    /// Per-symbol failures (symbol, message); batch continues after a failure.
    pub failures: Vec<(String, String)>,
}
