//! Single run and parameter sweep: run one strategy, or many variants in
//! parallel ranked into a leaderboard.

use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::report::Report;
use yuzu_core::run_backtest;

use crate::ctx::{load_ctx, referenced_series};

/// Which metric to rank by in a sweep (also the walk-forward selection metric).
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

/// Run one strategy over the full universe, or over an explicit `symbols`
/// subset (`None` = every symbol under `prices/`). Scoping changes what every
/// cross-sectional op sees, so a requested symbol missing from the data tree
/// is an error, not a silent drop. Note: a symbol list frozen *today* implies
/// survivorship bias in a historical run — see `docs/strategy-envelope.md`.
pub fn run_single(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    cfg: &BacktestConfig,
    price_key: &str,
    symbols: Option<&[String]>,
) -> Result<Report, String> {
    let ctx = load_ctx(
        root,
        from,
        to,
        cfg,
        price_key,
        symbols,
        &referenced_series(&[spec_json]),
    )?;
    run_backtest(spec_json, &ctx, price_key, cfg).map_err(|e| e.to_string())
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
    cfg: &BacktestConfig,
    price_key: &str,
    sort_by: SortKey,
) -> Vec<SweepEntry> {
    let specs: Vec<&str> = variants.iter().map(|(_, s)| s.as_str()).collect();
    let ctx = match load_ctx(
        root,
        from,
        to,
        cfg,
        price_key,
        None,
        &referenced_series(&specs),
    ) {
        Ok(v) => v,
        Err(e) => return variants.iter().map(|(n, _)| failed(n, e.clone())).collect(),
    };

    let mut board: Vec<SweepEntry> = variants
        .par_iter()
        .map(
            |(name, spec)| match run_backtest(spec, &ctx, price_key, cfg) {
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
