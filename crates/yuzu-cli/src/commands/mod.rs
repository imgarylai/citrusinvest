use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

pub(crate) mod audit;
pub(crate) mod backtest;
pub(crate) mod research;
pub(crate) mod sync;

/// Sort key for the sweep leaderboard (mirrors `yuzu_cli::SortKey`).
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum SortArg {
    Sharpe,
    TotalReturn,
    Cagr,
    Calmar,
}

impl From<SortArg> for yuzu_cli::SortKey {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Sharpe => yuzu_cli::SortKey::Sharpe,
            SortArg::TotalReturn => yuzu_cli::SortKey::TotalReturn,
            SortArg::Cagr => yuzu_cli::SortKey::Cagr,
            SortArg::Calmar => yuzu_cli::SortKey::Calmar,
        }
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct Variant {
    pub(crate) name: String,
    pub(crate) spec: serde_json::Value,
}

/// Flags shared by every command: data location, date range, and the
/// `BacktestConfig` knobs.
#[derive(Args)]
pub(crate) struct CommonArgs {
    /// Directory mirroring R2's prices/ tree.
    #[arg(long)]
    pub(crate) data: PathBuf,
    #[arg(long, default_value_t = 20000101)]
    pub(crate) from: i32,
    #[arg(long, default_value_t = 99991231)]
    pub(crate) to: i32,
    #[arg(long, default_value_t = 0.0)]
    pub(crate) fee_ratio: f64,
    /// Slippage per unit of turnover (e.g. 0.0005 = 5 bps per leg).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) slippage_ratio: f64,
    /// Book size in dollars for the liquidity cap (0 = cap off).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) initial_capital: f64,
    /// Max fraction of a symbol's daily dollar volume the book may hold.
    #[arg(long, default_value_t = 0.0)]
    pub(crate) max_participation: f64,
    /// Square-root market-impact coefficient (0 = off; needs --initial-capital).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) impact_coef: f64,
    /// Treat a symbol as delisted after N consecutive missing-price days (0 = off).
    #[arg(long, default_value_t = 0)]
    pub(crate) delist_after: usize,
    /// Fraction of a delisted position written off (0 = exit at last price, 1 = total loss).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) delist_haircut: f64,
    /// Benchmark symbol (its closes are loaded and compared), e.g. SPY.
    #[arg(long)]
    pub(crate) benchmark: Option<String>,
    /// Bootstrap resamples for confidence bands (0 = off).
    #[arg(long, default_value_t = 0)]
    pub(crate) bootstrap_samples: usize,
    /// Bootstrap block length in days (0 = auto sqrt(n)).
    #[arg(long, default_value_t = 0)]
    pub(crate) bootstrap_block: usize,
    /// Date (YYYYMMDD) the strategy went live; adds a `live` block of
    /// post-live equity metrics to the report (unset = omit).
    #[arg(long)]
    pub(crate) live_performance_start: Option<i32>,
    /// Stop-loss as a fraction of entry (e.g. 0.08 = −8%); unset = off.
    #[arg(long)]
    pub(crate) stop_loss: Option<f64>,
    /// Take-profit as a fraction of entry (e.g. 0.2 = +20%); unset = off.
    #[arg(long)]
    pub(crate) take_profit: Option<f64>,
    /// Trailing stop: exit this far below the best return since entry; unset = off.
    #[arg(long)]
    pub(crate) trail_stop: Option<f64>,
    /// Return the trailing stop must reach before it arms (default 0).
    #[arg(long, default_value_t = 0.0)]
    pub(crate) trail_stop_activation: f64,
    /// How a stop fills: `touched` (stop price / gapped open, default) or `close`.
    #[arg(long, value_enum, default_value_t = StopFillArg::Touched)]
    pub(crate) stop_fill: StopFillArg,
    /// Output file (default: stdout).
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
}

/// CLI mirror of `yuzu_core::backtest::StopFill`.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum StopFillArg {
    Touched,
    Close,
}

/// Index whose point-in-time membership `fmp-sync --index` reconstructs.
#[cfg(feature = "fmp-sync")]
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum IndexArg {
    Sp500,
    Nasdaq,
    Dowjones,
}

