//! Lookahead-bias detector: rerun a strategy with its position matrix lagged and
//! report the metric decay — a strategy that collapses under a small lag is
//! living on same-close execution or same-day data timestamps.

use std::path::Path;

use serde::Serialize;
use yuzu_core::backtest::BacktestConfig;

use crate::ctx::load_ctx;

/// Headline metrics of one leg of the lookahead comparison.
#[derive(Serialize)]
pub struct LookaheadLeg {
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub max_drawdown: f64,
}

/// Baseline vs signal-lagged comparison (issue #23). A strategy whose edge
/// survives executing `shift_days` later is robust to same-close execution
/// assumptions and same-day data timestamps; one that collapses is living on
/// lookahead (or on fills it could never get).
#[derive(Serialize)]
pub struct LookaheadReport {
    pub shift_days: usize,
    pub baseline: LookaheadLeg,
    pub lagged: LookaheadLeg,
    /// `baseline.sharpe - lagged.sharpe`.
    pub sharpe_drop: f64,
    /// `baseline.total_return - lagged.total_return`.
    pub return_drop: f64,
    /// True when a clearly positive baseline (`sharpe > 0.5`) loses more than
    /// half its Sharpe under the lag.
    pub suspicious: bool,
}

fn lookahead_leg(dates: &[i32], equity: &[f64]) -> LookaheadLeg {
    LookaheadLeg {
        total_return: yuzu_core::metrics::total_return(equity),
        cagr: yuzu_core::metrics::cagr(equity, dates),
        sharpe: yuzu_core::metrics::sharpe(equity),
        max_drawdown: yuzu_core::metrics::max_drawdown(equity),
    }
}

/// Run the strategy twice — as-is, and with the position matrix lagged by
/// `shift_days` (signals executed N days late) — and report the metric deltas.
pub fn run_lookahead(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    shift_days: usize,
    cfg: &BacktestConfig,
) -> Result<LookaheadReport, String> {
    if shift_days == 0 {
        return Err("shift_days must be > 0".into());
    }
    let ctx = load_ctx(root, from, to, cfg, "close")?;
    let positions = yuzu_core::run_strategy(spec_json, &ctx).map_err(|e| e.to_string())?;
    let prices = ctx.panels.get("close").ok_or("no close panel")?;
    let volume = ctx.panels.get("volume");
    let base = yuzu_core::backtest::run(&positions, prices, None, None, volume, cfg);
    let lagged_pos = positions.shift(shift_days);
    let lag = yuzu_core::backtest::run(&lagged_pos, prices, None, None, volume, cfg);

    let baseline = lookahead_leg(&base.dates, &base.equity);
    let lagged = lookahead_leg(&lag.dates, &lag.equity);
    let sharpe_drop = baseline.sharpe - lagged.sharpe;
    let return_drop = baseline.total_return - lagged.total_return;
    let suspicious = baseline.sharpe > 0.5 && lagged.sharpe < 0.5 * baseline.sharpe;
    Ok(LookaheadReport {
        shift_days,
        baseline,
        lagged,
        sharpe_drop,
        return_drop,
        suspicious,
    })
}

/// One shift level of the lookahead decay profile.
#[derive(Serialize)]
pub struct LookaheadProfilePoint {
    pub shift_days: usize,
    pub total_return: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub max_drawdown: f64,
    /// `lagged.sharpe / baseline.sharpe`; NaN when the baseline Sharpe ≤ 0.
    pub sharpe_retention: f64,
}

/// Decay profile across several shifts. The SHAPE is the diagnosis: a cliff at
/// shift 1 that then flattens = same-close execution dependence; smooth decay
/// = genuinely fast alpha; performance that holds until shift ≈ N then drops =
/// data stamped ~N days ahead of its real publication (fundamentals
/// lookahead); flat = robust to execution timing.
#[derive(Serialize)]
pub struct LookaheadProfile {
    pub baseline: LookaheadLeg,
    pub points: Vec<LookaheadProfilePoint>,
    /// Same rule as [`run_lookahead`], applied at the smallest shift.
    pub suspicious: bool,
}

/// Default shift ladder for [`run_lookahead_profile`].
pub const PROFILE_SHIFTS: &[usize] = &[1, 2, 3, 5, 10, 21];

/// Run the strategy once, then re-price it under each `shifts` lag and report
/// the metric decay curve. Signals are evaluated once — only the position
/// matrix is shifted per level, so an N-level profile costs ~N NAV loops, not
/// N strategy evaluations.
pub fn run_lookahead_profile(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    shifts: &[usize],
    cfg: &BacktestConfig,
) -> Result<LookaheadProfile, String> {
    if shifts.is_empty() {
        return Err("profile needs at least one shift".into());
    }
    if shifts.contains(&0) {
        return Err("shifts must be > 0".into());
    }
    let mut shifts = shifts.to_vec();
    shifts.sort_unstable();
    shifts.dedup();

    let ctx = load_ctx(root, from, to, cfg, "close")?;
    let positions = yuzu_core::run_strategy(spec_json, &ctx).map_err(|e| e.to_string())?;
    let prices = ctx.panels.get("close").ok_or("no close panel")?;
    let volume = ctx.panels.get("volume");
    let base = yuzu_core::backtest::run(&positions, prices, None, None, volume, cfg);
    let baseline = lookahead_leg(&base.dates, &base.equity);

    let points: Vec<LookaheadProfilePoint> = shifts
        .iter()
        .map(|&shift| {
            let lag =
                yuzu_core::backtest::run(&positions.shift(shift), prices, None, None, volume, cfg);
            let leg = lookahead_leg(&lag.dates, &lag.equity);
            let sharpe_retention = if baseline.sharpe > 0.0 {
                leg.sharpe / baseline.sharpe
            } else {
                f64::NAN
            };
            LookaheadProfilePoint {
                shift_days: shift,
                total_return: leg.total_return,
                cagr: leg.cagr,
                sharpe: leg.sharpe,
                max_drawdown: leg.max_drawdown,
                sharpe_retention,
            }
        })
        .collect();

    let suspicious = baseline.sharpe > 0.5 && points[0].sharpe < 0.5 * baseline.sharpe;
    Ok(LookaheadProfile {
        baseline,
        points,
        suspicious,
    })
}
