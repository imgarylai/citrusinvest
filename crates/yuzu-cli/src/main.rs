use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

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

#[derive(Subcommand)]
enum Cmd {
    /// Run one strategy over the full universe.
    Run {
        /// Directory mirroring R2's prices/ tree.
        #[arg(long)]
        data: PathBuf,
        /// Path to a JSON Expr spec file.
        #[arg(long)]
        spec: PathBuf,
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
        /// Treat a symbol as delisted after N consecutive missing-price days (0 = off).
        #[arg(long, default_value_t = 0)]
        delist_after: usize,
        /// Fraction of a delisted position written off (0 = exit at last price, 1 = total loss).
        #[arg(long, default_value_t = 0.0)]
        delist_haircut: f64,
        /// Benchmark symbol (its closes are loaded and compared), e.g. SPY.
        #[arg(long)]
        benchmark: Option<String>,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run many strategy variants in parallel and emit a ranked leaderboard.
    Sweep {
        /// Directory mirroring R2's prices/ tree.
        #[arg(long)]
        data: PathBuf,
        /// Path to a JSON file: `[{"name": "...", "spec": {...Expr...}}, ...]`.
        #[arg(long)]
        specs: PathBuf,
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
        /// Treat a symbol as delisted after N consecutive missing-price days (0 = off).
        #[arg(long, default_value_t = 0)]
        delist_after: usize,
        /// Fraction of a delisted position written off (0 = exit at last price, 1 = total loss).
        #[arg(long, default_value_t = 0.0)]
        delist_haircut: f64,
        /// Benchmark symbol (its closes are loaded and compared), e.g. SPY.
        #[arg(long)]
        benchmark: Option<String>,
        /// Metric to rank by.
        #[arg(long, value_enum, default_value_t = SortArg::Sharpe)]
        sort: SortArg,
        /// Keep only the top N entries (default: all).
        #[arg(long)]
        top: Option<usize>,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().cmd {
        Cmd::Run {
            data,
            spec,
            from,
            to,
            fee_ratio,
            slippage_ratio,
            initial_capital,
            max_participation,
            delist_after,
            delist_haircut,
            benchmark,
            out,
        } => {
            let cfg = yuzu_core::backtest::BacktestConfig {
                fee_ratio,
                slippage_ratio,
                initial_capital,
                max_participation,
                delist_after,
                delist_haircut,
                benchmark_key: benchmark,
                ..Default::default()
            };
            let spec_json = std::fs::read_to_string(&spec)?;
            let report = yuzu_cli::run_single(&data, &spec_json, from, to, &cfg)?;
            let json = serde_json::to_string_pretty(&report)?;
            match out {
                Some(p) => std::fs::write(p, json)?,
                None => println!("{json}"),
            }
        }
        Cmd::Sweep {
            data,
            specs,
            from,
            to,
            fee_ratio,
            slippage_ratio,
            initial_capital,
            max_participation,
            delist_after,
            delist_haircut,
            benchmark,
            sort,
            top,
            out,
        } => {
            let cfg = yuzu_core::backtest::BacktestConfig {
                fee_ratio,
                slippage_ratio,
                initial_capital,
                max_participation,
                delist_after,
                delist_haircut,
                benchmark_key: benchmark,
                ..Default::default()
            };
            let raw = std::fs::read_to_string(&specs)?;
            let parsed: Vec<Variant> = serde_json::from_str(&raw)?;
            let variants: Vec<(String, String)> = parsed
                .into_iter()
                .map(|v| {
                    let spec_str = serde_json::to_string(&v.spec)?;
                    Ok((v.name, spec_str))
                })
                .collect::<Result<_, serde_json::Error>>()?;

            let mut board = yuzu_cli::run_sweep(&data, &variants, from, to, &cfg, sort.into());
            if let Some(n) = top {
                board.truncate(n);
            }
            let json = serde_json::to_string_pretty(&board)?;
            match out {
                Some(p) => std::fs::write(p, json)?,
                None => println!("{json}"),
            }
        }
    }
    Ok(())
}
