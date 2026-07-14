//! Research studies over a strategy signal (not backtests): factor rank-IC /
//! quantile diagnostics, and event studies. Thin wrappers that evaluate the
//! spec and hand off to `yuzu_core::research`.

use std::path::Path;

use yuzu_core::backtest::BacktestConfig;

use crate::ctx::{load_ctx, referenced_series};

/// Factor report (#45): evaluate `spec` to a factor panel, form `horizon`-day
/// forward returns from close, and return rank-IC / quantile diagnostics as
/// JSON. Not a strategy run — no positions, no NAV. `--neutralize-industry`
/// demeans the factor within sector first (needs an industry map in the tree).
pub fn run_factor(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    horizon: usize,
    quantiles: usize,
    neutralize_industry: bool,
) -> Result<yuzu_core::research::FactorReport, String> {
    let ctx = load_ctx(
        root,
        from,
        to,
        &BacktestConfig::default(),
        "close",
        None,
        &referenced_series(&[spec_json]),
    )?;
    let mut factor = yuzu_core::run_strategy(spec_json, &ctx).map_err(|e| e.to_string())?;
    if neutralize_industry {
        factor = factor.neutralize_industry(&ctx.industry, true);
    }
    let close = ctx.panels.get("close").ok_or("no close panel")?;
    let fwd = yuzu_core::research::forward_returns(close, horizon);
    Ok(yuzu_core::research::factor_report(&factor, &fwd, quantiles))
}

/// Event study (#45): evaluate `spec` to a 0/1 event panel, take daily returns
/// from close, and average the return path over `[-pre, +post]` around each
/// event. JSON out; not a strategy run.
pub fn run_event(
    root: &Path,
    spec_json: &str,
    from: i32,
    to: i32,
    pre: usize,
    post: usize,
) -> Result<yuzu_core::research::EventStudy, String> {
    let ctx = load_ctx(
        root,
        from,
        to,
        &BacktestConfig::default(),
        "close",
        None,
        &referenced_series(&[spec_json]),
    )?;
    let events = yuzu_core::run_strategy(spec_json, &ctx).map_err(|e| e.to_string())?;
    let close = ctx.panels.get("close").ok_or("no close panel")?;
    let rets = yuzu_core::research::daily_returns(close);
    Ok(yuzu_core::research::event_study(&events, &rets, pre, post))
}
