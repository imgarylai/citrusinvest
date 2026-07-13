//! Daily-equity NAV loop.

use crate::align::align;
use crate::panel::Panel;
use ndarray::Array2;
use std::collections::HashMap;

use super::config::BacktestConfig;
use super::cost::rebalance_cost;
use super::delist::scan_delistings;
use super::stops::check_stop;
use super::trade::{Trade, TradeSide};
use super::weights::{cap_weights_by_liquidity, cap_weights_row, normalize_weights_row};

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
/// Map an entry-weight sign (`dir`) to a [`TradeSide`]. A zero/NaN direction
/// never reaches here (a trade only opens on a non-zero weight); treat the
/// non-negative case as long.
fn side_of(dir: f64) -> TradeSide {
    if dir < 0.0 {
        TradeSide::Short
    } else {
        TradeSide::Long
    }
}

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
    /// Drifted book weight per symbol on the final row (only non-zero holdings).
    /// Feed this into the next segment's [`run_with_initial`] to pay seam
    /// turnover only on the difference (walk-forward carry-over, #21).
    pub terminal_weights: HashMap<String, f64>,
}

/// Bundled panels for the NAV loop — avoids a long positional argument list.
#[derive(Clone, Copy)]
pub struct NavInputs<'a> {
    pub positions: &'a Panel,
    pub prices: &'a Panel,
    pub open: Option<&'a Panel>,
    pub high: Option<&'a Panel>,
    pub low: Option<&'a Panel>,
    pub volume: Option<&'a Panel>,
    /// Prior segment terminal weights (symbol → weight); `None` starts flat.
    pub initial_weights: Option<&'a HashMap<String, f64>>,
}

/// Run the NAV loop from a flat starting book. See [`run_with_initial`].
pub fn run(
    positions: &Panel,
    prices: &Panel,
    high: Option<&Panel>,
    low: Option<&Panel>,
    volume: Option<&Panel>,
    cfg: &BacktestConfig,
) -> BacktestRun {
    run_nav(
        NavInputs {
            positions,
            prices,
            open: None,
            high,
            low,
            volume,
            initial_weights: None,
        },
        cfg,
    )
}

/// Like [`run`], but the book starts holding `initial_weights` (symbol → weight)
/// instead of flat. Day-0 pays turnover only on the difference between those
/// carried holdings and the day-0 target, so stitching segments that keep the
/// same names doesn't pay a full entry cost at every seam. Keyed by symbol, so
/// it survives a differing column order / universe between segments; symbols
/// absent from the map (or from this segment's panel) start flat.
///
/// Prefer [`run_nav`] + [`NavInputs`] for new call sites; this multi-arg form
/// is kept for API compatibility.
#[allow(clippy::too_many_arguments)]
pub fn run_with_initial(
    positions: &Panel,
    prices: &Panel,
    open: Option<&Panel>,
    high: Option<&Panel>,
    low: Option<&Panel>,
    volume: Option<&Panel>,
    cfg: &BacktestConfig,
    initial_weights: Option<&HashMap<String, f64>>,
) -> BacktestRun {
    run_nav(
        NavInputs {
            positions,
            prices,
            open,
            high,
            low,
            volume,
            initial_weights,
        },
        cfg,
    )
}

