//! Daily-equity NAV loop: turns a position-weight matrix + price panel into an
//! equity curve and a trade list. See `docs/backtest-engine.md` for the model.

use crate::align::align;
use crate::panel::Panel;
use ndarray::Array2;
use std::collections::HashMap;

/// Direction of a trade — `long` when the entry weight was positive, `short`
/// when negative. Serialized lowercase (`"long"` / `"short"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TradeSide {
    Long,
    Short,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Trade {
    pub symbol: String,
    pub entry_date: i32,
    pub exit_date: Option<i32>,
    pub ret: f64,
    pub period: u32,
    pub mae: Option<f64>,
    pub mfe: Option<f64>,
    /// Fill price the position was opened at (the price-panel value on
    /// `entry_date`). May be `null` if that cell was missing.
    pub entry_price: f64,
    /// Fill price the position was closed at — the panel value on `exit_date`,
    /// or the last valid price less `delist_haircut` for a delisting exit.
    /// Absent for open (mark-to-market) trades.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_price: Option<f64>,
    /// `long` / `short`, from the sign of the entry weight.
    pub side: TradeSide,
}

/// How a triggered stop fills. `Touched` (the realistic default) fills at the
/// stop level when the bar's range straddled it, or at the day's **open** when
/// the bar gapped through it (a worse-than-stop fill you couldn't avoid).
/// `Close` fills at the day's close — the "end-of-day rule" execution style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopFill {
    #[default]
    Touched,
    Close,
}

/// Execution-layer stops applied by the NAV loop to whatever position book it is
/// given (not just `hold_until`). Each holding is tracked from its entry price;
/// when a day's prices cross a level the position is force-exited to cash at the
/// [`StopFill`] price and re-entry into that name is blocked until the position
/// signal drops and re-adds it. All-off by default, so an unset `StopConfig`
/// leaves the equity curve (and every golden) unchanged.
#[derive(Debug, Clone, Copy)]
pub struct StopConfig {
    /// Exit when the return from entry falls to `−stop_loss` (e.g. `0.08` = −8%).
    /// `f64::NEG_INFINITY` (the default) disables it.
    pub stop_loss: f64,
    /// Exit when the return from entry rises to `+take_profit`.
    /// `f64::INFINITY` (the default) disables it.
    pub take_profit: f64,
    /// Exit when the return drops `trail_stop` below the best return seen since
    /// entry (a trailing stop). `f64::INFINITY` (the default) disables it.
    pub trail_stop: f64,
    /// The trailing stop only arms once the return since entry first reaches
    /// `+trail_stop_activation`. `0.0` arms immediately.
    pub trail_stop_activation: f64,
    /// How a triggered stop fills (default [`StopFill::Touched`]).
    pub fill: StopFill,
}

impl Default for StopConfig {
    fn default() -> Self {
        StopConfig {
            stop_loss: f64::NEG_INFINITY,
            take_profit: f64::INFINITY,
            trail_stop: f64::INFINITY,
            trail_stop_activation: 0.0,
            fill: StopFill::Touched,
        }
    }
}

impl StopConfig {
    /// True when no stop level is set — the NAV loop skips the stop pass entirely.
    fn is_off(&self) -> bool {
        self.stop_loss == f64::NEG_INFINITY
            && self.take_profit == f64::INFINITY
            && self.trail_stop == f64::INFINITY
    }

    /// True when at least one stop level is set. Callers use this to decide
    /// whether to load the OHLC panels the `Touched` fill needs.
    pub fn is_active(&self) -> bool {
        !self.is_off()
    }

