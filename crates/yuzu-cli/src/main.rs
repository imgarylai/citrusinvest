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
        Cmd::Run { data, spec, from, to, fee_ratio, out } => {
            let spec_json = std::fs::read_to_string(&spec)?;
            let report = yuzu_cli::run_single(&data, &spec_json, from, to, fee_ratio)?;
            let json = serde_json::to_string_pretty(&report)?;
            match out {
                Some(p) => std::fs::write(p, json)?,
                None => println!("{json}"),
            }
        }
        Cmd::Sweep { data, specs, from, to, fee_ratio, sort, top, out } => {
            let raw = std::fs::read_to_string(&specs)?;
            let parsed: Vec<Variant> = serde_json::from_str(&raw)?;
            let variants: Vec<(String, String)> = parsed
                .into_iter()
                .map(|v| {
                    let spec_str = serde_json::to_string(&v.spec)?;
                    Ok((v.name, spec_str))
                })
                .collect::<Result<_, serde_json::Error>>()?;

            let mut board = yuzu_cli::run_sweep(&data, &variants, from, to, fee_ratio, sort.into());
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
