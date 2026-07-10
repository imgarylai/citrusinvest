//! Factor diagnostics and event studies — **research** workflows that take
//! panels and return JSON, deliberately kept **out of the lemon AST** (see
//! issue #45). These are library entry points, not strategy ops: they don't
//! produce positions or change backtest NAV semantics.
//!
//! - [`factor_report`] — rank IC (+ ICIR), quantile-portfolio returns, and the
//!   long-short spread of a factor against a forward-return panel.
//! - [`event_study`] — average (and cumulative) return around a 0/1 event panel
//!   over a `(pre, post)` window.
//!
//! Both are pure functions of their input panels; the caller decides where the
//! panels come from (e.g. a lemon spec evaluated to a factor, forward returns
//! from a price panel via [`forward_returns`]). Industry-neutralizing a factor
//! first is just `factor.neutralize_industry(&industry, true)` before the call.

use crate::align::align;
use crate::ops::stat::{argsort_stable as argsort, average_ranks, mean_std as mean_std_ddof};
use crate::panel::Panel;
use serde::Serialize;

/// Forward simple return over `horizon` trading rows: `price[t+h]/price[t] − 1`
/// per cell, NaN where either endpoint is missing or `price[t] == 0`. The last
/// `horizon` rows are NaN (no future price). A natural `forward_return` input to
/// [`factor_report`].
pub fn forward_returns(prices: &Panel, horizon: usize) -> Panel {
    let (nrows, ncols) = prices.data.dim();
    let mut data = ndarray::Array2::from_elem((nrows, ncols), f64::NAN);
    if horizon > 0 && nrows > horizon {
        for r in 0..nrows - horizon {
            for c in 0..ncols {
                let p0 = prices.data[[r, c]];
                let p1 = prices.data[[r + horizon, c]];
                if p0.is_finite() && p1.is_finite() && p0 != 0.0 {
                    data[[r, c]] = p1 / p0 - 1.0;
                }
            }
        }
    }
    Panel {
        dates: prices.dates.clone(),
        symbols: prices.symbols.clone(),
        data,
    }
}

/// Backward daily simple return `price[t]/price[t-1] − 1` per cell (the return
/// realized *on* row `t`); NaN on row 0 and where either endpoint is missing.
/// The natural `returns` input to [`event_study`].
pub fn daily_returns(prices: &Panel) -> Panel {
    let (nrows, ncols) = prices.data.dim();
    let mut data = ndarray::Array2::from_elem((nrows, ncols), f64::NAN);
    for r in 1..nrows {
        for c in 0..ncols {
            let p0 = prices.data[[r - 1, c]];
            let p1 = prices.data[[r, c]];
            if p0.is_finite() && p1.is_finite() && p0 != 0.0 {
                data[[r, c]] = p1 / p0 - 1.0;
            }
        }
    }
    Panel {
        dates: prices.dates.clone(),
        symbols: prices.symbols.clone(),
        data,
    }
}

/// Factor diagnostics against a forward-return panel.
#[derive(Debug, Serialize)]
pub struct FactorReport {
    /// Dates that had ≥2 jointly-valid cells (a defined cross-sectional IC).
    pub dates: Vec<i32>,
    /// Per-date Spearman rank IC between the factor and forward returns.
    pub ic: Vec<f64>,
    /// Mean of `ic`.
    pub mean_ic: f64,
    /// Sample std (ddof=1) of `ic`; NaN with <2 periods.
    pub ic_std: f64,
    /// Information ratio of the IC series: `mean_ic / ic_std` (NaN if `ic_std`
    /// is 0/NaN). Not annualized — multiply by √periods_per_year if desired.
    pub icir: f64,
    /// Number of quantile buckets (low factor → high).
    pub quantiles: usize,
    /// Equal-weighted mean forward return per quantile, averaged over periods
    /// (index 0 = lowest factor bucket).
    pub quantile_returns: Vec<f64>,
    /// `quantile_returns.last() − quantile_returns.first()` (top minus bottom).
    pub long_short: f64,
    /// Average fraction of the top bucket's names that leave it the next period
    /// (membership turnover); NaN with fewer than two periods.
    pub top_quantile_turnover: f64,
}

