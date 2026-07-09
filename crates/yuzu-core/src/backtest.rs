//! Daily-equity NAV loop: turns a position-weight matrix + price panel into an
//! equity curve and a trade list. See `docs/backtest-engine.md` for the model.

use crate::align::align;
use crate::panel::Panel;
use ndarray::Array2;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Trade {
    pub symbol: String,
    pub entry_date: i32,
    pub exit_date: Option<i32>,
    pub ret: f64,
    pub period: u32,
    pub mae: Option<f64>,
    pub mfe: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct BacktestConfig {
    pub fee_ratio: f64,
    pub tax_ratio: f64,
    pub position_limit: f64,
    /// Slippage charged per unit of turnover (both sides), on top of `fee_ratio`.
    /// A crude stand-in for market impact / spread: `0.0005` = 5 bps per trade
    /// leg. `0.0` (the default) disables it.
    pub slippage_ratio: f64,
    /// Notional book size in dollars, used only by the liquidity cap below to
    /// convert weights into dollar positions. `0.0` (the default) disables the cap.
    pub initial_capital: f64,
    /// Max fraction of a symbol's daily dollar volume the book may hold:
    /// `|w| <= max_participation * price * volume / initial_capital`. Requires a
    /// volume panel and `initial_capital > 0`. `0.0` (the default) disables it.
    /// The cap is measured against `initial_capital`, not compounded equity.
    pub max_participation: f64,
    /// Square-root market-impact coefficient. On each rebalance, every traded
    /// cell pays `impact_coef * sqrt(participation)` per unit of turnover,
    /// where `participation = |Δw| * initial_capital / dollar_volume`, capped
    /// at 1. Requires `initial_capital > 0` and a volume panel; a cell with
    /// missing or zero dollar volume pays only the flat `slippage_ratio`.
    /// `0.0` (the default) disables it and reproduces the flat-cost path
    /// exactly.
    pub impact_coef: f64,
    /// After this many consecutive missing-price rows a symbol is treated as
    /// delisted: the position is force-closed at its last valid price (less
    /// `delist_haircut`) and re-entry is blocked until prices resume. `0` (the
    /// default) keeps the legacy behavior (a dead position freezes at its last
    /// value — survivorship-friendly, beware).
    pub delist_after: usize,
    /// Fraction of a force-closed position's value written off on delisting:
    /// `0.0` = exit at the last valid price, `1.0` = total loss. Shorts gain
    /// symmetrically. Only used when `delist_after > 0`.
    pub delist_haircut: f64,
    /// Name of a series in the `EvalContext` to compare against (e.g. a panel
    /// holding SPY closes). When set, `run_backtest` adds a rebased benchmark
    /// curve and benchmark-relative metrics (alpha/beta/excess/tracking
    /// error/information ratio) to the report. The NAV loop ignores it.
    pub benchmark_key: Option<String>,
    /// Number of circular-block-bootstrap resamples of the daily returns; the
    /// report gains p05/p50/p95 bands for Sharpe/CAGR/max drawdown. `0` (the
    /// default) disables it. Deterministic (fixed internal seed).
    pub bootstrap_samples: usize,
    /// Bootstrap block length in trading days; `0` (the default) auto-selects
    /// `⌊√n⌋`. Only used when `bootstrap_samples > 0`.
    pub bootstrap_block: usize,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        BacktestConfig {
            fee_ratio: 0.0,
            tax_ratio: 0.0,
            position_limit: 0.0,
            slippage_ratio: 0.0,
            initial_capital: 0.0,
            max_participation: 0.0,
            impact_coef: 0.0,
            delist_after: 0,
            delist_haircut: 0.0,
            benchmark_key: None,
            bootstrap_samples: 0,
            bootstrap_block: 0,
        }
    }
}

pub(crate) fn normalize_weights_row(row: &mut [f64]) {
    let total = row.iter().map(|w| w.abs()).sum::<f64>().max(1.0);
    for w in row.iter_mut() {
        *w /= total;
    }
}