#[cfg(feature = "fmp-sync")]
impl From<IndexArg> for yuzu_cli::fmp::Index {
    fn from(a: IndexArg) -> Self {
        match a {
            IndexArg::Sp500 => yuzu_cli::fmp::Index::Sp500,
            IndexArg::Nasdaq => yuzu_cli::fmp::Index::Nasdaq,
            IndexArg::Dowjones => yuzu_cli::fmp::Index::DowJones,
        }
    }
}

/// Index for `eodhd-sync --index` (v1: SPX only).
#[cfg(feature = "eodhd-sync")]
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum EodhdIndexArg {
    Sp500,
}

#[cfg(feature = "eodhd-sync")]
impl From<EodhdIndexArg> for yuzu_cli::eodhd::Index {
    fn from(a: EodhdIndexArg) -> Self {
        match a {
            EodhdIndexArg::Sp500 => yuzu_cli::eodhd::Index::Sp500,
        }
    }
}

/// Index for `finnhub-sync --index` (v1: SPX only).
#[cfg(feature = "finnhub-sync")]
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum FinnhubIndexArg {
    Sp500,
}

#[cfg(feature = "finnhub-sync")]
impl From<FinnhubIndexArg> for yuzu_cli::finnhub::Index {
    fn from(a: FinnhubIndexArg) -> Self {
        match a {
            FinnhubIndexArg::Sp500 => yuzu_cli::finnhub::Index::Sp500,
        }
    }
}

