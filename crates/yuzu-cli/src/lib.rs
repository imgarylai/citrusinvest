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
    if cfg.max_participation > 0.0 && cfg.initial_capital > 0.0 {
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

fn metric_of(report: &Report, key: SortKey) -> f64 {
    match key {
        SortKey::Sharpe => report.metrics.sharpe,
        SortKey::TotalReturn => report.metrics.total_return,
        SortKey::Cagr => report.metrics.cagr,
        SortKey::Calmar => report.metrics.calmar,
    }
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
}

/// Walk-forward analysis: roll a `train_days`/`test_days` window (in trading
/// days) over the close panel's date axis; in each window run every variant on
/// the train slice, pick the best by `sort_by`, run it on the test slice, and
/// chain the out-of-sample equity segments into one curve.
///
/// Indicators start cold at each window boundary (no warmup carry-over) —
/// identical handicap for every variant, but absolute levels differ from a
/// full-range backtest.
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
    } = *params;
    if variants.is_empty() {
        return Err("no variants to select from".into());
    }
    if train_days == 0 || test_days == 0 {
        return Err("train_days and test_days must be > 0".into());
    }
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

        // in-sample: run every variant on the train slice in parallel
        let train_ctx = slice_ctx(&ctx, train_from, train_to);
        let scored: Vec<(usize, f64)> = variants
            .par_iter()
            .enumerate()
            .filter_map(|(i, (_, spec))| {
                run_backtest(spec, &train_ctx, "close", cfg)
                    .ok()
                    .map(|r| (i, metric_of(&r, sort_by)))
                    .filter(|(_, m)| !m.is_nan())
            })
            .collect();
        let (best, in_sample_metric) = scored
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .ok_or_else(|| format!("window {train_from}..{train_to}: every variant failed"))?;

        // out-of-sample: run the winner on the test slice
        let test_ctx = slice_ctx(&ctx, test_from, test_to);
        let report =
            run_backtest(&variants[best].1, &test_ctx, "close", cfg).map_err(|e| e.to_string())?;
        windows.push(WalkForwardWindow {
            train_from,
            train_to,
            test_from,
            test_to,
            chosen: variants[best].0.clone(),
            in_sample_metric,
            oos_return: report.metrics.total_return,
        });
        // chain: each segment starts at ~1.0 (including its day-0 entry cost)
        for (d, e) in report.dates.iter().zip(&report.equity) {
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