    /// Build from optional levels (`None` = that stop off) — the shape the CLI,
    /// server, and WASM request configs use, so the `±INF` sentinels live in one
    /// place.
    pub fn from_options(
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        trail_stop: Option<f64>,
        trail_stop_activation: f64,
        fill: StopFill,
    ) -> Self {
        StopConfig {
            stop_loss: stop_loss.unwrap_or(f64::NEG_INFINITY),
            take_profit: take_profit.unwrap_or(f64::INFINITY),
            trail_stop: trail_stop.unwrap_or(f64::INFINITY),
            trail_stop_activation,
            fill,
        }
    }
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
    /// Date (YYYYMMDD) a strategy went live. When set, `run_backtest` adds a
    /// `live` block to the report with equity-curve metrics computed on the
    /// segment from the first backtest date on or after this day. `None` (the
    /// default) omits the block. The NAV loop ignores it — it is a report-only
    /// concern and does not change the full-sample equity curve.
    pub live_performance_start: Option<i32>,
    /// Execution-layer stops (stop-loss / take-profit / trailing). All-off by
    /// default; see [`StopConfig`]. Requires the OHLC panels for `Touched` fills.
    pub stops: StopConfig,
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
            live_performance_start: None,
            stops: StopConfig::default(),
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

/// Fill price for a stop at `level` given the day's `open`, honoring gaps.
/// `adverse` = a stop-loss/trailing exit (price moved against you): a gap makes
/// the fill *worse* → `min(open, level)` long / `max(open, level)` short. A
/// favorable (take-profit) exit gaps in your favor → `max`/`min` swapped. NaN
/// open falls back to the level.
fn gap_fill(open: f64, level: f64, dir: f64, adverse: bool) -> f64 {
    if open.is_nan() {
        return level;
    }
    let long = dir >= 0.0;
    // adverse-long / favorable-short → take the lower; the other two → higher.
    if adverse == long {
        open.min(level)
    } else {
        open.max(level)
    }
}

/// Among two triggered adverse-side fills, the one hit **first** as price moved
/// against the position: the higher price for a long, the lower for a short.
fn first_touched(a: Option<f64>, b: f64, dir: f64) -> f64 {
    match a {
        None => b,
        Some(a) if dir >= 0.0 => a.max(b),
        Some(a) => a.min(b),
    }
}

/// Evaluate the stops for one held day. `entry` is the entry price, `dir` the
/// position sign (+1 long / −1 short); `peak` (best favorable return ratio since
/// entry) is updated in place. Returns the fill price when a stop triggers.
/// [`StopFill::Close`] triggers on the close and fills there; [`StopFill::Touched`]
/// triggers on the intraday range and fills at the level (or the gapped open).
#[allow(clippy::too_many_arguments)]
fn check_stop(
    entry: f64,
    dir: f64,
    peak: &mut f64,
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    cfg: &StopConfig,
) -> Option<f64> {
    if entry == 0.0 || entry.is_nan() {
        return None;
    }
    let signed = |p: f64| dir * (p / entry - 1.0); // return in the position's favor
    let sl_on = cfg.stop_loss != f64::NEG_INFINITY;
    let tp_on = cfg.take_profit != f64::INFINITY;
    let tr_on = cfg.trail_stop != f64::INFINITY;

    match cfg.fill {
        StopFill::Close => {
            if c.is_nan() {
                return None;
            }
            let rc = signed(c);
            *peak = peak.max(rc);
            let hit = (sl_on && rc <= -cfg.stop_loss.abs())
                || (tp_on && rc >= cfg.take_profit.abs())
                || (tr_on
                    && *peak >= cfg.trail_stop_activation.abs()
                    && rc <= *peak - cfg.trail_stop.abs());
            hit.then_some(c)
        }
        StopFill::Touched => {
            if h.is_nan() || l.is_nan() {
                return None;
            }
            let (fav_price, adv_price) = if dir >= 0.0 { (h, l) } else { (l, h) };
            let best = signed(fav_price);
            let worst = signed(adv_price);
            // The trailing stop keys off the peak established on PRIOR days, not
            // today's high — otherwise a wide up-day would self-trip (we can't
            // know from OHLC whether the high or the low came first intraday).
            let prior_peak = *peak;
            *peak = peak.max(best);
            let level = |t: f64| entry * (1.0 + dir * t); // price at signed-return t

            // Adverse-side stops (stop-loss, trailing) take priority; fill at the
            // first-touched level, gap-adjusted to the open.
            let mut adverse: Option<f64> = None;
            if sl_on && worst <= -cfg.stop_loss.abs() {
                let f = gap_fill(o, level(-cfg.stop_loss.abs()), dir, true);
                adverse = Some(first_touched(adverse, f, dir));
            }
            if tr_on
                && prior_peak >= cfg.trail_stop_activation.abs()
                && worst <= prior_peak - cfg.trail_stop.abs()
            {
                let f = gap_fill(o, level(prior_peak - cfg.trail_stop.abs()), dir, true);
                adverse = Some(first_touched(adverse, f, dir));
            }
            if adverse.is_some() {
                return adverse;
            }
            // Take-profit only when no adverse stop fired this day.
            if tp_on && best >= cfg.take_profit.abs() {
                return Some(gap_fill(o, level(cfg.take_profit.abs()), dir, false));
            }
            None
        }
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

/// Run the NAV loop from a flat starting book. See [`run_with_initial`].
pub fn run(
    positions: &Panel,
    prices: &Panel,
    high: Option<&Panel>,
    low: Option<&Panel>,
    volume: Option<&Panel>,
    cfg: &BacktestConfig,
) -> BacktestRun {
    run_with_initial(positions, prices, None, high, low, volume, cfg, None)
}

/// Like [`run`], but the book starts holding `initial_weights` (symbol → weight)
/// instead of flat. Day-0 pays turnover only on the difference between those
/// carried holdings and the day-0 target, so stitching segments that keep the
/// same names doesn't pay a full entry cost at every seam. Keyed by symbol, so
/// it survives a differing column order / universe between segments; symbols
/// absent from the map (or from this segment's panel) start flat.
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
                let (er, ep) = open.take().unwrap();
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

        // Fill prices and side come off the same panel cells that drive returns.
        assert_eq!(long.side, TradeSide::Long);
        assert!((long.entry_price - 10.0).abs() < 1e-12); // close on entry day
        assert!((long.exit_price.unwrap() - 11.0).abs() < 1e-12); // close on exit day
        assert_eq!(short.side, TradeSide::Short);
        assert!((short.entry_price - 10.0).abs() < 1e-12);
        assert!(short.exit_price.is_none()); // open trade: no realized exit
    }

    #[test]
    fn delisting_exit_price_is_haircut_last_valid() {
        use crate::panel::Panel;
        // Held from day 0; price goes missing from day 2 on -> delisted after 1
        // missing row. Exit fills at the last valid price (20) less a 10% haircut.
        let dates = vec![20240102, 20240103, 20240104, 20240105];
        let pos = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            vec![vec![10.0], vec![20.0], vec![f64::NAN], vec![f64::NAN]],
        )
        .unwrap();
        let cfg = BacktestConfig {
            delist_after: 1,
            delist_haircut: 0.1,
            ..Default::default()
        };
        let r = run(&pos, &px, None, None, None, &cfg);
        let t = &r.trades[0];
        assert_eq!(t.side, TradeSide::Long);
        assert!((t.entry_price - 10.0).abs() < 1e-12);
        assert!(t.exit_date.is_some(), "delisting force-closes the trade");
        assert!(
            (t.exit_price.unwrap() - 18.0).abs() < 1e-12,
            "exit fills at 20 * (1 - 0.1) = 18, got {:?}",
            t.exit_price
        );
    }

