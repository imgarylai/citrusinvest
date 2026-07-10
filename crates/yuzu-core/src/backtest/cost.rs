//! Rebalance turnover cost (fees, tax, flat slippage, square-root impact).

use super::config::BacktestConfig;

/// Turnover cost of moving `drift` to `target`. The flat component keeps its
/// original accumulation order (row-sum × ratio) so `impact_coef = 0`
/// reproduces the legacy path bit-for-bit; only the square-root impact
/// component iterates per cell over `dollar_vol` (issue #19). A cell with
/// missing or zero dollar volume contributes no impact — the flat slippage
/// already covers it — so no NaN/Inf can reach the total.
pub(crate) fn rebalance_cost(
    drift: &[f64],
    target: &[f64],
    dollar_vol: Option<&[f64]>,
    cfg: &BacktestConfig,
) -> f64 {
    let turnover: f64 = drift.iter().zip(target).map(|(d, t)| (t - d).abs()).sum();
    let sells: f64 = drift
        .iter()
        .zip(target)
        .map(|(d, t)| (d - t).max(0.0))
        .sum();
    let mut cost = (cfg.fee_ratio + cfg.slippage_ratio) * turnover + cfg.tax_ratio * sells;
    if cfg.impact_coef > 0.0 && cfg.initial_capital > 0.0 {
        if let Some(dv) = dollar_vol {
            for ((d, t), &v) in drift.iter().zip(target).zip(dv) {
                let dw = (t - d).abs();
                if dw == 0.0 || v.is_nan() || v <= 0.0 {
                    continue;
                }
                let participation = (dw * cfg.initial_capital / v).min(1.0);
                cost += dw * cfg.impact_coef * participation.sqrt();
            }
        }
    }
    cost
}
