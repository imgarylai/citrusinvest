//! Native batch backtest runner. Logic lives here (testable); `main.rs` is a thin
//! clap front end. Reads a locally-synced mirror of R2's `prices/` tree.

use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::report::Report;
use yuzu_core::{run_backtest, EvalContext};
use yuzu_data::{load_panel, Field, LocalSource, PRICES_DIR};

/// Symbols that have a `prices/<sym>.csv.gz` file under `root`, sorted.
pub fn list_symbols(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let prices = root.join(PRICES_DIR);
    if !prices.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(prices)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(sym) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_suffix(".csv.gz"))
        {
            out.push(sym.to_string());
        }
    }
    out.sort();
    Ok(out)
}

/// Load the close panel for every symbol under `root` into an `EvalContext`.
/// (Scope a run to a subset by syncing only those files into the data dir.)
pub(crate) fn load_close(root: &Path, from: i32, to: i32) -> Result<EvalContext, String> {
    let syms = list_symbols(root).map_err(|e| e.to_string())?;
    let src = LocalSource::new(root);
    let panel = load_panel(&src, &syms, Field::AdjClose, from, to, PRICES_DIR)
        .map_err(|e| e.to_string())?;
    let mut panels = HashMap::new();
    panels.insert("close".to_string(), panel);
    Ok(EvalContext {
        panels,
        industry: HashMap::new(),
    })
}

/// Which metric to rank by in a sweep.
#[derive(Clone, Copy)]
pub enum SortKey {
    Sharpe,
    TotalReturn,
    Cagr,
    Calmar,
}

/// One row in the sweep leaderboard.
#[derive(Serialize)]
pub struct SweepEntry {
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub calmar: f64,
}

fn failed(name: &str, err: String) -> SweepEntry {
    SweepEntry {
        name: name.to_string(),
        ok: false,
        error: Some(err),
        total_return: f64::NAN,
        cagr: f64::NAN,
        sharpe: f64::NAN,
        sortino: f64::NAN,
        max_drawdown: f64::NAN,
        calmar: f64::NAN,
    }
}

/// Run many strategy variants in parallel (Rayon) and return a ranked leaderboard.
///
/// The panel is loaded once and shared across all parallel workers.
/// Successful entries come first, sorted descending by `sort_by`; failures sink last.
pub fn run_sweep(
    root: &Path,
    variants: &[(String, String)],
    from: i32,
    to: i32,
    fee_ratio: f64,
    sort_by: SortKey,
) -> Vec<SweepEntry> {
    let ctx = match load_close(root, from, to) {
        Ok(v) => v,
        Err(e) => return variants.iter().map(|(n, _)| failed(n, e.clone())).collect(),
    };
    let cfg = BacktestConfig {
        fee_ratio,
        ..Default::default()
    };

    let mut board: Vec<SweepEntry> = variants
        .par_iter()
        .map(
            |(name, spec)| match run_backtest(spec, &ctx, "close", &cfg) {
                Ok(r) => SweepEntry {
                    name: name.clone(),
                    ok: true,
                    error: None,
                    total_return: r.metrics.total_return,
                    cagr: r.metrics.cagr,
                    sharpe: r.metrics.sharpe,
                    sortino: r.metrics.sortino,
                    max_drawdown: r.metrics.max_drawdown,
                    calmar: r.metrics.calmar,
                },
                Err(e) => failed(name, e.to_string()),
            },
        )
        .collect();

    let key = |e: &SweepEntry| match sort_by {
        SortKey::Sharpe => e.sharpe,
        SortKey::TotalReturn => e.total_return,
        SortKey::Cagr => e.cagr,
        SortKey::Calmar => e.calmar,
    };
    // ok entries first, then non-NaN metrics before NaN, then by metric descending;
    // failures and NaN-metric runs sink to the bottom.
    board.sort_by(|a, b| {
        b.ok.cmp(&a.ok)
            .then(key(a).is_nan().cmp(&key(b).is_nan()))
            .then(
                key(b)
                    .partial_cmp(&key(a))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    board
}

/// Run one strategy over the full (or specified) universe.
pub fn run_single(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    fee_ratio: f64,
) -> Result<Report, String> {
    let ctx = load_close(root, from, to)?;
    let cfg = BacktestConfig {
        fee_ratio,
        ..Default::default()
    };
    run_backtest(spec_json, &ctx, "close", &cfg).map_err(|e| e.to_string())
}