/// NAV entry that takes a [`NavInputs`] bundle (preferred for new call sites).
pub fn run_nav(inputs: NavInputs<'_>, cfg: &BacktestConfig) -> BacktestRun {
    let NavInputs {
        positions,
        prices,
        open,
        high,
        low,
        volume,
        initial_weights,
    } = inputs;
    let (pos, px) = align(positions, prices);
    let op = conform_to(&px, open);
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

    // Execution-layer stops: track each holding from its entry price and
    // force-exit at the stop fill; `stopped[r,c]` marks the exit day (fill in
    // `stop_fill`), and a stopped name stays flat until the raw signal resets.
    let stops_on = !cfg.stops.is_off();
    let mut stopped = Array2::from_elem((nrows, n), false);
    let mut stop_fill = Array2::from_elem((nrows, n), f64::NAN);
    let mut entry_px = vec![f64::NAN; n]; // entry price of the current holding
    let mut entry_dir = vec![0.0_f64; n]; // +1 long / −1 short
    let mut peak = vec![f64::NAN; n]; // best favorable return ratio since entry
    let mut blocked = vec![false; n]; // stopped-out, awaiting a signal reset

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
                if stops_on {
                    let held = last[c] != 0.0;
                    if blocked[c] {
                        if !held {
                            blocked[c] = false; // signal reset → future re-entry allowed
                        }
                        last[c] = 0.0; // stay flat while blocked
                        continue;
                    }
                    if !held {
                        entry_px[c] = f64::NAN; // exited normally → reset tracking
                        continue;
                    }
                    let dir = last[c].signum();
                    if entry_px[c].is_nan() || entry_dir[c] != dir {
                        entry_px[c] = px.data[[r, c]]; // fresh entry (or flip) at close
                        entry_dir[c] = dir;
                        peak[c] = 0.0;
                    }
                    let o = op.as_ref().map_or(f64::NAN, |m| m[[r, c]]);
                    let hh = hi.as_ref().map_or(f64::NAN, |m| m[[r, c]]);
                    let ll = lo.as_ref().map_or(f64::NAN, |m| m[[r, c]]);
                    if let Some(f) = check_stop(
                        entry_px[c],
                        dir,
                        &mut peak[c],
                        o,
                        hh,
                        ll,
                        px.data[[r, c]],
                        &cfg.stops,
                    ) {
                        stopped[[r, c]] = true;
                        stop_fill[[r, c]] = f;
                        last[c] = 0.0; // force-exit this day
                        blocked[c] = true;
                        entry_px[c] = f64::NAN;
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
    // The book carried into day 0: the prior segment's holdings projected onto
    // this panel's symbols (flat when unset / a symbol is absent).
    let w_start: Vec<f64> = match initial_weights {
        Some(m) => px
            .symbols
            .iter()
            .map(|s| m.get(s).copied().unwrap_or(0.0))
            .collect(),
        None => vec![0.0_f64; n],
    };
    // actual (drifted) weights carried into each day; start at the day-0 target.
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
    // day-0 entry cost (carried book -> first target; flat when unset)
    value *= 1.0 - rebalance_cost(&w_start, &w_prev, dv_row(0).as_deref(), cfg);
    equity[0] = value;

    for r in 1..nrows {
        // asset simple returns for the day (missing price -> 0 return)
        let mut g = 0.0;
        let mut drift = vec![0.0_f64; n];
        for c in 0..n {
            let p0 = px.data[[r - 1, c]];
            // A stop exits at its fill price, not the close.
            let p1 = if stops_on && stopped[[r, c]] {
                stop_fill[[r, c]]
            } else {
                px.data[[r, c]]
            };
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
                // `exit_now` implies `open` is Some; take without unwrap.
                let Some((er, ep)) = open.take() else {
                    continue;
                };
                // A delisting exit fills at the last valid price less the
                // haircut and pays no exit-leg costs (nothing traded).
                let delisted = delist
                    .as_ref()
                    .map(|(_, confirm)| confirm[[r, c]])
                    .unwrap_or(false);
                // A stop exit is a real trade (pays exit costs) but fills at its
                // stop price; a delisting fills at last-valid × (1 − haircut).
                let stopped_here = stops_on && stopped[[r, c]];
                let xp = if delisted {
                    last_valid_px * (1.0 - cfg.delist_haircut)
                } else if stopped_here {
                    stop_fill[[r, c]]
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
                    entry_price: ep,
                    exit_price: Some(xp),
                    side: side_of(dir),
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
                entry_price: ep,
                exit_price: None, // open trade: no realized exit fill
                side: side_of(dir),
            });
        }
    }

    // Final drifted book, keyed by symbol (non-zero holdings only) — the
    // starting book a following segment carries in via `run_with_initial`.
    let terminal_weights: HashMap<String, f64> = px
        .symbols
        .iter()
        .zip(&w_prev)
        .filter(|(_, &w)| w != 0.0)
        .map(|(s, &w)| (s.clone(), w))
        .collect();

    BacktestRun {
        dates,
        equity,
        trades,
        exposure,
        terminal_weights,
    }
}
