//! `hold_until`: rank-priority rotation — the one genuinely sequential loop.
//! Select up to `nstocks_limit` names each day by entry-rank + held-position
//! priority. Price stops (stop-loss / take-profit / trailing) are **not** here —
//! they moved to the execution layer (`BacktestConfig::stops`, see `backtest.rs`),
//! so `hold_until` is pure selection.

use crate::align::align;
use crate::ops::stat::argsort_stable;
use crate::panel::{bool_to_f64, is_true, Panel};
use ndarray::Array2;

#[derive(Default)]
pub struct HoldUntilOpts {
    pub nstocks_limit: Option<usize>,
    pub rank: Option<Panel>,
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
        let exit_i = exit.data.mapv(|x| if is_true(x) { 1.0 } else { 0.0 });

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

        for i in 1..nrows {
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
