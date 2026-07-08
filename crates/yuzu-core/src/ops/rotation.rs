//! `hold_until`: rank-priority rotation — the one genuinely sequential loop.
//! Rank-priority rotation with optional price stops:
//! select up to `nstocks_limit` names each day by entry-rank + held-position priority,
//! with optional stop_loss / take_profit / trail_stop exits.

use crate::align::align;
use crate::panel::{bool_to_f64, is_true, Panel};
use ndarray::{Array1, Array2};

pub struct HoldUntilOpts {
    pub nstocks_limit: Option<usize>,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub trail_stop: f64,
    pub trail_stop_activation: f64,
    pub rank: Option<Panel>,
    pub price: Option<Panel>,
}

impl Default for HoldUntilOpts {
    fn default() -> Self {
        HoldUntilOpts {
            nstocks_limit: None,
            stop_loss: f64::NEG_INFINITY,
            take_profit: f64::INFINITY,
            trail_stop: f64::INFINITY,
            trail_stop_activation: 0.0,
            rank: None,
            price: None,
        }
    }
}

/// indices that would sort `xs` ascending, ties broken by original index (stable).
fn argsort_stable(xs: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|&a, &b| xs[a].partial_cmp(&xs[b]).unwrap().then(a.cmp(&b)));
    idx
}

impl Panel {
    pub fn hold_until(&self, exit: &Panel, opts: &HoldUntilOpts) -> Panel {
        // Align entry + exit (union rows, intersect cols).
        let (entry, exit) = align(self, exit);
        let n = entry.ncols();
        let nstocks = opts.nstocks_limit.unwrap_or(n);

        // ranking normalized to [0,1]; default all-ones -> normalized 0.
        let ranking = match &opts.rank {
            Some(r) => {
                let (_, r2) = align(&entry, r);
                normalize_ranking(&r2.data)
            }
            None => Array2::from_elem(entry.data.dim(), 0.0),
        };

        let entry_i = entry.data.mapv(|x| if is_true(x) { 1.0 } else { 0.0 });
        let mut exit_i = exit.data.mapv(|x| if is_true(x) { 1.0 } else { 0.0 });
        let price = opts.price.as_ref().map(|p| {
            let (_, p2) = align(&entry, p);
            p2.data
        });

        let nrows = entry.nrows();
        let mut ret = Array2::from_elem((nrows, n), 0.0_f64);

        // Row 0
        {
            let row0: Vec<f64> = (0..n).map(|c| entry_i[[0, c]]).collect();
            for &c in argsort_stable(&row0).iter().rev().take(nstocks) {
                ret[[0, c]] = 1.0;
            }
            for c in 0..n {
                if exit_i[[0, c]] == 1.0 || entry_i[[0, c]] == 0.0 {
                    ret[[0, c]] = 0.0;
                }
            }
        }

        let mut entry_price = Array1::from_elem(n, f64::NAN);
        let mut max_return = Array1::from_elem(n, 1.0_f64);

        for i in 1..nrows {
            apply_price_stops(
                &ret,
                &mut entry_price,
                &mut max_return,
                &mut exit_i,
                price.as_ref(),
                i,
                opts,
            );

            let mut rank: Vec<f64> = (0..n)
                .map(|c| entry_i[[i, c]] * ranking[[i, c]] + ret[[i - 1, c]] * 3.0)
                .collect();
            for c in 0..n {
                if exit_i[[i, c]] == 1.0 || (entry_i[[i, c]] == 0.0 && ret[[i - 1, c]] == 0.0) {
                    rank[c] = -1.0;
                }
            }
            for &c in argsort_stable(&rank).iter().rev().take(nstocks) {
                ret[[i, c]] = 1.0;
            }
            for c in 0..n {
                if rank[c] == -1.0 {
                    ret[[i, c]] = 0.0;
                }
            }
        }

        let data = ret.mapv(bool_to_f64_from_unit);
        Panel {
            dates: entry.dates,
            symbols: entry.symbols,
            data,
        }
    }
}

fn bool_to_f64_from_unit(x: f64) -> f64 {
    bool_to_f64(x == 1.0)
}

fn normalize_ranking(r: &Array2<f64>) -> Array2<f64> {
    let finite: Vec<f64> = r.iter().copied().filter(|x| x.is_finite()).collect();
    let (min, max) = finite
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| {
            (lo.min(x), hi.max(x))
        });
    let span = max - min;
    r.mapv(|x| {
        if !x.is_finite() || span == 0.0 {
            0.0
        } else {
            (x - min) / span
        }
    })
}

// Helper fns below the tests are intentional; keeping the module here reads best.
#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Panel;

    fn p(rows: Vec<Vec<f64>>) -> Panel {
        let dates = (0..rows.len() as i32).map(|i| 20240102 + i).collect();
        Panel::from_rows(dates, vec!["A".into(), "B".into(), "C".into()], rows).unwrap()
    }

    #[test]
    fn rank_decides_among_fresh_entries() {
        // Day 0: nobody enters. Day 1: A and B both enter, limit 1.
        // ranking prefers B (higher value -> higher normalized rank), so B is held.
        let entry = p(vec![vec![0.0, 0.0, 0.0], vec![1.0, 1.0, 0.0]]);
        let exit = p(vec![vec![0.0, 0.0, 0.0], vec![0.0, 0.0, 0.0]]);
        let rank = p(vec![vec![1.0, 1.0, 1.0], vec![1.0, 3.0, 1.0]]);
        let opts = HoldUntilOpts {
            nstocks_limit: Some(1),
            rank: Some(rank),
            ..Default::default()
        };
        let r = entry.hold_until(&exit, &opts);
        assert_eq!(r.data[[1, 0]], 0.0); // A loses
        assert_eq!(r.data[[1, 1]], 1.0); // B wins on rank
        assert_eq!(r.data[[1, 2]], 0.0); // C never entered
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_price_stops(
    ret: &Array2<f64>,
    entry_price: &mut Array1<f64>,
    max_return: &mut Array1<f64>,
    exit_i: &mut Array2<f64>,
    price: Option<&Array2<f64>>,
    i: usize,
    opts: &HoldUntilOpts,
) {
    if opts.stop_loss == f64::NEG_INFINITY
        && opts.take_profit == f64::INFINITY
        && opts.trail_stop == f64::INFINITY
    {
        return;
    }
    let price = price.expect("price required when stop rules enabled");
    let n = ret.ncols();
    for c in 0..n {
        let is_entry = if i > 1 {
            ret[[i - 2, c]] == 0.0
        } else {
            ret[[i - 1, c]] == 1.0
        };
        let waiting = entry_price[c].is_nan() && ret[[i - 1, c]] == 1.0;
        if is_entry || waiting {
            entry_price[c] = price[[i, c]];
            max_return[c] = 1.0;
        }
        let held = ret[[i - 1, c]] == 1.0;
        let returns = price[[i, c]] / entry_price[c];
        let mut stop = held
            && (returns > 1.0 + opts.take_profit.abs() || returns < 1.0 - opts.stop_loss.abs());
        if opts.trail_stop != f64::INFINITY {
            if held {
                max_return[c] = max_return[c].max(returns);
            }
            let active = max_return[c] >= 1.0 + opts.trail_stop_activation.abs();
            stop = stop || (held && active && returns < max_return[c] - opts.trail_stop.abs());
        }
        let exited = ret[[i - 1, c]] == 0.0 && !entry_price[c].is_nan();
        if exited {
            entry_price[c] = f64::NAN;
            max_return[c] = 1.0;
        }
        if stop {
            exit_i[[i, c]] = 1.0;
        }
    }
}