    #[test]
    fn terminal_weights_report_the_final_book() {
        use crate::panel::Panel;
        // Hold A every day; end drifted book is A at weight 1.0 (single name).
        let dates = vec![20240102, 20240103];
        let pos =
            Panel::from_rows(dates.clone(), vec!["A".into()], vec![vec![1.0], vec![1.0]]).unwrap();
        let px = Panel::from_rows(dates, vec!["A".into()], vec![vec![10.0], vec![11.0]]).unwrap();
        let r = run(&pos, &px, None, None, None, &BacktestConfig::default());
        assert_eq!(r.terminal_weights.len(), 1);
        assert!((r.terminal_weights["A"] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn initial_weights_pay_seam_turnover_only_on_the_difference() {
        use crate::panel::Panel;
        // One name held every day, flat prices so nothing drifts; a 1% fee makes
        // the day-0 turnover visible in equity.
        let dates = vec![20240102, 20240103, 20240104];
        let pos = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0]],
        )
        .unwrap();
        let px = Panel::from_rows(
            dates,
            vec!["A".into()],
            vec![vec![10.0], vec![10.0], vec![10.0]],
        )
        .unwrap();
        let cfg = BacktestConfig {
            fee_ratio: 0.01,
            ..Default::default()
        };

        // Flat start: day-0 buys the whole book, paying 1% on turnover 1.0.
        let flat = run(&pos, &px, None, None, None, &cfg);
        assert!((flat.equity[0] - 0.99).abs() < 1e-12);
        assert!((flat.terminal_weights["A"] - 1.0).abs() < 1e-12);

        // Carrying the identical book -> zero seam turnover, no day-0 cost.
        let carried = HashMap::from([("A".to_string(), 1.0)]);
        let warm = run_with_initial(&pos, &px, None, None, None, None, &cfg, Some(&carried));
        assert!((warm.equity[0] - 1.0).abs() < 1e-12, "no seam cost");
        assert!((warm.equity[2] - 1.0).abs() < 1e-12);

        // Carrying half the target -> turnover 0.5, so only half the entry fee.
        let half = HashMap::from([("A".to_string(), 0.5)]);
        let partial = run_with_initial(&pos, &px, None, None, None, None, &cfg, Some(&half));
        assert!((partial.equity[0] - (1.0 - 0.01 * 0.5)).abs() < 1e-12);

        // A carried symbol that isn't in this segment's target still costs to
        // unwind: hold B, carry A -> turnover |0-1| (sell A) + |1-0| (buy B) = 2.
        let posb = Panel::from_rows(
            vec![20240102, 20240103],
            vec!["A".into(), "B".into()],
            vec![vec![0.0, 1.0], vec![0.0, 1.0]],
        )
        .unwrap();
        let pxb = Panel::from_rows(
            vec![20240102, 20240103],
            vec!["A".into(), "B".into()],
            vec![vec![10.0, 10.0], vec![10.0, 10.0]],
        )
        .unwrap();
        let cross = run_with_initial(
            &posb,
            &pxb,
            None,
            None,
            None,
            None,
            &cfg,
            Some(&HashMap::from([("A".to_string(), 1.0)])),
        );
        assert!((cross.equity[0] - (1.0 - 0.01 * 2.0)).abs() < 1e-12);
    }

