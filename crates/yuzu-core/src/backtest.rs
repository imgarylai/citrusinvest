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
    // when the liquidity cap is active.
    let liquidity_on = cfg.max_participation > 0.0 && cfg.initial_capital > 0.0 && volume.is_some();
    let dollar_vol: Option<Array2<f64>> = if liquidity_on {
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
            }
            // rebalance event = first row, or the ffilled raw allocation changed.
            rebalance[r] = prev_raw.as_deref() != Some(last.as_slice());
            prev_raw = Some(last.clone());
            let mut row = last.clone();
            normalize_weights_row(&mut row);
            cap_weights_row(&mut row, cfg.position_limit);
            if let Some(dv) = &dollar_vol {
                let dv_row: Vec<f64> = (0..n).map(|c| dv[[r, c]]).collect();
                cap_weights_by_liquidity(
                    &mut row,
                    &dv_row,
                    cfg.max_participation,
                    cfg.initial_capital,
                );
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
    // day-0 entry cost (flat -> first target)
    value *= 1.0 - rebalance_cost(&vec![0.0; n], &w_prev, cfg);
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
        // Weights drift between rebalances. Only on a rebalance day do we reset to
        // the target and pay turnover cost; otherwise the drifted weights carry over
        // with no cost (buy at change-points, hold
        // and drift in between).
        if rebalance[r] {
            let tgt: Vec<f64> = (0..n).map(|c| target[[r, c]]).collect();
            value *= 1.0 - rebalance_cost(&drift, &tgt, cfg);
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
        for r in 0..nrows {
            let held = target[[r, c]] != 0.0;
            let entry_now = held && open.is_none();
            let exit_now = !held && open.is_some();
            if entry_now {
                open = Some((r, px.data[[r, c]]));
            } else if exit_now {
                let (er, ep) = open.take().unwrap();
                let xp = px.data[[r, c]];
                let gross = if ep == 0.0 || ep.is_nan() || xp.is_nan() {
                    1.0
                } else {
                    xp / ep
                };
                let net = (1.0 - cfg.fee_ratio - cfg.slippage_ratio)
                    * gross
                    * (1.0 - cfg.fee_ratio - cfg.tax_ratio - cfg.slippage_ratio);
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

fn rebalance_cost(drift: &[f64], target: &[f64], cfg: &BacktestConfig) -> f64 {
    let turnover: f64 = drift.iter().zip(target).map(|(d, t)| (t - d).abs()).sum();
    let sells: f64 = drift
        .iter()
        .zip(target)
        .map(|(d, t)| (d - t).max(0.0))
        .sum();
    (cfg.fee_ratio + cfg.slippage_ratio) * turnover + cfg.tax_ratio * sells
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
