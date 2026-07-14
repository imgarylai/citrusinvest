//! Sync configuration and result summary.

use std::time::Duration;

pub use pomelo_http::WriteMode;

/// Knobs for one [`sync`] run.
pub struct SyncConfig {
    /// Inclusive date bounds, packed `YYYYMMDD` (as everywhere else in the engine).
    pub from: i32,
    pub to: i32,
    /// Also fetch annual fundamentals → `fundamentals/{SYM}.csv.gz`.
    pub include_fundamentals: bool,
    /// Also fetch company sector → `tracked/universe.csv.gz`.
    pub include_industry: bool,
    /// Also compute the six snapshot-factor panels (`piotroski_score`,
    /// `altman_z`, `fcf_yield`, `pe_industry_pctile`, `analyst_upside_pct`,
    /// `consensus_rating`) → `panels/{name}.csv.gz`. Current-snapshot factors
    /// for universe screening (see [`super::snapshot`]); `pe_industry_pctile`
    /// ranks P/E within an industry cohort drawn from this run's symbols.
    pub include_snapshot_factors: bool,
    /// Skip ETFs / mutual & closed-end funds (default on) — keep only individual
    /// stocks. Classified from the profile endpoint's `isEtf` / `isFund`.
    pub skip_non_stocks: bool,
    /// Skip symbols whose company market cap is below this, in **USD**
    /// (`0.0` = off). Read from the profile endpoint's `marketCap`. The CLI
    /// accepts unit suffixes (`1b`, `500m`) via [`parse_market_cap`].
    pub min_market_cap: f64,
    /// Max requests per minute (`0` = no throttle). FMP imposes a per-plan rate
    /// limit; set this to your plan's ceiling. Starter-class keys are commonly
    /// ~300/min — verify against your own plan.
    pub rate_limit_per_min: u32,
    /// Retries per request on a retryable error before giving up on the symbol.
    pub max_retries: u32,
    /// Base backoff **duration**; the Nth retry waits `base * 2^(N-1)` — e.g. a
    /// 2-second base gives 2s, 4s, 8s, 16s. `Duration::ZERO` disables the sleep
    /// (used by tests).
    pub backoff_base: Duration,
    /// How to treat an already-present tree.
    pub mode: WriteMode,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            from: 20000101,
            to: 99991231,
            include_fundamentals: false,
            include_industry: false,
            include_snapshot_factors: false,
            skip_non_stocks: true,
            min_market_cap: 0.0,
            rate_limit_per_min: 300,
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

/// What a [`sync`] run produced.
#[derive(Debug, Default)]
pub struct SyncSummary {
    /// Symbols whose price file was (re)written.
    pub symbols_written: usize,
    /// Symbols skipped because they already existed (`--resume`).
    pub symbols_skipped: usize,
    /// Symbols screened out by the ETF/fund or market-cap filters.
    pub symbols_filtered: usize,
    /// Total price rows written across all symbols.
    pub price_rows: usize,
    /// Symbols with fundamentals written.
    pub fundamentals_written: usize,
    /// Whether the industry snapshot was written.
    pub industry_written: bool,
    /// Number of `panels/{name}.csv.gz` snapshot-factor panels written.
    pub snapshot_factor_panels: usize,
    /// Per-symbol hard failures (symbol, redacted message). A failure on one
    /// symbol does not abort the batch.
    pub failures: Vec<(String, String)>,
}

// ---- date helpers -----------------------------------------------------------
