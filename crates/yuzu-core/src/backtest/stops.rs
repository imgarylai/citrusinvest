//! Execution-layer stop evaluation (touched / close fills).

use super::config::{StopConfig, StopFill};

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
pub(crate) fn check_stop(
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
