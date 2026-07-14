//! Walk-forward analysis: roll a train/test window over the calendar, pick the
//! best variant in-sample, evaluate it out-of-sample, and chain the OOS equity.

use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::EvalContext;

use crate::ctx::{load_ctx, referenced_series};
use crate::sweep::SortKey;

/// One walk-forward window: variant selection happened on the train range,
/// evaluation on the (out-of-sample) test range.
#[derive(Serialize)]
pub struct WalkForwardWindow {
    pub train_from: i32,
    pub train_to: i32,
    pub test_from: i32,
    pub test_to: i32,
    /// Variant chosen in-sample (grid name, e.g. `"n=20"`).
    pub chosen: String,
    /// The chosen variant's in-sample value of the selection metric.
    pub in_sample_metric: f64,
    /// The chosen variant's out-of-sample return over the test range.
    pub oos_return: f64,
}

/// Stitched out-of-sample result across all walk-forward windows.
#[derive(Serialize)]
pub struct WalkForwardReport {
    pub windows: Vec<WalkForwardWindow>,
    /// Out-of-sample equity, chained across test windows (base 1.0).
    pub dates: Vec<i32>,
    pub equity: Vec<f64>,
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub max_drawdown: f64,
}

fn slice_ctx(ctx: &EvalContext, from: i32, to: i32) -> EvalContext {
    EvalContext {
        panels: ctx
            .panels
            .iter()
            .map(|(k, p)| (k.clone(), p.slice_dates(from, to)))
            .collect(),
        industry: ctx.industry.clone(),
    }
}

fn metric_from_curve(dates: &[i32], equity: &[f64], key: SortKey) -> f64 {
    match key {
        SortKey::Sharpe => yuzu_core::metrics::sharpe(equity),
        SortKey::TotalReturn => yuzu_core::metrics::total_return(equity),
        SortKey::Cagr => yuzu_core::metrics::cagr(equity, dates),
        SortKey::Calmar => yuzu_core::metrics::calmar(equity, dates),
    }
}

/// Largest window argument anywhere in a spec tree (the `n` / `nwindow` / `d`
/// fields) — the auto value for walk-forward warmup.
pub fn max_lookback(spec: &serde_json::Value) -> usize {
    match spec {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let own = if matches!(k.as_str(), "n" | "nwindow" | "d") {
                    v.as_u64().unwrap_or(0) as usize
                } else {
                    0
                };
                own.max(max_lookback(v))
            })
            .max()
            .unwrap_or(0),
        serde_json::Value::Array(arr) => arr.iter().map(max_lookback).max().unwrap_or(0),
        _ => 0,
    }
}

/// Evaluate `spec` over `[eval_from, to]` but run the NAV loop only over
/// `[nav_from, to]`: indicators warm up on the earlier rows, P&L does not
/// include them. Returns the NAV range's (dates, equity).
#[allow(clippy::type_complexity)]
fn run_windowed(
    ctx: &EvalContext,
    spec: &str,
    cfg: &BacktestConfig,
    eval_from: i32,
    nav_from: i32,
    to: i32,
    initial_weights: Option<&HashMap<String, f64>>,
) -> Result<(Vec<i32>, Vec<f64>, HashMap<String, f64>), String> {
    let eval_ctx = slice_ctx(ctx, eval_from, to);
    let positions = yuzu_core::run_strategy(spec, &eval_ctx).map_err(|e| e.to_string())?;
    let positions = positions.slice_dates(nav_from, to);
    let prices = eval_ctx
        .panels
        .get("close")
        .ok_or("no close panel")?
        .slice_dates(nav_from, to);
    let volume = eval_ctx
        .panels
        .get("volume")
        .map(|p| p.slice_dates(nav_from, to));
    let run = yuzu_core::backtest::run_with_initial(
        &positions,
        &prices,
        None, // open (unused by walk-forward; stops here fall back to level fills)
        None,
        None,
        volume.as_ref(),
        cfg,
        initial_weights,
    );
    Ok((run.dates, run.equity, run.terminal_weights))
}

/// Window/selection settings for [`run_walkforward`].
pub struct WalkForwardParams {
    pub from: i32,
    pub to: i32,
    /// In-sample window length in trading days.
    pub train_days: usize,
    /// Out-of-sample window length in trading days.
    pub test_days: usize,
    /// Metric used to pick the in-sample winner.
    pub sort_by: SortKey,
    /// Indicator warmup rows carried into each window from before its start.
    /// `None` = auto: the largest window argument found in any variant's spec.
    pub warmup_days: Option<usize>,
}

