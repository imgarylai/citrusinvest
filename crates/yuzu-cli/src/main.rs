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
    /// Output file (default: stdout).
    #[arg(long)]
    out: Option<PathBuf>,
}

impl CommonArgs {
    fn config(&self) -> yuzu_core::backtest::BacktestConfig {
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
        /// Output data root: writes prices/ (+ fundamentals/, tracked/ if asked).
        #[arg(long)]
        out: PathBuf,
        /// Comma-separated tickers, e.g. AAPL,MSFT,GOOGL. Or use --symbols-file
        /// to sync a prebuilt list, or --all-symbols for the whole universe.
        #[arg(long, value_delimiter = ',')]
        symbols: Vec<String>,
        /// Read the symbol universe from a file (one ticker per line, or a
        /// `symbol,...` CSV) — e.g. one built by `yuzu-cli fmp-symbols`.
        /// Mutually exclusive with --symbols / --all-symbols.
        #[arg(long)]
        symbols_file: Option<PathBuf>,
        /// Sync every symbol FMP lists (its full stock universe) instead of an
        /// explicit --symbols list. Large — combine with --min-market-cap /
        /// --rate-limit / --resume. Mutually exclusive with --symbols.
        #[arg(long)]
        all_symbols: bool,
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
        /// Keep ETFs and mutual/closed-end funds. By default only individual
        /// stocks are synced (non-stocks are classified via the profile
        /// endpoint and skipped).
        #[arg(long)]
        include_etf: bool,
        /// Skip symbols whose company market cap is below this (0 = off).
        /// Reads the profile endpoint's marketCap.
        #[arg(long, default_value_t = 0.0)]
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
        /// Only symbols at/above this company market cap (0 = no floor).
        #[arg(long, default_value_t = 0.0)]
        min_market_cap: f64,
        /// Restrict to one or more exchanges (comma-separated FMP codes,
        /// e.g. NASDAQ,NYSE). Default: all exchanges.
        #[arg(long)]
        exchange: Option<String>,
        /// Include ETFs and funds (default: stocks only).
        #[arg(long)]
        include_etf: bool,
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
        Cmd::Run { common, spec } => {
            let cfg = common.config();
            let spec_json = std::fs::read_to_string(&spec)?;
            let report =
                yuzu_cli::run_single(&common.data, &spec_json, common.from, common.to, &cfg)?;
            emit(&common.out, serde_json::to_string_pretty(&report)?)?;
        }
        Cmd::Sweep {
            common,
            specs,
            sort,
            top,
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
        } => {
            let cfg = common.config();
            let variants = load_grid_variants(&grid)?;
            let mut board = yuzu_cli::run_sweep(
                &common.data,
                &variants,
                common.from,
                common.to,
                &cfg,
                sort.into(),
            );
            if let Some(n) = top {
                board.truncate(n);
            }
            emit(&common.out, serde_json::to_string_pretty(&board)?)?;
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
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSync {
            api_key,
            out,
            symbols,
            symbols_file,
            all_symbols,
            from,
            to,
            include_fundamentals,
            include_industry,
            include_etf,
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
            // Exactly one universe source: --symbols, --symbols-file, or --all-symbols.
            let sources = [!explicit.is_empty(), symbols_file.is_some(), all_symbols]
                .iter()
                .filter(|b| **b)
                .count();
            if sources > 1 {
                return Err("choose one of --symbols, --symbols-file, or --all-symbols".into());
            }
            if sources == 0 {
                return Err(
                    "provide --symbols AAPL,MSFT,..., --symbols-file <path>, or --all-symbols"
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
                skip_non_stocks: !include_etf,
                min_market_cap,
                rate_limit_per_min: rate_limit,
                max_retries,
                backoff_base: std::time::Duration::from_secs(2),
                mode,
            };
            let client = yuzu_cli::fmp::UreqClient::new();
            let symbols = if all_symbols {
                eprintln!("fetching full FMP symbol universe…");
                let all = yuzu_cli::fmp::list_all_symbols(&client, &api_key, &cfg)?;
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
            let summary = yuzu_cli::fmp::sync(&client, &api_key, &symbols, &out, &cfg)?;
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
        }
        #[cfg(feature = "fmp-sync")]
        Cmd::FmpSymbols {
            api_key,
            out,
            min_market_cap,
            exchange,
            include_etf,
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
                exchange,
                include_etf,
                limit,
            };
            let client = yuzu_cli::fmp::UreqClient::new();
            eprintln!("building symbol universe from FMP screener…");
            let syms = yuzu_cli::fmp::build_symbol_list(&client, &api_key, &cfg, &filter)?;
            if syms.is_empty() {
                return Err("screener returned no symbols (loosen the filters?)".into());
            }
            let mut body = String::from("# symbols built by `yuzu-cli fmp-symbols`\n");
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