impl CommonArgs {
    pub(crate) fn config(&self) -> yuzu_core::backtest::BacktestConfig {
        use yuzu_core::backtest::{StopConfig, StopFill};
        let stops = StopConfig::from_options(
            self.stop_loss,
            self.take_profit,
            self.trail_stop,
            self.trail_stop_activation,
            match self.stop_fill {
                StopFillArg::Touched => StopFill::Touched,
                StopFillArg::Close => StopFill::Close,
            },
        );
        yuzu_core::backtest::BacktestConfig {
            fee_ratio: self.fee_ratio,
            slippage_ratio: self.slippage_ratio,
            initial_capital: self.initial_capital,
            max_participation: self.max_participation,
            impact_coef: self.impact_coef,
            delist_after: self.delist_after,
            delist_haircut: self.delist_haircut,
            benchmark_key: self.benchmark.clone(),
            bootstrap_samples: self.bootstrap_samples,
            bootstrap_block: self.bootstrap_block,
            live_performance_start: self.live_performance_start,
            stops,
            ..Default::default()
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// Run one strategy over the full universe.
    Run(backtest::RunArgs),
    /// Run many strategy variants in parallel and emit a ranked leaderboard.
    Sweep(research::SweepArgs),
    /// Expand a parameter grid and run a ranked sweep over every combination.
    Grid(research::GridArgs),
    /// Factor diagnostics (rank IC / ICIR, quantile-portfolio returns,
    /// long-short spread) of a factor spec vs forward returns. Research JSON —
    /// not a backtest.
    Factor(research::FactorArgs),
    /// Event study: average (and cumulative) return around a 0/1 event spec
    /// over a [-pre, +post] window. Research JSON — not a backtest.
    Event(research::EventArgs),
    /// Compare a strategy against a signal-lagged rerun to flag lookahead
    /// bias / same-close execution dependence.
    Lookahead(research::LookaheadArgs),
    /// Walk-forward analysis: pick the best grid variant in-sample per window,
    /// evaluate it out-of-sample, and chain the OOS equity.
    Walkforward(research::WalkforwardArgs),
    /// Read-only data-quality audit of a synced data-layout tree.
    ///
    /// Walks `prices/` / `fundamentals/` / `panels/` / `tracked/` and reports
    /// per-check OK / WARN / FAIL (coverage, calendar gaps, adjustment sanity,
    /// survivorship, NaN density, filing-date lag, index membership). Human
    /// table by default; `--json` for machine consumption. Exits non-zero when
    /// any check FAILs, so it can gate CI or a nightly job.
    ///
    /// `--data` accepts a local path or an `s3://bucket[/prefix]` URL (same
    /// credential chain as `fmp-sync --out`, see docs/fmp-data-source.md).
    /// Discovery (which symbols/files exist) is list-only and cheap either way;
    /// the content checks (gaps, jumps, NaN density, filing lag) read every
    /// object, so a deep audit of a remote tree costs about as much as syncing
    /// it locally first.
    DataAudit(audit::DataAuditArgs),
    /// Sync FMP data with YOUR OWN API key into a local `data-layout.md` tree.
    ///
    /// Direct HTTP (no third-party FMP SDK); the key stays on this machine and
    /// no FMP data is redistributed. MVP is enough for close/OHLC TA and
    /// cross-section ops over a short US window — see docs/fmp-data-source.md
    /// for what an FMP Starter key can and cannot honestly backtest.
    ///
    /// Example:
    ///   yuzu-cli fmp-sync --api-key "$FMP_API_KEY" --out ./mydata \
    ///     --symbols AAPL,MSFT,GOOGL --from 20200101 --to 20251231
    #[cfg(feature = "fmp-sync")]
    FmpSync(sync::fmp::FmpSyncArgs),
    /// Build a screened symbol universe from FMP and write it to a file — the
    /// "establish the sync list first" step for whole-market backtests.
    ///
    /// The output (one ticker per line) is meant to be reviewed/edited and then
    /// fed to `fmp-sync --symbols-file`. Uses FMP's company screener, so filters
    /// are applied server-side (with a client-side re-check).
    ///
    /// Example:
    ///   yuzu-cli fmp-symbols --api-key "$FMP_API_KEY" --out ./universe.txt \
    ///     --min-market-cap 1e9 --exchange NASDAQ,NYSE
    #[cfg(feature = "fmp-sync")]
    FmpSymbols(sync::fmp::FmpSymbolsArgs),
    /// Sync EODHD data with YOUR OWN API token into a `data-layout.md` tree.
    ///
    /// Second official vendor path (epic #192) so users are not FMP-only.
    /// Direct HTTP (no third-party EODHD SDK); the token stays on this machine.
    /// Coverage map: docs/data-sources.md § EODHD.
    ///
    /// Example:
    ///   yuzu-cli eodhd-sync --api-token "$EODHD_API_TOKEN" --out ./mydata \
    ///     --symbols AAPL,MSFT --from 20200101 --to 20251231
    #[cfg(feature = "eodhd-sync")]
    EodhdSync(sync::eodhd::EodhdSyncArgs),
    /// Build a screened symbol universe from EODHD and write it to a file.
    ///
    /// Example:
    ///   yuzu-cli eodhd-symbols --api-token "$EODHD_API_TOKEN" --out ./universe.txt \
    ///     --min-market-cap 1b --exchange us --limit 200
    #[cfg(feature = "eodhd-sync")]
    EodhdSymbols(sync::eodhd::EodhdSymbolsArgs),
    /// Sync Alpha Vantage data with YOUR OWN API key into a `data-layout.md` tree.
    ///
    /// Optional vendor path (epic #209). Prices: TIME_SERIES_DAILY_ADJUSTED →
    /// prices/ with adj OHLC scale (outputsize=full). Industry from OVERVIEW;
    /// delisted via LISTING_STATUS; annual IS/BS densify with period-end
    /// report_event (no filing_date on AV). Coverage: docs/data-sources.md § AV.
    ///
    /// Example:
    ///   yuzu-cli av-sync --api-key "$ALPHA_VANTAGE_API_KEY" --out ./mydata \
    ///     --symbols AAPL,MSFT --from 20200101 --to 20251231 \
    ///     --include-industry --include-delisted
    #[cfg(feature = "alpha-vantage-sync")]
    AvSync(sync::alpha_vantage::AvSyncArgs),
    /// Build a symbol universe from Alpha Vantage LISTING_STATUS (active).
    ///
    /// Not a market-cap screener (AV has none). Filters optional exchange /
    /// assetType. **No index membership** — AV cannot produce honest
    /// `panels/in_sp500` (#207 / #217); use FMP/EODHD/Finnhub or BYO for PIT.
    ///
    /// Example:
    ///   yuzu-cli av-symbols --api-key "$ALPHA_VANTAGE_API_KEY" --out ./u.txt \
    ///     --exchange NASDAQ --asset-type Stock --limit 500
    #[cfg(feature = "alpha-vantage-sync")]
    AvSymbols(sync::alpha_vantage::AvSymbolsArgs),
    /// Sync Finnhub data with YOUR OWN API key into a `data-layout.md` tree.
    ///
    /// Optional vendor path (epic #210). Writes adjusted daily candles →
    /// `prices/{SYM}.csv.gz` (#226); industry/fundamentals/index/snapshot flags
    /// are reserved for later phases. Coverage / gaps: docs/data-sources.md § Finnhub.
    ///
    /// Example:
    ///   yuzu-cli finnhub-sync --api-key "$FINNHUB_API_KEY" --out ./mydata \
    ///     --symbols AAPL,MSFT --from 20200101 --to 20251231
    #[cfg(feature = "finnhub-sync")]
    FinnhubSync(sync::finnhub::FinnhubSyncArgs),
    /// Build an exchange's symbol universe from Finnhub and write it to a file.
    ///
    /// Not a market-cap screener — the full `/stock/symbol` listing, filtered by
    /// security type. For point-in-time index membership use `finnhub-sync --index`.
    ///
    /// Example:
    ///   yuzu-cli finnhub-symbols --api-key "$FINNHUB_API_KEY" --out ./universe.txt \
    ///     --exchange US --security-type "Common Stock" --limit 500
    #[cfg(feature = "finnhub-sync")]
    FinnhubSymbols(sync::finnhub::FinnhubSymbolsArgs),
}

pub(crate) fn emit(out: &Option<PathBuf>, json: String) -> std::io::Result<()> {
    match out {
        Some(p) => std::fs::write(p, json),
        None => {
            println!("{json}");
            Ok(())
        }
    }
}

/// `data-audit --data`: a local path or `s3://bucket[/prefix]` (same
/// `pomelo_s3::OutStore` parsing / credential chain as `fmp-sync --out`).
/// S3/R2 support pulls the `fmp-sync` feature's ureq/rusty-s3 stack — an
/// `s3://` URL without it is a clear error rather than a silent local-path
/// misinterpretation.
#[cfg(feature = "fmp-sync")]
pub(crate) fn run_data_audit_over(
    data: &str,
    from: i32,
    to: i32,
) -> Result<pomelo_audit::DataAuditReport, Box<dyn std::error::Error>> {
    let src = pomelo_s3::OutStore::parse(data)?;
    Ok(pomelo_audit::run_data_audit(&src, data, from, to)?)
}

#[cfg(not(feature = "fmp-sync"))]
pub(crate) fn run_data_audit_over(
    data: &str,
    from: i32,
    to: i32,
) -> Result<pomelo_audit::DataAuditReport, Box<dyn std::error::Error>> {
    if data.starts_with("s3://") {
        return Err("data-audit --data s3://… needs the fmp-sync feature (on by default; rebuild without --no-default-features)".into());
    }
    let src = pomelo_data::LocalSource::new(data);
    Ok(pomelo_audit::run_data_audit(&src, data, from, to)?)
}

pub(crate) fn load_grid_variants(
    path: &PathBuf,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)?;
    let grid: yuzu_cli::GridSpec = serde_json::from_str(&raw)?;
    yuzu_cli::expand_grid(&grid)
        .into_iter()
        .map(|(name, spec)| Ok((name, serde_json::to_string(&spec)?)))
        .collect::<Result<_, serde_json::Error>>()
        .map_err(Into::into)
}

pub fn dispatch(cmd: Cmd) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Cmd::Run(args) => backtest::run(args),
        Cmd::Sweep(args) => research::sweep(args),
        Cmd::Grid(args) => research::grid(args),
        Cmd::Factor(args) => research::factor(args),
        Cmd::Event(args) => research::event(args),
        Cmd::Lookahead(args) => research::lookahead(args),
        Cmd::Walkforward(args) => research::walkforward(args),
        Cmd::DataAudit(args) => audit::run(args),
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSync(args) => sync::fmp::sync(args),
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSymbols(args) => sync::fmp::symbols(args),
        #[cfg(feature = "eodhd-sync")]
        Cmd::EodhdSync(args) => sync::eodhd::sync(args),
        #[cfg(feature = "eodhd-sync")]
        Cmd::EodhdSymbols(args) => sync::eodhd::symbols(args),
        #[cfg(feature = "alpha-vantage-sync")]
        Cmd::AvSync(args) => sync::alpha_vantage::sync(args),
        #[cfg(feature = "alpha-vantage-sync")]
        Cmd::AvSymbols(args) => sync::alpha_vantage::symbols(args),
        #[cfg(feature = "finnhub-sync")]
        Cmd::FinnhubSync(args) => sync::finnhub::sync(args),
        #[cfg(feature = "finnhub-sync")]
        Cmd::FinnhubSymbols(args) => sync::finnhub::symbols(args),
    }
}