/// Compute a [`FactorReport`]: align `factor` and `forward_return`, then per
/// date rank-correlate them and bucket symbols into `quantiles` groups by
/// factor, accumulating each bucket's mean forward return. A date contributes
/// to the IC series only when ≥2 cells are jointly finite; quantile stats use
/// the same jointly-finite set. `quantiles` is clamped to ≥1.
pub fn factor_report(factor: &Panel, forward_return: &Panel, quantiles: usize) -> FactorReport {
    let q = quantiles.max(1);
    let (fa, fr) = align(factor, forward_return);
    // Snap forward returns onto the factor's exact grid so cells line up.
    let fret = fr.project_onto(&fa.dates, &fa.symbols);
    let (nrows, ncols) = fa.data.dim();

    let mut dates = Vec::new();
    let mut ic = Vec::new();
    let mut q_sum = vec![0.0_f64; q];
    let mut q_cnt = vec![0usize; q];
    let mut prev_top: Option<Vec<usize>> = None;
    let mut turnover_sum = 0.0_f64;
    let mut turnover_cnt = 0usize;

    for r in 0..nrows {
        let valid: Vec<usize> = (0..ncols)
            .filter(|&c| fa.data[[r, c]].is_finite() && fret[[r, c]].is_finite())
            .collect();
        if valid.len() < 2 {
            prev_top = None; // a gap breaks the turnover chain
            continue;
        }
        let fvals: Vec<f64> = valid.iter().map(|&c| fa.data[[r, c]]).collect();
        let rvals: Vec<f64> = valid.iter().map(|&c| fret[[r, c]]).collect();

        dates.push(fa.dates[r]);
        ic.push(spearman(&fvals, &rvals));

        // Bucket by factor rank: bucket = floor(rank0 / m * q), capped at q-1.
        let order = argsort(&fvals); // ascending factor
        let m = valid.len();
        let mut top_cols = Vec::new();
        for (rank0, &vi) in order.iter().enumerate() {
            let b = ((rank0 * q) / m).min(q - 1);
            q_sum[b] += rvals[vi];
            q_cnt[b] += 1;
            if b == q - 1 {
                top_cols.push(valid[vi]);
            }
        }
        if let Some(prev) = &prev_top {
            if !prev.is_empty() {
                let left = prev.iter().filter(|c| !top_cols.contains(c)).count();
                turnover_sum += left as f64 / prev.len() as f64;
                turnover_cnt += 1;
            }
        }
        prev_top = Some(top_cols);
    }

    let quantile_returns: Vec<f64> = (0..q)
        .map(|b| {
            if q_cnt[b] == 0 {
                f64::NAN
            } else {
                q_sum[b] / q_cnt[b] as f64
            }
        })
        .collect();
    let long_short = match (quantile_returns.first(), quantile_returns.last()) {
        (Some(&lo), Some(&hi)) => hi - lo,
        _ => f64::NAN,
    };
    let (mean_ic, ic_std) = mean_std(&ic);
    let icir = if ic_std > 0.0 {
        mean_ic / ic_std
    } else {
        f64::NAN
    };
    let top_quantile_turnover = if turnover_cnt == 0 {
        f64::NAN
    } else {
        turnover_sum / turnover_cnt as f64
    };

    FactorReport {
        dates,
        ic,
        mean_ic,
        ic_std,
        icir,
        quantiles: q,
        quantile_returns,
        long_short,
        top_quantile_turnover,
    }
}

/// Average return path around an event.
#[derive(Debug, Serialize)]
pub struct EventStudy {
    pub pre: usize,
    pub post: usize,
    /// Lags from `-pre` to `+post` (inclusive), length `pre + post + 1`.
    pub lags: Vec<i64>,
    /// Mean return across all events at each lag (NaN where no event had a
    /// finite return at that lag).
    pub avg_return: Vec<f64>,
    /// Cumulative sum of `avg_return` from `-pre` (NaN once a lag is NaN).
    pub cumulative: Vec<f64>,
    /// Number of `(date, symbol)` events contributing at lag 0.
    pub event_count: usize,
}

/// Compute an [`EventStudy`]: for every cell where `events` is `1`, gather
/// `returns` from `pre` rows before to `post` rows after the event row (same
/// symbol) and average across events by lag. Returns are raw (a market/abnormal
/// model is left to the caller — e.g. subtract a benchmark return panel first).
/// `events` and `returns` are aligned by (date, symbol); an event too close to
/// a panel edge simply contributes to fewer lags. `event_count` counts events
/// with a finite lag-0 return.
pub fn event_study(events: &Panel, returns: &Panel, pre: usize, post: usize) -> EventStudy {
    let (ev, rt) = align(events, returns);
    let ret = rt.project_onto(&ev.dates, &ev.symbols);
    let (nrows, ncols) = ev.data.dim();
    let width = pre + post + 1;

    let mut sums = vec![0.0_f64; width];
    let mut counts = vec![0usize; width];
    let mut event_count = 0usize;

    for r in 0..nrows {
        for c in 0..ncols {
            if !crate::panel::is_true(ev.data[[r, c]]) {
                continue;
            }
            // lag 0 return anchors the event count.
            if ret[[r, c]].is_finite() {
                event_count += 1;
            }
            for (k, slot) in (0..width).enumerate() {
                // lag = k - pre; row = r + lag.
                let lag = k as i64 - pre as i64;
                let rr = r as i64 + lag;
                if rr < 0 || rr as usize >= nrows {
                    continue;
                }
                let v = ret[[rr as usize, c]];
                if v.is_finite() {
                    sums[slot] += v;
                    counts[slot] += 1;
                }
            }
        }
    }

    let lags: Vec<i64> = (0..width).map(|k| k as i64 - pre as i64).collect();
    let avg_return: Vec<f64> = (0..width)
        .map(|k| {
            if counts[k] == 0 {
                f64::NAN
            } else {
                sums[k] / counts[k] as f64
            }
        })
        .collect();
    let mut cumulative = vec![f64::NAN; width];
    let mut acc = 0.0;
    for k in 0..width {
        if avg_return[k].is_finite() {
            acc += avg_return[k];
            cumulative[k] = acc;
        } else {
            // once a lag is undefined the running sum can't continue meaningfully
            cumulative[k] = f64::NAN;
        }
    }

    EventStudy {
        pre,
        post,
        lags,
        avg_return,
        cumulative,
        event_count,
    }
}