/// Clamp each position's weight to `±limit` (sign-preserving), leaving the
/// residual in cash (a per-position weight cap). `limit <= 0` disables.
pub(crate) fn cap_weights_row(row: &mut [f64], limit: f64) {
    if limit <= 0.0 {
        return;
    }
    for w in row.iter_mut() {
        *w = w.clamp(-limit, limit);
    }
}

/// Cap each weight by the symbol's share of tradable dollar volume:
/// `|w[c]| <= max_participation * dollar_vol[c] / initial_capital` (sign-
/// preserving; residual stays in cash). A NaN dollar volume (missing volume or
/// price data) leaves the weight unchanged — data gaps aren't liquidity.
pub(crate) fn cap_weights_by_liquidity(
    row: &mut [f64],
    dollar_vol: &[f64],
    max_participation: f64,
    initial_capital: f64,
) {
    if max_participation <= 0.0 || initial_capital <= 0.0 {
        return;
    }
    for (w, dv) in row.iter_mut().zip(dollar_vol) {
        if dv.is_nan() {
            continue;
        }
        let cap = max_participation * dv / initial_capital;
        *w = w.clamp(-cap, cap);
    }
}

/// Delisting scan over the price panel. Returns `(dead, confirm)`, both
/// dates × symbols booleans: `confirm` is true on the row where a symbol's
/// NaN-price run first reaches `delist_after` (the forced-exit day); `dead` is
/// true from that row until prices resume. `None` when `delist_after == 0`.
fn scan_delistings(px: &Panel, delist_after: usize) -> Option<(Array2<bool>, Array2<bool>)> {
    if delist_after == 0 {
        return None;
    }
    let (nrows, n) = (px.nrows(), px.ncols());
    let mut dead = Array2::from_elem((nrows, n), false);
    let mut confirm = Array2::from_elem((nrows, n), false);
    for c in 0..n {
        let mut nan_run = 0usize;
        for r in 0..nrows {
            if px.data[[r, c]].is_nan() {
                nan_run += 1;
                if nan_run == delist_after {
                    confirm[[r, c]] = true;
                }
                if nan_run >= delist_after {
                    dead[[r, c]] = true;
                }
            } else {
                nan_run = 0;
            }
        }
    }
    Some((dead, confirm))
}

/// Project `other` onto `grid`'s (dates × symbols), NaN where a cell is absent.
/// None when `other` is None — so MAE/MFE degrade to None downstream. Built by
/// (date, symbol) lookup, independent of `align`, so indices line up with `grid`.
fn conform_to(grid: &Panel, other: Option<&Panel>) -> Option<Array2<f64>> {
    let other = other?;
    let row_of: HashMap<i32, usize> = other
        .dates
        .iter()
        .copied()
        .enumerate()
        .map(|(i, d)| (d, i))
        .collect();
    let col_of: HashMap<&str, usize> = other
        .symbols
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    let mut out = Array2::from_elem(grid.data.raw_dim(), f64::NAN);
    for (r, day) in grid.dates.iter().enumerate() {
        let Some(&or) = row_of.get(day) else { continue };
        for (c, sym) in grid.symbols.iter().enumerate() {
            if let Some(&oc) = col_of.get(sym.as_str()) {
                out[[r, c]] = other.data[[or, oc]];
            }
        }
    }
    Some(out)
}

/// Direction-aware MAE/MFE over rows er..=exit for column c vs entry price ep.
/// MFE = best unrealized return; MAE = worst. (None, None) when high/low absent
/// or ep invalid. NaN high/low days are skipped.
fn excursion(
    hi: &Option<Array2<f64>>,
    lo: &Option<Array2<f64>>,
    er: usize,
    exit: usize,
    c: usize,
    ep: f64,
    dir: f64,
) -> (Option<f64>, Option<f64>) {
    let (Some(hi), Some(lo)) = (hi, lo) else {
        return (None, None);
    };
    if ep == 0.0 || ep.is_nan() {
        return (None, None);
    }
    let (mut mae, mut mfe): (Option<f64>, Option<f64>) = (None, None);
    for r in er..=exit {
        let (h, l) = (hi[[r, c]], lo[[r, c]]);
        if h.is_nan() || l.is_nan() {
            continue;
        }
        let favorable = if dir >= 0.0 { h } else { l };
        let adverse = if dir >= 0.0 { l } else { h };
        let fav = dir * (favorable / ep - 1.0);
        let adv = dir * (adverse / ep - 1.0);
        mfe = Some(mfe.map_or(fav, |m| m.max(fav)));
        mae = Some(mae.map_or(adv, |m| m.min(adv)));
    }
    (mae, mfe)
}