/// Walk-forward analysis: roll a `train_days`/`test_days` window (in trading
/// days) over the close panel's date axis; in each window run every variant on
/// the train slice, pick the best by `sort_by`, run it on the test slice, and
/// chain the out-of-sample equity segments into one curve.
///
/// Indicators **warm up on the rows before each window** (`warmup_days`,
/// auto = the largest window argument in any variant) while P&L is counted
/// only inside the window, and each test segment also prices the boundary-day
/// return from the previous window's last close. The first train window has
/// no earlier data, so it still starts cold.
pub fn run_walkforward(
    root: &Path,
    variants: &[(String, String)],
    params: &WalkForwardParams,
    cfg: &BacktestConfig,
) -> Result<WalkForwardReport, String> {
    let WalkForwardParams {
        from,
        to,
        train_days,
        test_days,
        sort_by,
        warmup_days,
    } = *params;
    if variants.is_empty() {
        return Err("no variants to select from".into());
    }
    if train_days == 0 || test_days == 0 {
        return Err("train_days and test_days must be > 0".into());
    }
    let warmup = match warmup_days {
        Some(n) => n,
        None => variants
            .iter()
            .filter_map(|(_, spec)| serde_json::from_str(spec).ok())
            .map(|v: serde_json::Value| max_lookback(&v))
            .max()
            .unwrap_or(0),
    };
    let specs: Vec<&str> = variants.iter().map(|(_, s)| s.as_str()).collect();
    let ctx = load_ctx(
        root,
        from,
        to,
        cfg,
        "close",
        None,
        &referenced_series(&specs),
    )?;
    let dates = ctx
        .panels
        .get("close")
        .ok_or("no close panel")?
        .dates
        .clone();
    if dates.len() < train_days + 1 {
        return Err(format!(
            "only {} trading days loaded; need > train_days ({train_days})",
            dates.len()
        ));
    }

    let mut windows = Vec::new();
    let mut oos_dates: Vec<i32> = Vec::new();
    let mut oos_equity: Vec<f64> = Vec::new();
    let mut scale = 1.0_f64;
    // Book carried across OOS seams: the previous test segment's terminal
    // holdings become the next segment's starting book, so a seam that keeps
    // the same names pays turnover only on the difference (#21). `None` for the
    // first segment (it enters flat). Only affects cost-bearing runs — with
    // zero fees/slippage the stitched curve is unchanged.
    let mut carry: Option<HashMap<String, f64>> = None;

    let mut start = 0usize;
    while start + train_days < dates.len() {
        let train_from = dates[start];
        let train_to = dates[start + train_days - 1];
        let test_start = start + train_days;
        let test_end = (test_start + test_days).min(dates.len());
        let test_from = dates[test_start];
        let test_to = dates[test_end - 1];

        // in-sample: every variant in parallel; signals warm up on the rows
        // before the train window, P&L is counted inside it only.
        let train_eval_from = dates[start.saturating_sub(warmup)];
        let scored: Vec<(usize, f64)> = variants
            .par_iter()
            .enumerate()
            .filter_map(|(i, (_, spec))| {
                // In-sample scoring is an independent selection experiment per
                // variant — always from flat (no carry).
                run_windowed(&ctx, spec, cfg, train_eval_from, train_from, train_to, None)
                    .ok()
                    .map(|(d, e, _)| (i, metric_from_curve(&d, &e, sort_by)))
                    .filter(|(_, m)| !m.is_nan())
            })
            .collect();
        let (best, in_sample_metric) = scored
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .ok_or_else(|| format!("window {train_from}..{train_to}: every variant failed"))?;

        // out-of-sample: the winner, warmed up through the train window. The
        // NAV starts one row early (the train window's last close) so the
        // boundary-day return is priced; that extra row is dropped when
        // stitching (its date belongs to the previous segment).
        let test_eval_from = dates[test_start.saturating_sub(warmup)];
        let nav_from = dates[test_start - 1];
        let (seg_dates, seg_equity, seg_terminal) = run_windowed(
            &ctx,
            &variants[best].1,
            cfg,
            test_eval_from,
            nav_from,
            test_to,
            carry.as_ref(),
        )?;
        // Carry this segment's terminal book into the next seam.
        carry = Some(seg_terminal);
        windows.push(WalkForwardWindow {
            train_from,
            train_to,
            test_from,
            test_to,
            chosen: variants[best].0.clone(),
            in_sample_metric,
            // vs a flat 1.0 base: includes the entry cost and the boundary day
            oos_return: seg_equity.last().unwrap() - 1.0,
        });
        for (d, e) in seg_dates.iter().zip(&seg_equity) {
            if *d < test_from {
                continue; // the warmup/boundary row belongs to the previous segment
            }
            oos_dates.push(*d);
            oos_equity.push(scale * e);
        }
        scale = *oos_equity.last().unwrap();

        start = test_end;
    }

    if windows.is_empty() {
        return Err("date range too short for one train+test window".into());
    }
    Ok(WalkForwardReport {
        total_return: yuzu_core::metrics::total_return(&oos_equity),
        cagr: yuzu_core::metrics::cagr(&oos_equity, &oos_dates),
        sharpe: yuzu_core::metrics::sharpe(&oos_equity),
        max_drawdown: yuzu_core::metrics::max_drawdown(&oos_equity),
        windows,
        dates: oos_dates,
        equity: oos_equity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn max_lookback_finds_the_largest_window_arg() {
        // `n` / `nwindow` / `d` count; the max wins, nested in objects or arrays.
        let spec = json!({"op": "sub", "a": {"op": "sma", "n": 50}, "b": {"op": "sma", "n": 200}});
        assert_eq!(max_lookback(&spec), 200);
        assert_eq!(max_lookback(&json!({"nwindow": 30})), 30);
        assert_eq!(max_lookback(&json!([{"d": 5}, {"n": 12}])), 12);
        // Non-window keys don't count.
        assert_eq!(max_lookback(&json!({"other": 999})), 0);
    }
}
