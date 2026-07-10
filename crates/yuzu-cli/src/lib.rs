//! Native batch backtest runner. Logic lives here (testable); `main.rs` is a thin
//! clap front end. Reads a locally-synced mirror of R2's `prices/` tree.

pub mod fmp;

use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::report::Report;
use yuzu_core::{run_backtest, EvalContext};
use yuzu_data::{load_panel, Field, LocalSource, PRICES_DIR};

/// Symbols with a per-symbol price file under `root/prices`, sorted and
/// de-duplicated. Recognizes `.csv.gz`, `.parquet`, and `.csv`; the
/// loaders detect the actual format from content.
pub fn list_symbols(root: &Path) -> std::io::Result<Vec<String>> {
    // `.csv.gz` before `.csv` so a gzip file isn't mis-stripped to "<sym>.csv".
    const EXTS: &[&str] = &[".csv.gz", ".parquet", ".csv"];
    let mut syms = std::collections::BTreeSet::new();
    let prices = root.join(PRICES_DIR);
    if !prices.exists() {
        return Ok(Vec::new());
    }
    for entry in std::fs::read_dir(prices)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if let Some(sym) = EXTS.iter().find_map(|ext| name.strip_suffix(ext)) {
                syms.insert(sym.to_string());
            }
        }
    }
    Ok(syms.into_iter().collect())
}

/// Load the close panel for every symbol under `root` into an `EvalContext`.
/// (Scope a run to a subset by syncing only those files into the data dir.)
/// Adds the volume panel when the config's liquidity cap is active, and a
/// "benchmark" panel (that symbol's closes) when `cfg.benchmark_key` names a
/// symbol that isn't already a loaded panel.
pub(crate) fn load_ctx(
    root: &Path,
    from: i32,
    to: i32,
    cfg: &BacktestConfig,
) -> Result<EvalContext, String> {
    let syms = list_symbols(root).map_err(|e| e.to_string())?;
    let src = LocalSource::new(root);
    let mut panels = HashMap::new();
    let close = load_panel(&src, &syms, Field::AdjClose, from, to, PRICES_DIR)
        .map_err(|e| e.to_string())?;
    panels.insert("close".to_string(), close);
    if (cfg.max_participation > 0.0 || cfg.impact_coef > 0.0) && cfg.initial_capital > 0.0 {
        let volume = load_panel(&src, &syms, Field::Volume, from, to, PRICES_DIR)
            .map_err(|e| e.to_string())?;
        panels.insert("volume".to_string(), volume);
    }
    // The CLI treats benchmark_key as a SYMBOL: its closes are loaded as a
    // one-column panel under that key (e.g. --benchmark SPY).
    if let Some(sym) = &cfg.benchmark_key {
        if !panels.contains_key(sym) {
            let bench = load_panel(
                &src,
                std::slice::from_ref(sym),
                Field::AdjClose,
                from,
                to,
                PRICES_DIR,
            )
            .map_err(|e| e.to_string())?;
            panels.insert(sym.clone(), bench);
        }
    }
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
    cfg: &BacktestConfig,
    sort_by: SortKey,
) -> Vec<SweepEntry> {
    let ctx = match load_ctx(root, from, to, cfg) {
        Ok(v) => v,
        Err(e) => return variants.iter().map(|(n, _)| failed(n, e.clone())).collect(),
    };

    let mut board: Vec<SweepEntry> = variants
        .par_iter()
        .map(
            |(name, spec)| match run_backtest(spec, &ctx, "close", cfg) {
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
    cfg: &BacktestConfig,
) -> Result<Report, String> {
    let ctx = load_ctx(root, from, to, cfg)?;
    run_backtest(spec_json, &ctx, "close", cfg).map_err(|e| e.to_string())
}

// ---- parameter grid ---------------------------------------------------------

/// A grid file: a spec template plus parameter value lists. Inside `spec`, any
/// JSON string equal to `"$name"` is a placeholder for the parameter `name`.
#[derive(serde::Deserialize)]
pub struct GridSpec {
    pub spec: serde_json::Value,
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, Vec<serde_json::Value>>,
}

fn substitute(
    node: &serde_json::Value,
    binding: &std::collections::BTreeMap<&str, &serde_json::Value>,
) -> serde_json::Value {
    match node {
        serde_json::Value::String(s) => {
            if let Some(name) = s.strip_prefix('$') {
                if let Some(v) = binding.get(name) {
                    return (*v).clone();
                }
            }
            node.clone()
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute(v, binding)))
                .collect(),
        ),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| substitute(v, binding)).collect())
        }
        _ => node.clone(),
    }
}

/// Expand a [`GridSpec`] into one named variant per parameter combination
/// (cartesian product, parameter order = alphabetical by name). Names look
/// like `"n=10,thresh=0.5"`. A grid with no params yields the spec itself.
pub fn expand_grid(grid: &GridSpec) -> Vec<(String, serde_json::Value)> {
    let names: Vec<&String> = grid.params.keys().collect();
    let lists: Vec<&Vec<serde_json::Value>> = grid.params.values().collect();
    if names.is_empty() {
        return vec![("base".to_string(), grid.spec.clone())];
    }
    let mut out = Vec::new();
    let mut idx = vec![0usize; names.len()];
    loop {
        // names[k] and lists[k] come from the same BTreeMap iteration order.
        let binding: std::collections::BTreeMap<&str, &serde_json::Value> = (0..names.len())
            .map(|k| (names[k].as_str(), &lists[k][idx[k]]))
            .collect();
        let name = (0..names.len())
            .map(|k| format!("{}={}", names[k], lists[k][idx[k]]))
            .collect::<Vec<_>>()
            .join(",");
        out.push((name, substitute(&grid.spec, &binding)));
        // odometer increment
        let mut k = names.len();
        loop {
            if k == 0 {
                return out;
            }
            k -= 1;
            idx[k] += 1;
            if idx[k] < lists[k].len() {
                break;
            }
            idx[k] = 0;
        }
    }
}

// ---- walk-forward -----------------------------------------------------------

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
fn run_windowed(
    ctx: &EvalContext,
    spec: &str,
    cfg: &BacktestConfig,
    eval_from: i32,
    nav_from: i32,
    to: i32,
) -> Result<(Vec<i32>, Vec<f64>), String> {
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
    let run = yuzu_core::backtest::run(&positions, &prices, None, None, volume.as_ref(), cfg);
    Ok((run.dates, run.equity))
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
    let ctx = load_ctx(root, from, to, cfg)?;
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
                run_windowed(&ctx, spec, cfg, train_eval_from, train_from, train_to)
                    .ok()
                    .map(|(d, e)| (i, metric_from_curve(&d, &e, sort_by)))
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
        let (seg_dates, seg_equity) = run_windowed(
            &ctx,
            &variants[best].1,
            cfg,
            test_eval_from,
            nav_from,
            test_to,
        )?;
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

// ---- lookahead-bias detector --------------------------------------------------

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
    let ctx = load_ctx(root, from, to, cfg)?;
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

    let ctx = load_ctx(root, from, to, cfg)?;
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