pub struct BacktestRun {
    pub dates: Vec<i32>,
    pub equity: Vec<f64>,
    pub trades: Vec<Trade>,
    pub exposure: Vec<f64>,
}

pub fn run(
    positions: &Panel,
    prices: &Panel,
    high: Option<&Panel>,
    low: Option<&Panel>,
    volume: Option<&Panel>,
    cfg: &BacktestConfig,
) -> BacktestRun {
    let (pos, px) = align(positions, prices);
    let hi = conform_to(&px, high);
    let lo = conform_to(&px, low);
    // dollar volume per cell (NaN where volume is absent); only materialized
    // when the liquidity cap or the impact model is active.
    let liquidity_on = cfg.max_participation > 0.0 && cfg.initial_capital > 0.0 && volume.is_some();
    let impact_on = cfg.impact_coef > 0.0 && cfg.initial_capital > 0.0 && volume.is_some();
    let dollar_vol: Option<Array2<f64>> = if liquidity_on || impact_on {
        conform_to(&px, volume).map(|mut v| {
            v.zip_mut_with(&px.data, |dv, p| *dv *= p);
            v
        })
    } else {
        None
    };
    let n = px.ncols();
    let nrows = px.nrows();
    let dates = px.dates.clone();
    let delist = scan_delistings(&px, cfg.delist_after);

    // Forward-fill positions down rows; record rebalance days (the ffilled raw
    // allocation changed) and the row-normalized target weights. Weights are only
    // reset to target on a rebalance day — they drift in between (see NAV loop).
    let mut target = Array2::zeros(px.data.raw_dim());
    let mut rebalance = vec![false; nrows];
    let mut exposure = vec![0.0_f64; nrows];
    {
        let mut last = vec![0.0_f64; n];
        let mut prev_raw: Option<Vec<f64>> = None;
        for r in 0..nrows {
            for c in 0..n {
                let v = pos.data[[r, c]];
                if !v.is_nan() {
                    last[c] = v;
                }
                // A confirmed-dead symbol can't be held or entered; zeroing the
                // raw allocation also makes the confirmation row a rebalance
                // event, so the NAV loop applies the forced exit. Re-entry after
                // a relisting needs the position panel to re-assert a value.
                if let Some((dead, _)) = &delist {
                    if dead[[r, c]] {
                        last[c] = 0.0;
                    }
                }
            }
            // rebalance event = first row, or the ffilled raw allocation changed.
            rebalance[r] = prev_raw.as_deref() != Some(last.as_slice());
            prev_raw = Some(last.clone());
            let mut row = last.clone();
            normalize_weights_row(&mut row);
            cap_weights_row(&mut row, cfg.position_limit);
            if liquidity_on {
                if let Some(dv) = &dollar_vol {
                    let dv_row: Vec<f64> = (0..n).map(|c| dv[[r, c]]).collect();
                    cap_weights_by_liquidity(
                        &mut row,
                        &dv_row,
                        cfg.max_participation,
                        cfg.initial_capital,
                    );
                }
            }
            exposure[r] = row.iter().map(|w| w.abs()).sum();
            for c in 0..n {
                target[[r, c]] = row[c];
            }
        }
    }

    let mut equity = vec![1.0_f64; nrows];
    let mut value = 1.0_f64;
    // actual (drifted) weights carried into each day; start flat then set to day-0 target.
    let mut w_prev = vec![0.0_f64; n];
    for c in 0..n {
        w_prev[c] = target[[0, c]];
    }
    // per-row dollar-volume slice for the impact model (None when off)
    let dv_row = |r: usize| -> Option<Vec<f64>> {
        if !impact_on {
            return None;
        }
        dollar_vol
            .as_ref()
            .map(|dv| (0..n).map(|c| dv[[r, c]]).collect())
    };
    // day-0 entry cost (flat -> first target)
    value *= 1.0 - rebalance_cost(&vec![0.0; n], &w_prev, dv_row(0).as_deref(), cfg);
    equity[0] = value;

    for r in 1..nrows {
        // asset simple returns for the day (missing price -> 0 return)
        let mut g = 0.0;
        let mut drift = vec![0.0_f64; n];
        for c in 0..n {
            let p0 = px.data[[r - 1, c]];
            let p1 = px.data[[r, c]];
            let ret = if p0.is_nan() || p1.is_nan() || p0 == 0.0 {
                0.0
            } else {
                p1 / p0 - 1.0
            };
            g += w_prev[c] * ret;
            drift[c] = w_prev[c] * (1.0 + ret);
        }
        value *= 1.0 + g;
        // renormalize drifted weights by the realized gross factor (keeps cash implicit)
        let factor = 1.0 + g;
        if factor != 0.0 {
            for c in 0..n {
                drift[c] /= factor;
            }
        }
        // Delisting confirmation: write the position down by the haircut and
        // move the remainder to cash BEFORE the rebalance is costed — a forced
        // exit is not a trade, so it pays no fee/tax/slippage (the target row
        // is already zero for the dead symbol, so it adds no turnover either).
        if let Some((_, confirm)) = &delist {
            let mut loss = 0.0;
            for c in 0..n {
                if confirm[[r, c]] && drift[c] != 0.0 {
                    loss += drift[c] * cfg.delist_haircut;
                    drift[c] = 0.0;
                }
            }
            if loss != 0.0 {
                value *= 1.0 - loss;
                // surviving weights are unchanged in dollars but equity shrank
                let f = 1.0 - loss;
                if f != 0.0 {
                    for w in drift.iter_mut() {
                        *w /= f;
                    }
                }
            }
        }
        // Weights drift between rebalances. Only on a rebalance day do we reset to
        // the target and pay turnover cost; otherwise the drifted weights carry over
        // with no cost (buy at change-points, hold
        // and drift in between).
        if rebalance[r] {
            let tgt: Vec<f64> = (0..n).map(|c| target[[r, c]]).collect();
            value *= 1.0 - rebalance_cost(&drift, &tgt, dv_row(r).as_deref(), cfg);
            w_prev = tgt;
        } else {
            w_prev = drift;
        }
        equity[r] = value;
    }

    // Trade extraction from target-weight transitions per symbol.
    let mut trades: Vec<Trade> = Vec::new();
    for c in 0..n {
        let mut open: Option<(usize, f64)> = None; // (entry_row, entry_price)
        let mut last_valid_px = f64::NAN;
        for r in 0..nrows {
            if !px.data[[r, c]].is_nan() {
                last_valid_px = px.data[[r, c]];
            }
            let held = target[[r, c]] != 0.0;
            let entry_now = held && open.is_none();
            let exit_now = !held && open.is_some();
            if entry_now {
                open = Some((r, px.data[[r, c]]));
            } else if exit_now {
                let (er, ep) = open.take().unwrap();
                // A delisting exit fills at the last valid price less the
                // haircut and pays no exit-leg costs (nothing traded).
                let delisted = delist
                    .as_ref()
                    .map(|(_, confirm)| confirm[[r, c]])
                    .unwrap_or(false);
                let xp = if delisted {
                    last_valid_px * (1.0 - cfg.delist_haircut)
                } else {
                    px.data[[r, c]]
                };
                let gross = if ep == 0.0 || ep.is_nan() || xp.is_nan() {
                    1.0
                } else {
                    xp / ep
                };
                let exit_leg = if delisted {
                    1.0
                } else {
                    1.0 - cfg.fee_ratio - cfg.tax_ratio - cfg.slippage_ratio
                };
                let net = (1.0 - cfg.fee_ratio - cfg.slippage_ratio) * gross * exit_leg;
                let dir = target[[er, c]].signum();
                let (mae, mfe) = excursion(&hi, &lo, er, r, c, ep, dir);
                trades.push(Trade {
                    symbol: px.symbols[c].clone(),
                    entry_date: px.dates[er],
                    exit_date: Some(px.dates[r]),
                    ret: net - 1.0,
                    period: (r - er) as u32,
                    mae,
                    mfe,
                });
            }
        }
        if let Some((er, ep)) = open {
            let xp = px.data[[nrows - 1, c]];
            let gross = if ep == 0.0 || ep.is_nan() || xp.is_nan() {
                1.0
            } else {
                xp / ep
            };
            let dir = target[[er, c]].signum();
            let (mae, mfe) = excursion(&hi, &lo, er, nrows - 1, c, ep, dir);
            trades.push(Trade {
                symbol: px.symbols[c].clone(),
                entry_date: px.dates[er],
                exit_date: None,
                ret: gross - 1.0, // open trade: mark-to-market, no exit fee
                period: (nrows - 1 - er) as u32,
                mae,
                mfe,
            });
        }
    }

    BacktestRun {
        dates,
        equity,
        trades,
        exposure,
    }
}

