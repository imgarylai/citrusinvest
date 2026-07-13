use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "yuzu-cli", about = "Native batch backtest runner")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Sort key for the sweep leaderboard (mirrors `yuzu_cli::SortKey`).
#[derive(Clone, Copy, ValueEnum)]
enum SortArg {
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
struct Variant {
    name: String,
    spec: serde_json::Value,
}

/// Flags shared by every command: data location, date range, and the
/// `BacktestConfig` knobs.
#[derive(Args)]
struct CommonArgs {
    /// Directory mirroring R2's prices/ tree.
    #[arg(long)]
    data: PathBuf,
    #[arg(long, default_value_t = 20000101)]
    from: i32,
    #[arg(long, default_value_t = 99991231)]
    to: i32,
    #[arg(long, default_value_t = 0.0)]
    fee_ratio: f64,
    /// Slippage per unit of turnover (e.g. 0.0005 = 5 bps per leg).
    #[arg(long, default_value_t = 0.0)]
    slippage_ratio: f64,
    /// Book size in dollars for the liquidity cap (0 = cap off).
    #[arg(long, default_value_t = 0.0)]
    initial_capital: f64,
    /// Max fraction of a symbol's daily dollar volume the book may hold.
    #[arg(long, default_value_t = 0.0)]
    max_participation: f64,
    /// Square-root market-impact coefficient (0 = off; needs --initial-capital).
    #[arg(long, default_value_t = 0.0)]
    impact_coef: f64,
    /// Treat a symbol as delisted after N consecutive missing-price days (0 = off).
    #[arg(long, default_value_t = 0)]
    delist_after: usize,
    /// Fraction of a delisted position written off (0 = exit at last price, 1 = total loss).
    #[arg(long, default_value_t = 0.0)]
    delist_haircut: f64,
    /// Benchmark symbol (its closes are loaded and compared), e.g. SPY.
    #[arg(long)]
    benchmark: Option<String>,
    /// Bootstrap resamples for confidence bands (0 = off).
    #[arg(long, default_value_t = 0)]
    bootstrap_samples: usize,
    /// Bootstrap block length in days (0 = auto sqrt(n)).
    #[arg(long, default_value_t = 0)]
    bootstrap_block: usize,
    /// Date (YYYYMMDD) the strategy went live; adds a `live` block of
    /// post-live equity metrics to the report (unset = omit).
    #[arg(long)]
    live_performance_start: Option<i32>,
    /// Stop-loss as a fraction of entry (e.g. 0.08 = −8%); unset = off.
    #[arg(long)]
    stop_loss: Option<f64>,
    /// Take-profit as a fraction of entry (e.g. 0.2 = +20%); unset = off.
    #[arg(long)]
    take_profit: Option<f64>,
    /// Trailing stop: exit this far below the best return since entry; unset = off.
    #[arg(long)]
    trail_stop: Option<f64>,
    /// Return the trailing stop must reach before it arms (default 0).
    #[arg(long, default_value_t = 0.0)]
    trail_stop_activation: f64,
    /// How a stop fills: `touched` (stop price / gapped open, default) or `close`.
    #[arg(long, value_enum, default_value_t = StopFillArg::Touched)]
    stop_fill: StopFillArg,
    /// Output file (default: stdout).
    #[arg(long)]
    out: Option<PathBuf>,
}

/// CLI mirror of `yuzu_core::backtest::StopFill`.
#[derive(Clone, Copy, ValueEnum)]
enum StopFillArg {
    Touched,
    Close,
}

/// Index whose point-in-time membership `fmp-sync --index` reconstructs.
#[cfg(feature = "fmp-sync")]
#[derive(Clone, Copy, ValueEnum)]
enum IndexArg {
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
enum EodhdIndexArg {
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

impl CommonArgs {
    fn config(&self) -> yuzu_core::backtest::BacktestConfig {
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
enum Cmd {
    /// Run one strategy over the full universe.
    Run {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a JSON Expr spec file.
        #[arg(long)]
        spec: PathBuf,
        /// Price series that drives fills and daily returns: open/high/low/close.
        /// For next-open execution, keep close-based signals and lag them
        /// (`shift(signal, 1)`), then set `--price-key open`.
        #[arg(long, default_value = "close")]
        price_key: String,
    },
    /// Run many strategy variants in parallel and emit a ranked leaderboard.
    Sweep {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a JSON file: `[{"name": "...", "spec": {...Expr...}}, ...]`.
        #[arg(long)]
        specs: PathBuf,
        /// Metric to rank by.
        #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
        sort: SortArg,
        /// Keep only the top N entries (default: all).
        #[arg(long)]
        top: Option<usize>,
        /// Price series that drives fills and daily returns: open/high/low/close.
        #[arg(long, default_value = "close")]
        price_key: String,
    },
    /// Expand a parameter grid and run a ranked sweep over every combination.
    Grid {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a grid file: `{"spec": {...with "$name"...}, "params": {"name": [values]}}`.
        #[arg(long)]
        grid: PathBuf,
        /// Metric to rank by.
        #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
        sort: SortArg,
        /// Keep only the top N entries (default: all).
        #[arg(long)]
        top: Option<usize>,
        /// Price series that drives fills and daily returns: open/high/low/close.
        #[arg(long, default_value = "close")]
        price_key: String,
    },
    /// Factor diagnostics (rank IC / ICIR, quantile-portfolio returns,
    /// long-short spread) of a factor spec vs forward returns. Research JSON —
    /// not a backtest.
    Factor {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a JSON Expr spec evaluated to the factor panel.
        #[arg(long)]
        spec: PathBuf,
        /// Forward-return horizon in trading days.
        #[arg(long, default_value_t = 1)]
        horizon: usize,
        /// Number of factor quantile buckets.
        #[arg(long, default_value_t = 5)]
        quantiles: usize,
        /// Demean the factor within each sector before ranking (needs an
        /// industry map in the data tree).
        #[arg(long)]
        neutralize_industry: bool,
    },
    /// Event study: average (and cumulative) return around a 0/1 event spec
    /// over a [-pre, +post] window. Research JSON — not a backtest.
    Event {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a JSON Expr spec evaluated to the 0/1 event panel.
        #[arg(long)]
        spec: PathBuf,
        /// Rows before each event to include.
        #[arg(long, default_value_t = 5)]
        pre: usize,
        /// Rows after each event to include.
        #[arg(long, default_value_t = 5)]
        post: usize,
    },
    /// Compare a strategy against a signal-lagged rerun to flag lookahead
    /// bias / same-close execution dependence.
    Lookahead {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a JSON Expr spec file.
        #[arg(long)]
        spec: PathBuf,
        /// Days to lag the position matrix in the comparison run.
        #[arg(long, default_value_t = 1)]
        shift_days: usize,
        /// Run the full decay profile (shifts 1,2,3,5,10,21) instead of a
        /// single comparison; --shift-days is ignored.
        #[arg(long)]
        profile: bool,
    },
    /// Walk-forward analysis: pick the best grid variant in-sample per window,
    /// evaluate it out-of-sample, and chain the OOS equity.
    Walkforward {
        #[command(flatten)]
        common: CommonArgs,
        /// Path to a grid file (same format as `grid`).
        #[arg(long)]
        grid: PathBuf,
        /// In-sample window length in trading days.
        #[arg(long, default_value_t = 504)]
        train_days: usize,
        /// Out-of-sample window length in trading days.
        #[arg(long, default_value_t = 126)]
        test_days: usize,
        /// Indicator warmup rows carried into each window (default: auto —
        /// the largest window argument found in any variant).
        #[arg(long)]
        warmup_days: Option<usize>,
        /// Metric used to pick the in-sample winner.
        #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
        sort: SortArg,
    },
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
    DataAudit {
        /// Data root to audit: a local path, or `s3://bucket[/prefix]`.
        #[arg(long)]
        data: String,
        #[arg(long, default_value_t = 20000101)]
        from: i32,
        #[arg(long, default_value_t = 99991231)]
        to: i32,
        /// Emit the full report as JSON instead of the human table.
        #[arg(long)]
        json: bool,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
    },
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
    FmpSync {
        /// FMP API key (kept local). Falls back to $FMP_API_KEY if unset.
        #[arg(long)]
        api_key: Option<String>,
        /// Output data root — a local path, or an `s3://bucket[/prefix]` URL to
        /// write directly to S3/R2. Credentials resolve from the env, trying the
        /// `S3_*` vars first (R2 / static keys) then `AWS_*` (AWS S3 + IAM role,
        /// incl. `AWS_SESSION_TOKEN`) — see docs/fmp-data-source.md. Both targets
        /// produce a byte-identical tree. (`--index` needs a local path.)
        #[arg(long)]
        out: String,
        /// Comma-separated tickers, e.g. AAPL,MSFT,GOOGL. Or use --symbols-file
        /// to sync a prebuilt list, or --all-symbols for the whole universe.
        #[arg(long, value_delimiter = ',')]
        symbols: Vec<String>,
        /// Read the symbol universe from a file (one ticker per line, or a
        /// `symbol,...` CSV) — e.g. one built by `yuzu-cli fmp-symbols`.
        /// Mutually exclusive with --symbols / --all-symbols.
        #[arg(long)]
        symbols_file: Option<PathBuf>,
        /// Sync the whole screened universe (FMP screener) instead of an
        /// explicit list — the exchanges in --exchange (default US: NASDAQ,NYSE,
        /// AMEX), honoring --min-market-cap / --include-etf. Large; combine with
        /// --rate-limit / --resume. Mutually exclusive with --symbols.
        #[arg(long)]
        all_symbols: bool,
        /// Reconstruct a point-in-time index universe: sync every name that was
        /// ever a member over [from,to] (survivorship-honest, incl. names that
        /// later left), and write a `in_<index>` 0/1 membership panel to
        /// panels/. Backtest with `mask(signal, in_sp500)`. Index-scoped and
        /// degrades for very old dates (#125). Mutually exclusive with the other
        /// universe sources.
        #[arg(long, value_enum)]
        index: Option<IndexArg>,
        /// Exchanges for --all-symbols (comma-separated FMP codes). Default the
        /// three US majors; pass `all` for every exchange.
        #[arg(long, default_value = yuzu_cli::fmp::US_EXCHANGES)]
        exchange: String,
        #[arg(long, default_value_t = 20000101)]
        from: i32,
        #[arg(long, default_value_t = 20991231)]
        to: i32,
        /// Also fetch annual fundamentals (best-effort; see #51 / #53).
        #[arg(long)]
        include_fundamentals: bool,
        /// Also fetch each symbol's sector → tracked/universe.csv.gz.
        #[arg(long)]
        include_industry: bool,
        /// Also compute snapshot-factor panels (piotroski_score, altman_z,
        /// fcf_yield, analyst_upside_pct, consensus_rating) → panels/{name}.csv.gz.
        /// Current-snapshot factors for universe screening; extra FMP requests
        /// per symbol (see docs/fmp-data-source.md).
        #[arg(long)]
        include_snapshot_factors: bool,
        /// Keep ETFs and mutual/closed-end funds. By default only individual
        /// stocks are synced (non-stocks are classified via the profile
        /// endpoint and skipped).
        #[arg(long)]
        include_etf: bool,
        /// Also union FMP's delisted-companies universe (filtered by --exchange)
        /// into the symbol list, so dead names are synced too — their price
        /// files simply end at the delisting date, and the engine's delist_after
        /// forced-exit handles them. Removes survivorship bias at the data layer
        /// (#124 / #26). Note: delisted rows carry no market cap, so
        /// --min-market-cap does not filter them.
        #[arg(long)]
        include_delisted: bool,
        /// Skip symbols whose company market cap is below this, in USD (0 = off).
        /// Accepts unit suffixes: 1b, 500m, 10k, 2.5t (or a plain number / 1e9).
        #[arg(long, default_value = "0", value_parser = yuzu_cli::fmp::parse_market_cap)]
        min_market_cap: f64,
        /// Max requests per minute (0 = no throttle). Match your FMP plan's
        /// rate limit (Starter-class keys are commonly ~300/min).
        #[arg(long, default_value_t = 300)]
        rate_limit: u32,
        /// Retries per request on 429 / 5xx / transport errors.
        #[arg(long, default_value_t = 4)]
        max_retries: u32,
        /// Merge fetched rows into existing files instead of overwriting
        /// (extend an existing tree's history).
        #[arg(long)]
        append: bool,
        /// Skip symbols that already have a price file (resume an interrupted
        /// run). Takes precedence over --append.
        #[arg(long)]
        resume: bool,
    },
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
    FmpSymbols {
        /// FMP API key (kept local). Falls back to $FMP_API_KEY if unset.
        #[arg(long)]
        api_key: Option<String>,
        /// Output file for the symbol list (one ticker per line).
        #[arg(long)]
        out: PathBuf,
        /// Only symbols at/above this company market cap, in USD (0 = no floor).
        /// Accepts unit suffixes: 1b, 500m, 10k, 2.5t (or a plain number / 1e9).
        #[arg(long, default_value = "0", value_parser = yuzu_cli::fmp::parse_market_cap)]
        min_market_cap: f64,
        /// Restrict to one or more exchanges (comma-separated FMP codes).
        /// Default the three US majors (NASDAQ,NYSE,AMEX); pass `all` for every
        /// exchange.
        #[arg(long, default_value = yuzu_cli::fmp::US_EXCHANGES)]
        exchange: String,
        /// Include ETFs and funds (default: stocks only).
        #[arg(long)]
        include_etf: bool,
        /// Also append FMP's delisted-companies universe (filtered by
        /// --exchange) to the list, so a whole-market backtest is
        /// survivorship-honest (#124 / #26). Delisted rows carry no market cap,
        /// so --min-market-cap does not filter them.
        #[arg(long)]
        include_delisted: bool,
        /// Cap the number of symbols returned (default: the API's).
        #[arg(long)]
        limit: Option<usize>,
        /// Max requests per minute (0 = no throttle).
        #[arg(long, default_value_t = 300)]
        rate_limit: u32,
        /// Retries on 429 / 5xx / transport errors.
        #[arg(long, default_value_t = 4)]
        max_retries: u32,
    },
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
    EodhdSync {
        /// EODHD API token (kept local). Falls back to $EODHD_API_TOKEN or
        /// $EODHD_API_KEY if unset.
        #[arg(long)]
        api_token: Option<String>,
        /// Output data root — local path or `s3://bucket[/prefix]` (same
        /// credential chain as fmp-sync --out).
        #[arg(long)]
        out: String,
        /// Comma-separated tickers (`AAPL` or `AAPL.US`). Or use --symbols-file / --index.
        #[arg(long, value_delimiter = ',')]
        symbols: Vec<String>,
        /// Read symbols from a file (one per line, or comma-separated; `#` comments).
        #[arg(long)]
        symbols_file: Option<PathBuf>,
        /// Sync ever-members of an index over [from,to] and write `panels/in_sp500`
        /// (local --out only). Mutually exclusive with --symbols / --symbols-file.
        #[arg(long, value_enum)]
        index: Option<EodhdIndexArg>,
        /// Default exchange when a bare ticker is given (default: US → `AAPL.US`).
        #[arg(long, default_value = yuzu_cli::eodhd::DEFAULT_EXCHANGE)]
        exchange: String,
        #[arg(long, default_value_t = 20000101)]
        from: i32,
        #[arg(long, default_value_t = 20991231)]
        to: i32,
        /// Also densify annual statement factors → fundamentals/{SYM}.csv.gz
        /// (filing-date visibility; pe/ps/pb/market_cap left NaN historically).
        #[arg(long)]
        include_fundamentals: bool,
        /// Also fetch sector map → tracked/universe.csv.gz from fundamentals General.
        #[arg(long)]
        include_industry: bool,
        /// Also union EODHD delisted names for `--exchange` into the sync list
        /// (survivorship-honest price files). Uses exchange-symbol-list?delisted=1.
        #[arg(long)]
        include_delisted: bool,
        /// Max requests per minute (0 = no throttle).
        #[arg(long, default_value_t = 0)]
        rate_limit: u32,
        /// Retries on 429 / 5xx / transport errors.
        #[arg(long, default_value_t = 4)]
        max_retries: u32,
        /// Merge fetched rows into existing files instead of overwriting.
        #[arg(long)]
        append: bool,
        /// Skip symbols that already have a price file.
        #[arg(long)]
        resume: bool,
    },
    /// Build a screened symbol universe from EODHD and write it to a file.
    ///
    /// Example:
    ///   yuzu-cli eodhd-symbols --api-token "$EODHD_API_TOKEN" --out ./universe.txt \
    ///     --min-market-cap 1b --exchange us --limit 200
    #[cfg(feature = "eodhd-sync")]
    EodhdSymbols {
        #[arg(long)]
        api_token: Option<String>,
        #[arg(long)]
        out: PathBuf,
        /// Only symbols at/above this market cap in USD (0 = no floor).
        /// Accepts suffixes: 1b, 500m, 10k.
        #[arg(long, default_value = "0", value_parser = yuzu_cli::eodhd::parse_market_cap)]
        min_market_cap: f64,
        /// Exchange filter for the screener (default: us). Pass `all` for none.
        #[arg(long, default_value = "us")]
        exchange: String,
        /// Cap the number of symbols returned.
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value_t = 0)]
        rate_limit: u32,
        #[arg(long, default_value_t = 4)]
        max_retries: u32,
    },
}

fn emit(out: &Option<PathBuf>, json: String) -> std::io::Result<()> {
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
fn run_data_audit_over(
    data: &str,
    from: i32,
    to: i32,
) -> Result<pomelo_audit::DataAuditReport, Box<dyn std::error::Error>> {
    let src = pomelo_s3::OutStore::parse(data)?;
    Ok(pomelo_audit::run_data_audit(&src, data, from, to)?)
}

#[cfg(not(feature = "fmp-sync"))]
fn run_data_audit_over(
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

fn load_grid_variants(path: &PathBuf) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(path)?;
    let grid: yuzu_cli::GridSpec = serde_json::from_str(&raw)?;
    yuzu_cli::expand_grid(&grid)
        .into_iter()
        .map(|(name, spec)| Ok((name, serde_json::to_string(&spec)?)))
        .collect::<Result<_, serde_json::Error>>()
        .map_err(Into::into)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().cmd {
        Cmd::Run {
            common,
            spec,
            price_key,
        } => {
            let cfg = common.config();
            let spec_json = std::fs::read_to_string(&spec)?;
            let report = yuzu_cli::run_single(
                &common.data,
                &spec_json,
                common.from,
                common.to,
                &cfg,
                &price_key,
            )?;
            emit(&common.out, serde_json::to_string_pretty(&report)?)?;
        }
        Cmd::Sweep {
            common,
            specs,
            sort,
            top,
            price_key,
        } => {
            let cfg = common.config();
            let raw = std::fs::read_to_string(&specs)?;
            let parsed: Vec<Variant> = serde_json::from_str(&raw)?;
            let variants: Vec<(String, String)> = parsed
                .into_iter()
                .map(|v| {
                    let spec_str = serde_json::to_string(&v.spec)?;
                    Ok((v.name, spec_str))
                })
                .collect::<Result<_, serde_json::Error>>()?;

            let mut board = yuzu_cli::run_sweep(
                &common.data,
                &variants,
                common.from,
                common.to,
                &cfg,
                &price_key,
                sort.into(),
            );
            if let Some(n) = top {
                board.truncate(n);
            }
            emit(&common.out, serde_json::to_string_pretty(&board)?)?;
        }
        Cmd::Grid {
            common,
            grid,
            sort,
            top,
            price_key,
        } => {
            let cfg = common.config();
            let variants = load_grid_variants(&grid)?;
            let mut board = yuzu_cli::run_sweep(
                &common.data,
                &variants,
                common.from,
                common.to,
                &cfg,
                &price_key,
                sort.into(),
            );
            if let Some(n) = top {
                board.truncate(n);
            }
            emit(&common.out, serde_json::to_string_pretty(&board)?)?;
        }
        Cmd::Factor {
            common,
            spec,
            horizon,
            quantiles,
            neutralize_industry,
        } => {
            let spec_json = std::fs::read_to_string(&spec)?;
            let report = yuzu_cli::run_factor(
                &common.data,
                &spec_json,
                common.from,
                common.to,
                horizon,
                quantiles,
                neutralize_industry,
            )?;
            emit(&common.out, serde_json::to_string_pretty(&report)?)?;
        }
        Cmd::Event {
            common,
            spec,
            pre,
            post,
        } => {
            let spec_json = std::fs::read_to_string(&spec)?;
            let report =
                yuzu_cli::run_event(&common.data, &spec_json, common.from, common.to, pre, post)?;
            emit(&common.out, serde_json::to_string_pretty(&report)?)?;
        }
        Cmd::Lookahead {
            common,
            spec,
            shift_days,
            profile,
        } => {
            let cfg = common.config();
            let spec_json = std::fs::read_to_string(&spec)?;
            let json = if profile {
                let report = yuzu_cli::run_lookahead_profile(
                    &common.data,
                    &spec_json,
                    common.from,
                    common.to,
                    yuzu_cli::PROFILE_SHIFTS,
                    &cfg,
                )?;
                serde_json::to_string_pretty(&report)?
            } else {
                let report = yuzu_cli::run_lookahead(
                    &common.data,
                    &spec_json,
                    common.from,
                    common.to,
                    shift_days,
                    &cfg,
                )?;
                serde_json::to_string_pretty(&report)?
            };
            emit(&common.out, json)?;
        }
        Cmd::Walkforward {
            common,
            grid,
            train_days,
            test_days,
            warmup_days,
            sort,
        } => {
            let cfg = common.config();
            let variants = load_grid_variants(&grid)?;
            let report = yuzu_cli::run_walkforward(
                &common.data,
                &variants,
                &yuzu_cli::WalkForwardParams {
                    from: common.from,
                    to: common.to,
                    train_days,
                    test_days,
                    sort_by: sort.into(),
                    warmup_days,
                },
                &cfg,
            )?;
            emit(&common.out, serde_json::to_string_pretty(&report)?)?;
        }
        Cmd::DataAudit {
            data,
            from,
            to,
            json,
            out,
        } => {
            let report = run_data_audit_over(&data, from, to)?;
            let overall = report.overall;
            let body = if json {
                serde_json::to_string_pretty(&report)?
            } else {
                pomelo_audit::render_table(&report)
            };
            emit(&out, body)?;
            // Non-zero exit on a FAIL so the audit can gate CI / a nightly job.
            if overall == pomelo_audit::Status::Fail {
                std::process::exit(2);
            }
        }
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSync {
            api_key,
            out,
            symbols,
            symbols_file,
            all_symbols,
            index,
            exchange,
            from,
            to,
            include_fundamentals,
            include_industry,
            include_snapshot_factors,
            include_etf,
            include_delisted,
            min_market_cap,
            rate_limit,
            max_retries,
            append,
            resume,
        } => {
            let api_key = api_key
                .or_else(|| std::env::var("FMP_API_KEY").ok())
                .filter(|k| !k.trim().is_empty())
                .ok_or("provide --api-key or set FMP_API_KEY")?;
            let explicit: Vec<String> = symbols
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            // Exactly one universe source: --symbols, --symbols-file, --all-symbols, --index.
            let sources = [
                !explicit.is_empty(),
                symbols_file.is_some(),
                all_symbols,
                index.is_some(),
            ]
            .iter()
            .filter(|b| **b)
            .count();
            if sources > 1 {
                return Err(
                    "choose one of --symbols, --symbols-file, --all-symbols, or --index".into(),
                );
            }
            if sources == 0 {
                return Err(
                    "provide --symbols AAPL,MSFT,..., --symbols-file <path>, --all-symbols, or --index <sp500|nasdaq|dowjones>"
                        .into(),
                );
            }
            let mode = if resume {
                yuzu_cli::fmp::WriteMode::Resume
            } else if append {
                yuzu_cli::fmp::WriteMode::Append
            } else {
                yuzu_cli::fmp::WriteMode::Overwrite
            };
            let cfg = yuzu_cli::fmp::SyncConfig {
                from,
                to,
                include_fundamentals,
                include_industry,
                include_snapshot_factors,
                skip_non_stocks: !include_etf,
                min_market_cap,
                rate_limit_per_min: rate_limit,
                max_retries,
                backoff_base: std::time::Duration::from_secs(2),
                mode,
            };
            let client = yuzu_cli::fmp::UreqClient::new();
            // For --index, fetch the reconstructor up front: its ever-members are
            // the sync universe, and it's reused after the sync to write the
            // membership panel over the resulting price calendar.
            let membership = match index {
                Some(idx) => Some(yuzu_cli::fmp::IndexMembership::fetch(
                    &client,
                    &api_key,
                    idx.into(),
                    &cfg,
                )?),
                None => None,
            };
            let mut symbols = if let Some(m) = &membership {
                eprintln!(
                    "reconstructing {} point-in-time membership…",
                    m.series_name()
                );
                let ever = m.ever_members(from, to);
                eprintln!(
                    "index universe: {} ever-members over [{from}, {to}]",
                    ever.len()
                );
                ever
            } else if all_symbols {
                let filter = yuzu_cli::fmp::SymbolFilter {
                    min_market_cap,
                    exchange: Some(exchange.clone()),
                    include_etf,
                    limit: None,
                };
                eprintln!("building symbol universe from FMP screener…");
                let all = yuzu_cli::fmp::build_symbol_list(&client, &api_key, &cfg, &filter)?;
                eprintln!("universe: {} symbols", all.len());
                all
            } else if let Some(path) = symbols_file {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("reading {}: {e}", path.display()))?;
                let syms = yuzu_cli::fmp::parse_symbols_list(&text);
                if syms.is_empty() {
                    return Err(format!("no symbols in {}", path.display()).into());
                }
                eprintln!("universe: {} symbols from {}", syms.len(), path.display());
                syms
            } else {
                explicit
            };
            if include_delisted {
                eprintln!("fetching delisted universe from FMP…");
                let delisted =
                    yuzu_cli::fmp::fetch_delisted(&client, &api_key, &cfg, Some(&exchange))?;
                let mut seen: std::collections::HashSet<String> = symbols.iter().cloned().collect();
                let before = symbols.len();
                for d in delisted {
                    if seen.insert(d.symbol.clone()) {
                        symbols.push(d.symbol);
                    }
                }
                eprintln!("delisted: +{} names unioned in", symbols.len() - before);
            }
            let out_store = pomelo_s3::OutStore::parse(&out)?;
            if membership.is_some() && out_store.is_s3() {
                return Err("--index requires a local --out (the membership panel write reads a local trading calendar); sync to a local path, or omit --index".into());
            }
            let summary = yuzu_cli::fmp::sync_into(&client, &api_key, &symbols, &out_store, &cfg)?;
            eprintln!(
                "done: {} written, {} skipped, {} filtered, {} price rows, {} fundamentals, {} failures",
                summary.symbols_written,
                summary.symbols_skipped,
                summary.symbols_filtered,
                summary.price_rows,
                summary.fundamentals_written,
                summary.failures.len(),
            );
            for (sym, err) in &summary.failures {
                eprintln!("  FAILED {sym}: {err}");
            }
            if summary.symbols_written == 0 {
                return Err("no symbols were written".into());
            }
            // Now that prices exist (a trading calendar), write the PIT
            // membership panel over exactly those days.
            if let Some(m) = &membership {
                // Guarded above: reaching here means `out` is a local path.
                let (days, cols) =
                    yuzu_cli::fmp::write_index_membership(std::path::Path::new(&out), m, from, to)?;
                eprintln!(
                    "wrote panels/{}.csv.gz: {days} days × {cols} symbols (mask with mask(signal, {}))",
                    m.series_name(),
                    m.series_name()
                );
            }
        }
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSymbols {
            api_key,
            out,
            min_market_cap,
            exchange,
            include_etf,
            include_delisted,
            limit,
            rate_limit,
            max_retries,
        } => {
            let api_key = api_key
                .or_else(|| std::env::var("FMP_API_KEY").ok())
                .filter(|k| !k.trim().is_empty())
                .ok_or("provide --api-key or set FMP_API_KEY")?;
            let cfg = yuzu_cli::fmp::SyncConfig {
                rate_limit_per_min: rate_limit,
                max_retries,
                backoff_base: std::time::Duration::from_secs(2),
                ..Default::default()
            };
            let filter = yuzu_cli::fmp::SymbolFilter {
                min_market_cap,
                exchange: Some(exchange.clone()),
                include_etf,
                limit,
            };
            let client = yuzu_cli::fmp::UreqClient::new();
            eprintln!("building symbol universe from FMP screener…");
            let mut syms = yuzu_cli::fmp::build_symbol_list(&client, &api_key, &cfg, &filter)?;
            if syms.is_empty() {
                return Err("screener returned no symbols (loosen the filters?)".into());
            }
            if include_delisted {
                eprintln!("fetching delisted universe from FMP…");
                let delisted =
                    yuzu_cli::fmp::fetch_delisted(&client, &api_key, &cfg, Some(&exchange))?;
                let mut seen: std::collections::HashSet<String> = syms.iter().cloned().collect();
                let before = syms.len();
                for d in delisted {
                    if seen.insert(d.symbol.clone()) {
                        syms.push(d.symbol);
                    }
                }
                syms.sort();
                eprintln!("delisted: +{} names appended", syms.len() - before);
            }
            let mut body = String::from("# symbols built by `yuzu-cli fmp-symbols`\n");
            for s in &syms {
                body.push_str(s);
                body.push('\n');
            }
            std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
            eprintln!("wrote {} symbols to {}", syms.len(), out.display());
        }
        #[cfg(feature = "eodhd-sync")]
        Cmd::EodhdSync {
            api_token,
            out,
            symbols,
            symbols_file,
            index,
            exchange,
            from,
            to,
            include_fundamentals,
            include_industry,
            include_delisted,
            rate_limit,
            max_retries,
            append,
            resume,
        } => {
            let api_token = api_token
                .or_else(|| std::env::var("EODHD_API_TOKEN").ok())
                .or_else(|| std::env::var("EODHD_API_KEY").ok())
                .filter(|k| !k.trim().is_empty())
                .ok_or("provide --api-token or set EODHD_API_TOKEN (or EODHD_API_KEY)")?;
            let had_symbols_flag = !symbols.is_empty();
            let had_file = symbols_file.is_some();
            let mut explicit: Vec<String> = symbols
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(path) = symbols_file {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("reading {}: {e}", path.display()))?;
                explicit.extend(yuzu_cli::eodhd::parse_symbols_list(&text));
            }
            // De-dupe while preserving order.
            let mut seen = std::collections::HashSet::new();
            explicit.retain(|s| seen.insert(s.clone()));

            if (had_symbols_flag || had_file) && index.is_some() {
                return Err("choose one of --symbols/--symbols-file or --index (not both)".into());
            }

            let mode = if resume {
                yuzu_cli::eodhd::WriteMode::Resume
            } else if append {
                yuzu_cli::eodhd::WriteMode::Append
            } else {
                yuzu_cli::eodhd::WriteMode::Overwrite
            };
            let cfg = yuzu_cli::eodhd::SyncConfig {
                from,
                to,
                default_exchange: exchange.clone(),
                include_fundamentals,
                include_industry,
                rate_limit_per_min: rate_limit,
                max_retries,
                backoff_base: std::time::Duration::from_secs(2),
                mode,
            };
            let client = yuzu_cli::eodhd::UreqClient::new();

            let membership = if let Some(idx) = index {
                eprintln!("fetching EODHD index membership…");
                let m =
                    yuzu_cli::eodhd::IndexMembership::fetch(&client, &api_token, idx.into(), &cfg)?;
                explicit = m.ever_members(from, to);
                seen = explicit.iter().cloned().collect();
                eprintln!(
                    "index {}: {} ever-members over [{from},{to}]",
                    m.series_name(),
                    explicit.len()
                );
                Some(m)
            } else {
                None
            };

            if include_delisted {
                eprintln!("fetching delisted universe from EODHD ({exchange})…");
                let delisted =
                    yuzu_cli::eodhd::fetch_delisted(&client, &api_token, &cfg, &exchange)?;
                let before = explicit.len();
                for d in delisted {
                    if seen.insert(d.symbol.clone()) {
                        explicit.push(d.symbol);
                    }
                }
                eprintln!("delisted: +{} names unioned in", explicit.len() - before);
            }
            if explicit.is_empty() {
                return Err(
                    "provide --symbols and/or --symbols-file, or --index, or --include-delisted"
                        .into(),
                );
            }
            let out_store = pomelo_s3::OutStore::parse(&out)?;
            if membership.is_some() && out_store.is_s3() {
                return Err(
                    "--index requires a local --out (membership panel write needs a local trading calendar)"
                        .into(),
                );
            }
            let summary =
                yuzu_cli::eodhd::sync_into(&client, &api_token, &explicit, &out_store, &cfg)?;
            eprintln!(
                "done: {} written, {} skipped, {} filtered, {} price rows, {} fundamentals, industry={}, {} failures",
                summary.symbols_written,
                summary.symbols_skipped,
                summary.symbols_filtered,
                summary.price_rows,
                summary.fundamentals_written,
                summary.industry_written,
                summary.failures.len(),
            );
            for (sym, err) in &summary.failures {
                eprintln!("  FAILED {sym}: {err}");
            }
            if summary.symbols_written == 0 {
                return Err("no symbols were written".into());
            }
            if let Some(m) = &membership {
                let (days, cols) = yuzu_cli::eodhd::write_index_membership(
                    std::path::Path::new(&out),
                    m,
                    from,
                    to,
                )?;
                eprintln!(
                    "wrote panels/{}.csv.gz: {days} days × {cols} symbols (mask with mask(signal, {}))",
                    m.series_name(),
                    m.series_name()
                );
            }
        }
        #[cfg(feature = "eodhd-sync")]
        Cmd::EodhdSymbols {
            api_token,
            out,
            min_market_cap,
            exchange,
            limit,
            rate_limit,
            max_retries,
        } => {
            let api_token = api_token
                .or_else(|| std::env::var("EODHD_API_TOKEN").ok())
                .or_else(|| std::env::var("EODHD_API_KEY").ok())
                .filter(|k| !k.trim().is_empty())
                .ok_or("provide --api-token or set EODHD_API_TOKEN (or EODHD_API_KEY)")?;
            let cfg = yuzu_cli::eodhd::SyncConfig {
                rate_limit_per_min: rate_limit,
                max_retries,
                backoff_base: std::time::Duration::from_secs(2),
                ..Default::default()
            };
            let filter = yuzu_cli::eodhd::SymbolFilter {
                min_market_cap,
                exchange,
                limit,
            };
            let client = yuzu_cli::eodhd::UreqClient::new();
            eprintln!("building symbol universe from EODHD screener…");
            let syms = yuzu_cli::eodhd::build_symbol_list(&client, &api_token, &cfg, &filter)?;
            if syms.is_empty() {
                return Err("screener returned no symbols (loosen the filters?)".into());
            }
            let mut body = String::from("# symbols built by `yuzu-cli eodhd-symbols`\n");
            for s in &syms {
                body.push_str(s);
                body.push('\n');
            }
            std::fs::write(&out, body).map_err(|e| format!("writing {}: {e}", out.display()))?;
            eprintln!("wrote {} symbols to {}", syms.len(), out.display());
        }
    }
    Ok(())
}