// ---- small numeric helpers (shared kernels in ops::stat) --------------------

/// Spearman rank correlation of two equal-length slices. Pearson correlation of
/// their average ranks; 0.0 when either side is constant (zero variance).
fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let ra = average_ranks(a);
    let rb = average_ranks(b);
    pearson(&ra, &rb)
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let ma = a.iter().sum::<f64>() / n;
    let mb = b.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut va = 0.0;
    let mut vb = 0.0;
    for (x, y) in a.iter().zip(b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma).powi(2);
        vb += (y - mb).powi(2);
    }
    if va == 0.0 || vb == 0.0 {
        return 0.0;
    }
    cov / (va.sqrt() * vb.sqrt())
}

/// Sample mean and std (`ddof = 1`) over finite entries.
#[inline]
fn mean_std(xs: &[f64]) -> (f64, f64) {
    mean_std_ddof(xs, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Panel;

    fn panel(dates: Vec<i32>, syms: Vec<&str>, rows: Vec<Vec<f64>>) -> Panel {
        Panel::from_rows(dates, syms.into_iter().map(String::from).collect(), rows).unwrap()
    }

    #[test]
    fn forward_returns_look_ahead_by_horizon() {
        let px = panel(
            vec![1, 2, 3, 4],
            vec!["A"],
            vec![vec![10.0], vec![11.0], vec![12.0], vec![13.0]],
        );
        let f = forward_returns(&px, 1);
        assert!((f.data[[0, 0]] - 0.1).abs() < 1e-12); // 11/10 - 1
        assert!((f.data[[2, 0]] - (13.0 / 12.0 - 1.0)).abs() < 1e-12);
        assert!(f.data[[3, 0]].is_nan()); // no future price on the last row
    }

    #[test]
    fn daily_returns_are_backward_one_day() {
        let px = panel(
            vec![1, 2, 3],
            vec!["A"],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        );
        let r = daily_returns(&px);
        assert!(r.data[[0, 0]].is_nan()); // no prior on row 0
        assert!((r.data[[1, 0]] - 0.1).abs() < 1e-12); // 11/10 - 1
        assert!((r.data[[2, 0]] - (12.0 / 11.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn factor_ic_is_perfect_when_factor_orders_returns() {
        // 2 dates, 4 symbols; forward return increases with the factor exactly.
        let factor = panel(
            vec![1, 2],
            vec!["A", "B", "C", "D"],
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![4.0, 3.0, 2.0, 1.0]],
        );
        let fret = panel(
            vec![1, 2],
            vec!["A", "B", "C", "D"],
            vec![vec![0.1, 0.2, 0.3, 0.4], vec![0.4, 0.3, 0.2, 0.1]],
        );
        let rep = factor_report(&factor, &fret, 2);
        // Rank IC is +1 each date (factor perfectly orders returns).
        assert_eq!(rep.ic.len(), 2);
        assert!((rep.mean_ic - 1.0).abs() < 1e-12);
        // Top quantile (high factor) return > bottom quantile; spread positive.
        assert!(rep.long_short > 0.0);
        // date1 top = {C,D}? high factor A,B... factor row0 = 1,2,3,4 -> top two C,D
        // returns 0.3,0.4 -> mean 0.35; bottom A,B 0.1,0.2 -> 0.15. row1 symmetric.
        assert!((rep.quantile_returns[1] - 0.35).abs() < 1e-12);
        assert!((rep.quantile_returns[0] - 0.15).abs() < 1e-12);
        assert!((rep.long_short - 0.2).abs() < 1e-12);
    }

    #[test]
    fn factor_ic_negative_when_factor_inverts_returns() {
        let factor = panel(vec![1], vec!["A", "B", "C"], vec![vec![1.0, 2.0, 3.0]]);
        let fret = panel(vec![1], vec!["A", "B", "C"], vec![vec![0.3, 0.2, 0.1]]);
        let rep = factor_report(&factor, &fret, 3);
        assert!((rep.mean_ic - (-1.0)).abs() < 1e-12);
        // ic_std NaN with one period -> icir NaN.
        assert!(rep.ic_std.is_nan());
        assert!(rep.icir.is_nan());
    }

    #[test]
    fn factor_top_quantile_turnover_tracks_membership() {
        // 3 dates, 4 names, quantiles=2 (top bucket = 2 names). The top pair
        // rotates fully from {C,D} to {A,B} to {C,D}.
        let factor = panel(
            vec![1, 2, 3],
            vec!["A", "B", "C", "D"],
            vec![
                vec![1.0, 2.0, 3.0, 4.0], // top {C,D}
                vec![4.0, 3.0, 2.0, 1.0], // top {A,B}
                vec![1.0, 2.0, 3.0, 4.0], // top {C,D}
            ],
        );
        let fret = panel(
            vec![1, 2, 3],
            vec!["A", "B", "C", "D"],
            vec![
                vec![0.0, 0.0, 0.0, 0.0],
                vec![0.0, 0.0, 0.0, 0.0],
                vec![0.0, 0.0, 0.0, 0.0],
            ],
        );
        let rep = factor_report(&factor, &fret, 2);
        // Every period the whole top bucket leaves -> turnover 1.0.
        assert!((rep.top_quantile_turnover - 1.0).abs() < 1e-12);
    }

    #[test]
    fn event_study_averages_returns_by_lag() {
        // Events for A on date 3 and B on date 3. Returns constant per symbol so
        // the average path is trivial to check. pre=1, post=1.
        let events = panel(
            vec![1, 2, 3, 4],
            vec!["A", "B"],
            vec![
                vec![0.0, 0.0],
                vec![0.0, 0.0],
                vec![1.0, 1.0], // event at row 2 (date 3) for both
                vec![0.0, 0.0],
            ],
        );
        let returns = panel(
            vec![1, 2, 3, 4],
            vec!["A", "B"],
            vec![
                vec![0.01, 0.02],
                vec![0.01, 0.02], // lag -1
                vec![0.10, 0.20], // lag 0
                vec![0.05, 0.06], // lag +1
            ],
        );
        let es = event_study(&events, &returns, 1, 1);
        assert_eq!(es.event_count, 2);
        assert_eq!(es.lags, vec![-1, 0, 1]);
        assert!((es.avg_return[0] - 0.015).abs() < 1e-12); // (0.01+0.02)/2
        assert!((es.avg_return[1] - 0.15).abs() < 1e-12); // (0.10+0.20)/2
        assert!((es.avg_return[2] - 0.055).abs() < 1e-12); // (0.05+0.06)/2
                                                           // cumulative from -1: .015, .165, .22
        assert!((es.cumulative[2] - 0.22).abs() < 1e-12);
    }

    #[test]
    fn event_study_handles_edges_and_no_events() {
        // Event on the first row -> lag -1 is off the panel (fewer contributions).
        let events = panel(vec![1, 2], vec!["A"], vec![vec![1.0], vec![0.0]]);
        let returns = panel(vec![1, 2], vec!["A"], vec![vec![0.1], vec![0.2]]);
        let es = event_study(&events, &returns, 1, 1);
        assert!(es.avg_return[0].is_nan()); // lag -1 unavailable
        assert!((es.avg_return[1] - 0.1).abs() < 1e-12); // lag 0
        assert!((es.avg_return[2] - 0.2).abs() < 1e-12); // lag +1

        let none = panel(vec![1, 2], vec!["A"], vec![vec![0.0], vec![0.0]]);
        let es2 = event_study(&none, &returns, 1, 1);
        assert_eq!(es2.event_count, 0);
        assert!(es2.avg_return.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn serializes_to_json() {
        let factor = panel(vec![1], vec!["A", "B"], vec![vec![1.0, 2.0]]);
        let fret = panel(vec![1], vec!["A", "B"], vec![vec![0.1, 0.2]]);
        let json = serde_json::to_string(&factor_report(&factor, &fret, 2)).unwrap();
        assert!(json.contains("\"mean_ic\""));
        assert!(json.contains("\"quantile_returns\""));
        let events = panel(vec![1, 2], vec!["A"], vec![vec![0.0], vec![1.0]]);
        let es = event_study(&events, &fret_pad(), 1, 0);
        let ejson = serde_json::to_string(&es).unwrap();
        assert!(ejson.contains("\"avg_return\""));
    }

    fn fret_pad() -> Panel {
        panel(vec![1, 2], vec!["A"], vec![vec![0.1], vec![0.2]])
    }
}