/// Turnover cost of moving `drift` to `target`. The flat component keeps its
/// original accumulation order (row-sum × ratio) so `impact_coef = 0`
/// reproduces the legacy path bit-for-bit; only the square-root impact
/// component iterates per cell over `dollar_vol` (issue #19). A cell with
/// missing or zero dollar volume contributes no impact — the flat slippage
/// already covers it — so no NaN/Inf can reach the total.
fn rebalance_cost(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_weights_clamps_each_to_limit_leaving_cash() {
        let mut a = [0.5, 0.5];
        cap_weights_row(&mut a, 0.3);
        assert_eq!(a, [0.3, 0.3]); // each capped; sum 0.6, rest cash
        let mut b = [0.2, 0.2];
        cap_weights_row(&mut b, 0.3);
        assert_eq!(b, [0.2, 0.2]); // under cap, unchanged
        let mut c = [0.5];
        cap_weights_row(&mut c, 0.0);
        assert_eq!(c, [0.5]); // 0 = off
    }

    #[test]
    fn normalize_caps_at_one_but_leaves_small_books() {
        let mut a = [0.5, 0.5, 0.5]; // sum 1.5 -> divide by 1.5
        normalize_weights_row(&mut a);
        assert!((a[0] - 1.0 / 3.0).abs() < 1e-12);
        let mut b = [0.2, 0.3]; // sum 0.5 -> total clamped to 1.0 -> unchanged
        normalize_weights_row(&mut b);
        assert_eq!(b, [0.2, 0.3]);
    }

    #[test]
    fn single_asset_full_weight_tracks_price() {
        use crate::panel::Panel;
        // 1 asset, weight 1.0 every day, no fees -> equity tracks price ratio.
        let pos = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        )
        .unwrap();
        let run = run(&pos, &px, None, None, None, &BacktestConfig::default());
        assert_eq!(run.equity.len(), 3);
        assert!((run.equity[0] - 1.0).abs() < 1e-12);
        assert!((run.equity[1] - 1.1).abs() < 1e-12); // +10%
        assert!((run.equity[2] - 1.2).abs() < 1e-12); // 11->12 = +9.09% on 1.1
    }

    #[test]
    fn slippage_charges_turnover_like_a_fee() {
        use crate::panel::Panel;
        // Enter day 0, exit day 2: two turnover events of 1.0 each.
        let pos = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![0.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![10.0], vec![10.0]],
        )
        .unwrap();
        let slip = BacktestConfig {
            slippage_ratio: 0.001,
            ..Default::default()
        };
        let r = run(&pos, &px, None, None, None, &slip);
        // Flat price: equity = (1 - 0.001) entering * (1 - 0.001) exiting.
        let want = (1.0 - 0.001) * (1.0 - 0.001);
        assert!(
            (r.equity[2] - want).abs() < 1e-12,
            "equity {} want {want}",
            r.equity[2]
        );
        // The closed trade's net return carries slippage on both legs.
        let t = &r.trades[0];
        let want_ret = (1.0 - 0.001) * 1.0 * (1.0 - 0.001) - 1.0;
        assert!((t.ret - want_ret).abs() < 1e-12, "trade ret {}", t.ret);
        // Identical run with slippage folded into fee_ratio matches exactly.
        let fee = BacktestConfig {
            fee_ratio: 0.001,
            ..Default::default()
        };
        let r2 = run(&pos, &px, None, None, None, &fee);
        assert_eq!(r.equity, r2.equity);
    }

    #[test]
    fn impact_cost_criteria() {
        use crate::panel::Panel;
        let dates = vec![20240102, 20240103];
        let syms = vec!["LIQ".to_string(), "ILQ".to_string()];
        let pos = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1.0, 1.0]; 2]).unwrap();
        let px = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![10.0, 10.0]; 2]).unwrap();
        // dollar volume: LIQ = 10 * 1e9 = 1e10; ILQ = 10 * 100 = 1_000.
        let vol = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e9, 100.0]; 2]).unwrap();
        let cfg = |coef: f64| BacktestConfig {
            impact_coef: coef,
            initial_capital: 1_000_000.0,
            ..Default::default()
        };

        // Day-0 entry: each cell trades |Δw| = 0.5.
        // LIQ participation = 0.5 * 1e6 / 1e10 = 5e-5 (dimensionless — #1).
        // ILQ participation = 0.5 * 1e6 / 1e3 = 500 → capped at 1 (#4).
        let coef = 0.01;
        let r = run(&pos, &px, None, None, Some(&vol), &cfg(coef));
        let liq_impact = 0.5 * coef * (5e-5_f64).sqrt();
        let ilq_impact = 0.5 * coef * 1.0_f64; // capped participation
        let want = 1.0 - (liq_impact + ilq_impact);
        assert!(
            (r.equity[0] - want).abs() < 1e-15,
            "equity {} want {want}",
            r.equity[0]
        );
        // #2 monotonicity: the illiquid cell pays strictly more.
        assert!(ilq_impact > liq_impact);

        // #5/#6 zero coefficient reproduces the legacy path bit-for-bit.
        let off = run(&pos, &px, None, None, Some(&vol), &cfg(0.0));
        let legacy = run(
            &pos,
            &px,
            None,
            None,
            Some(&vol),
            &BacktestConfig::default(),
        );
        assert_eq!(off.equity, legacy.equity);

        // #8 linearity: with zero flat components, total cost is linear in coef.
        let r2 = run(&pos, &px, None, None, Some(&vol), &cfg(2.0 * coef));
        assert!(((1.0 - r2.equity[0]) - 2.0 * (1.0 - r.equity[0])).abs() < 1e-15);

        // #3 zero/NaN dollar volume: those cells contribute NO impact (flat
        // path only) and nothing non-finite reaches the total.
        for bad in [0.0, f64::NAN] {
            let vol_bad =
                Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e9, bad]; 2]).unwrap();
            let rb = run(&pos, &px, None, None, Some(&vol_bad), &cfg(coef));
            let want_liq_only = 1.0 - liq_impact;
            assert!(
                (rb.equity[0] - want_liq_only).abs() < 1e-15,
                "bad dv {bad}: equity {}",
                rb.equity[0]
            );
            assert!(rb.equity.iter().all(|e| e.is_finite()));
        }

        // No volume panel at all -> impact silently off.
        let rn = run(&pos, &px, None, None, None, &cfg(coef));
        assert_eq!(rn.equity, legacy.equity);
    }

    #[test]
    fn impact_cost_is_sign_symmetric() {
        use crate::panel::Panel;
        // #7: a buy of |Δw| = 1 and a later sell of |Δw| = 1 on a flat price
        // with identical dollar volume cost the same.
        let dates = vec![20240102, 20240103, 20240104];
        let syms = vec!["A".to_string()];
        let pos = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![vec![1.0], vec![1.0], vec![0.0]],
        )
        .unwrap();
        let px = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![10.0]; 3]).unwrap();
        let vol = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1e6]; 3]).unwrap();
        let cfg = BacktestConfig {
            impact_coef: 0.01,
            initial_capital: 1_000_000.0,
            ..Default::default()
        };
        let r = run(&pos, &px, None, None, Some(&vol), &cfg);
        let entry_cost = 1.0 - r.equity[0];
        let exit_cost = 1.0 - r.equity[2] / r.equity[1];
        assert!(entry_cost > 0.0);
        assert!(
            (entry_cost - exit_cost).abs() < 1e-15,
            "entry {entry_cost} vs exit {exit_cost}"
        );
    }

    #[test]
    fn liquidity_cap_limits_weight_to_volume_participation() {
        use crate::panel::Panel;
        let dates = vec![20240102, 20240103, 20240104];
        let syms = vec!["A".to_string()];
        let pos = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![vec![10.0], vec![10.0], vec![10.0]],
        )
        .unwrap();
        // Day-0 dollar volume = 10 * 1000 = 10_000. With capital 1_000_000 and
        // 5% participation, the cap is 10_000 * 0.05 / 1_000_000 = 0.0005.
        let vol = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![vec![1000.0], vec![1000.0], vec![1000.0]],
        )
        .unwrap();
        let cfg = BacktestConfig {
            initial_capital: 1_000_000.0,
            max_participation: 0.05,
            ..Default::default()
        };
        let r = run(&pos, &px, None, None, Some(&vol), &cfg);
        assert!((r.exposure[0] - 0.0005).abs() < 1e-12, "capped weight");

        // Cap off (defaults) or volume missing -> full weight.
        let r2 = run(
            &pos,
            &px,
            None,
            None,
            Some(&vol),
            &BacktestConfig::default(),
        );
        assert!((r2.exposure[0] - 1.0).abs() < 1e-12);
        let r3 = run(&pos, &px, None, None, None, &cfg);
        assert!((r3.exposure[0] - 1.0).abs() < 1e-12);

        // NaN volume day: weight passes through uncapped.
        let vol_nan = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![vec![f64::NAN], vec![1000.0], vec![1000.0]],
        )
        .unwrap();
        let r4 = run(&pos, &px, None, None, Some(&vol_nan), &cfg);
        assert!((r4.exposure[0] - 1.0).abs() < 1e-12, "NaN dv -> no cap");
    }

    #[test]
    fn delisting_forces_exit_with_haircut() {
        use crate::panel::Panel;
        let dates = vec![20240102, 20240103, 20240104, 20240105, 20240108];
        let syms = vec!["A".to_string(), "B".to_string()];
        // Both held from day 0. B's prices vanish from day 2 on (delisted).
        let pos = Panel::from_rows(dates.clone(), syms.clone(), vec![vec![1.0, 1.0]; 5]).unwrap();
        let px = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![
                vec![10.0, 10.0],
                vec![10.0, 10.0],
                vec![10.0, f64::NAN],
                vec![10.0, f64::NAN],
                vec![10.0, f64::NAN],
            ],
        )
        .unwrap();

        // Legacy (delist_after = 0): B freezes at its last value, equity flat.
        let r0 = run(&pos, &px, None, None, None, &BacktestConfig::default());
        assert!((r0.equity[4] - 1.0).abs() < 1e-12, "legacy freezes");
        assert!(r0
            .trades
            .iter()
            .all(|t| t.symbol != "B" || t.exit_date.is_none()));

        // delist_after = 2 confirms on day 3 (rows 2,3 NaN). Full haircut:
        // B was half the book -> equity halves; B's trade is a -100% loss.
        let cfg = BacktestConfig {
            delist_after: 2,
            delist_haircut: 1.0,
            ..Default::default()
        };
        let r = run(&pos, &px, None, None, None, &cfg);
        assert!((r.equity[2] - 1.0).abs() < 1e-12, "before confirmation");
        assert!((r.equity[3] - 0.5).abs() < 1e-12, "haircut hits equity");
        assert!((r.equity[4] - 0.5).abs() < 1e-12);
        let b = r
            .trades
            .iter()
            .find(|t| t.symbol == "B" && t.exit_date.is_some())
            .unwrap();
        assert_eq!(b.exit_date, Some(20240105));
        assert!((b.ret - (-1.0)).abs() < 1e-12, "total loss, ret {}", b.ret);
        // Surviving symbol A is now the whole book.
        assert!((r.exposure[3] - 1.0).abs() < 1e-12);

        // Haircut 0: forced exit at the last valid price -> no equity impact,
        // B's trade closes flat (entered and exited at 10).
        let cfg0 = BacktestConfig {
            delist_after: 2,
            delist_haircut: 0.0,
            ..Default::default()
        };
        let r2 = run(&pos, &px, None, None, None, &cfg0);
        assert!((r2.equity[4] - 1.0).abs() < 1e-12);
        let b2 = r2
            .trades
            .iter()
            .find(|t| t.symbol == "B" && t.exit_date.is_some())
            .unwrap();
        assert!(b2.ret.abs() < 1e-12, "flat exit, ret {}", b2.ret);
    }

    #[test]
    fn run_reports_per_day_gross_exposure() {
        use crate::panel::Panel;
        // 1 asset held every day at weight 1.0 -> exposure 1.0 each row.
        let pos = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        )
        .unwrap();
        let run = run(&pos, &px, None, None, None, &BacktestConfig::default());
        assert_eq!(run.exposure.len(), 3);
        for e in &run.exposure {
            assert!((e - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn computes_direction_aware_mae_mfe() {
        use crate::panel::Panel;
        let dates = vec![20240102, 20240103, 20240104, 20240105];
        let syms = vec!["LONG".to_string(), "SHORT".to_string()];
        // LONG: held days 0-2, exits day 3 (closed). SHORT: held all days (open).
        let pos = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![
                vec![1.0, -1.0],
                vec![1.0, -1.0],
                vec![1.0, -1.0],
                vec![0.0, -1.0],
            ],
        )
        .unwrap();
        let close = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![
                vec![10.0, 10.0],
                vec![11.0, 9.0],
                vec![12.0, 8.0],
                vec![11.0, 9.0],
            ],
        )
        .unwrap();
        let high = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![
                vec![10.0, 10.0],
                vec![13.0, 11.0],
                vec![12.0, 12.0],
                vec![11.0, 9.0],
            ],
        )
        .unwrap();
        let low = Panel::from_rows(
            dates.clone(),
            syms.clone(),
            vec![
                vec![9.0, 10.0],
                vec![11.0, 8.0],
                vec![12.0, 7.0],
                vec![10.0, 9.0],
            ],
        )
        .unwrap();

        let r = run(
            &pos,
            &close,
            Some(&high),
            Some(&low),
            None,
            &BacktestConfig::default(),
        );
        let long = r.trades.iter().find(|t| t.symbol == "LONG").unwrap();
        let short = r.trades.iter().find(|t| t.symbol == "SHORT").unwrap();

        // LONG ep=10, dir=+1, window days 0..=3: MFE from high 13 → 0.3; MAE from low 9 → -0.1
        assert!((long.mfe.unwrap() - 0.3).abs() < 1e-9, "long mfe");
        assert!((long.mae.unwrap() - (-0.1)).abs() < 1e-9, "long mae");
        // SHORT ep=10, dir=-1, open, window days 0..=3: MFE from low 7 → 0.3; MAE from high 12 → -0.2
        assert!((short.mfe.unwrap() - 0.3).abs() < 1e-9, "short mfe");
        assert!((short.mae.unwrap() - (-0.2)).abs() < 1e-9, "short mae");

        // No high/low → None.
        let r2 = run(&pos, &close, None, None, None, &BacktestConfig::default());
        assert!(r2.trades.iter().all(|t| t.mae.is_none() && t.mfe.is_none()));
    }
}
