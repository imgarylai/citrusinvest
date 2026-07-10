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
    }
    Ok(())
}