    // ---- execution-layer stops (#20) ---------------------------------------

    /// Build (pos, close, open, high, low) panels from row-wise OHLC for one
    /// symbol held long every day.
    fn stop_fixture(
        ohlc: &[(f64, f64, f64, f64)], // (open, high, low, close)
    ) -> (Panel, Panel, Panel, Panel, Panel) {
        use crate::panel::Panel;
        let dates: Vec<i32> = (0..ohlc.len() as i32).map(|i| 20240102 + i).collect();
        let col = |f: fn(&(f64, f64, f64, f64)) -> f64| {
            Panel::from_rows(
                dates.clone(),
                vec!["A".into()],
                ohlc.iter().map(|x| vec![f(x)]).collect(),
            )
            .unwrap()
        };
        let pos = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            ohlc.iter().map(|_| vec![1.0]).collect(),
        )
        .unwrap();
        (pos, col(|x| x.3), col(|x| x.0), col(|x| x.1), col(|x| x.2))
    }

    #[test]
    fn touched_stop_loss_fills_at_the_level_not_the_close() {
        // Entry close 100; day1 low 90 touches the 8% stop (level 92) while the
        // open (98) is above it -> fill at 92, not the close (95).
        let (pos, close, open, high, low) =
            stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 95.0)]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                stop_loss: 0.08,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        assert!(
            (r.equity[1] - 0.92).abs() < 1e-12,
            "fill at 92, got {}",
            r.equity[1]
        );
        let t = &r.trades[0];
        assert!((t.exit_price.unwrap() - 92.0).abs() < 1e-12);
    }

    #[test]
    fn gapped_stop_fills_at_the_open() {
        // Day1 gaps fully below the 92 stop (open 88) -> can't fill at 92; fills
        // at the open 88 (worse than the stop).
        let (pos, close, open, high, low) =
            stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (88.0, 89.0, 87.0, 88.0)]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                stop_loss: 0.08,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        assert!(
            (r.equity[1] - 0.88).abs() < 1e-12,
            "gap fill at open 88, got {}",
            r.equity[1]
        );
    }

    #[test]
    fn close_fill_mode_triggers_and_fills_on_the_close() {
        // Touched mode would fill at 92; Close mode instead needs the close to
        // breach and fills there. Close 91 -> −9% ≤ −8% -> fill at 91.
        let (pos, close, open, high, low) =
            stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 91.0)]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                stop_loss: 0.08,
                fill: StopFill::Close,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        assert!(
            (r.equity[1] - 0.91).abs() < 1e-12,
            "close fill at 91, got {}",
            r.equity[1]
        );
    }

    #[test]
    fn take_profit_fills_at_the_level() {
        // Day1 high 115 hits the +10% take-profit (level 110); open 108 below it
        // -> fill at 110.
        let (pos, close, open, high, low) =
            stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (108.0, 115.0, 107.0, 112.0)]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                take_profit: 0.10,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        assert!(
            (r.equity[1] - 1.10).abs() < 1e-12,
            "take-profit at 110, got {}",
            r.equity[1]
        );
    }

    #[test]
    fn stopped_name_stays_flat_until_the_signal_resets() {
        // Held days 0-1 (stops day1). Signal still on day2 → must stay flat
        // (the +200 spike must NOT be earned). Signal drops day3, re-asserts
        // day4 on a calm bar → a fresh trade opens.
        use crate::panel::Panel;
        let dates: Vec<i32> = (0..5).map(|i| 20240102 + i).collect();
        let pos = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            vec![vec![1.0], vec![1.0], vec![1.0], vec![0.0], vec![1.0]],
        )
        .unwrap();
        let mk = |v: Vec<f64>| {
            Panel::from_rows(
                dates.clone(),
                vec!["A".into()],
                v.into_iter().map(|x| vec![x]).collect(),
            )
            .unwrap()
        };
        let close = mk(vec![100.0, 95.0, 200.0, 100.0, 100.0]);
        let open = mk(vec![100.0, 98.0, 190.0, 100.0, 100.0]);
        let high = mk(vec![100.0, 99.0, 205.0, 100.0, 100.0]);
        let low = mk(vec![100.0, 90.0, 190.0, 100.0, 100.0]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                stop_loss: 0.08,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        // day1: 92/100 = 0.92; day2-4: flat (the +200 spike is NOT earned).
        assert!((r.equity[1] - 0.92).abs() < 1e-12);
        assert!(
            (r.equity[2] - 0.92).abs() < 1e-12,
            "must be flat day2, got {}",
            r.equity[2]
        );
        assert!((r.equity[4] - 0.92).abs() < 1e-12);
        // Two trades: the stopped exit (day1) and the fresh re-entry on day4.
        assert_eq!(r.trades.len(), 2, "expected a stopped exit + a re-entry");
        let reentry = r.trades.iter().find(|t| t.entry_date == 20240106).unwrap();
        assert!(reentry.exit_date.is_none(), "day4 re-entry is still open");
    }

    #[test]
    fn trailing_stop_arms_and_ratchets() {
        // Rise to +20% (arms the 10% trail once activation 5% passed), then a
        // pullback whose low crosses trail level = peak(1.20) − 0.10 = 1.10 →
        // level price 110. Day2 low 108 < 110, open 118 above → fill at 110.
        let (pos, close, open, high, low) = stop_fixture(&[
            (100.0, 100.0, 100.0, 100.0),
            (105.0, 120.0, 104.0, 118.0), // peak 1.20
            (118.0, 118.0, 108.0, 109.0), // pulls back through 110
        ]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                trail_stop: 0.10,
                trail_stop_activation: 0.05,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        // day1 close 118 -> 1.18; day2 exits at 110 -> ×(110/118).
        assert!(
            (r.equity[2] - 1.10).abs() < 1e-9,
            "trail exit at 110, got {}",
            r.equity[2]
        );
    }

    #[test]
    fn short_position_stop_loss_triggers_on_a_rise() {
        // Short entry at 100; stop_loss 8% for a short triggers when price RISES
        // to 108. Day1 high 110 touches; open 102 below -> fill at 108; a short
        // loses as price rises, so equity < 1.
        use crate::panel::Panel;
        let dates: Vec<i32> = (0..2).map(|i| 20240102 + i).collect();
        let pos = Panel::from_rows(
            dates.clone(),
            vec!["A".into()],
            vec![vec![-1.0], vec![-1.0]],
        )
        .unwrap();
        let mk = |v: Vec<f64>| {
            Panel::from_rows(
                dates.clone(),
                vec!["A".into()],
                v.into_iter().map(|x| vec![x]).collect(),
            )
            .unwrap()
        };
        let close = mk(vec![100.0, 106.0]);
        let open = mk(vec![100.0, 102.0]);
        let high = mk(vec![100.0, 110.0]);
        let low = mk(vec![100.0, 101.0]);
        let cfg = BacktestConfig {
            stops: StopConfig {
                stop_loss: 0.08,
                ..Default::default()
            },
            ..Default::default()
        };
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &cfg,
            None,
        );
        // short return day1 = w(−1)·(108/100 − 1) = −0.08 -> equity 0.92.
        assert!(
            (r.equity[1] - 0.92).abs() < 1e-12,
            "short stop at 108, got {}",
            r.equity[1]
        );
    }

    #[test]
    fn stops_off_by_default_leaves_the_curve_unchanged() {
        // A drop that would trip an 8% stop earns the full close-to-close move
        // when stops are off (default) — proving the feature is opt-in.
        let (pos, close, open, high, low) =
            stop_fixture(&[(100.0, 100.0, 100.0, 100.0), (98.0, 99.0, 90.0, 95.0)]);
        let r = run_with_initial(
            &pos,
            &close,
            Some(&open),
            Some(&high),
            Some(&low),
            None,
            &BacktestConfig::default(),
            None,
        );
        assert!(
            (r.equity[1] - 0.95).abs() < 1e-12,
            "no stop -> close 95, got {}",
            r.equity[1]
        );
    }
}
