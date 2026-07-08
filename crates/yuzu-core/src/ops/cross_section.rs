//! Per-row cross-sectional selection: `is_largest`/`is_smallest` pick the top/bottom
//! `n` non-NaN cells in each row. NaN is never selected; ties keep original column
//! order (Rust's stable `sort_by`) — a standard top-n selection.

use crate::panel::{bool_to_f64, Panel};
use ndarray::Array2;

impl Panel {
    fn top_n(&self, n: usize, largest: bool) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), 0.0);
        for r in 0..nrows {
            // valid (col, value) pairs
            let mut valid: Vec<(usize, f64)> = (0..ncols)
                .filter_map(|c| {
                    let v = self.data[[r, c]];
                    if v.is_nan() {
                        None
                    } else {
                        Some((c, v))
                    }
                })
                .collect();
            if n == 0 {
                continue;
            }
            if valid.len() <= n {
                for (c, _) in valid {
                    out[[r, c]] = 1.0;
                }
                continue;
            }
            // stable sort: by value (desc for largest / asc for smallest),
            // ties keep original column order (already ascending by c).
            valid.sort_by(|a, b| {
                let ord = a.1.partial_cmp(&b.1).unwrap();
                if largest {
                    ord.reverse()
                } else {
                    ord
                }
            });
            for &(c, _) in valid.iter().take(n) {
                out[[r, c]] = 1.0;
            }
        }
        // ensure exactly bool-valued
        let data = out.mapv(|x| bool_to_f64(x == 1.0));
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    pub fn is_largest(&self, n: usize) -> Panel {
        self.top_n(n, true)
    }
    pub fn is_smallest(&self, n: usize) -> Panel {
        self.top_n(n, false)
    }
}

#[cfg(test)]
mod tests {
    use crate::panel::Panel;

    #[test]
    fn nan_never_selected_and_ties_pick_earlier_column() {
        let p = Panel::from_rows(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into()],
            vec![vec![5.0, 5.0, f64::NAN]],
        )
        .unwrap();
        let r = p.is_largest(1);
        assert_eq!(r.data[[0, 0]], 1.0); // A wins tie (earlier column)
        assert_eq!(r.data[[0, 1]], 0.0);
        assert_eq!(r.data[[0, 2]], 0.0); // NaN never selected
    }
}

impl Panel {
    /// Scale each row so gross weight sums to 1: `w[c] / Σ|w[row]|` over the
    /// row's non-NaN cells. NaN cells stay NaN; a row whose gross sum is 0 (or
    /// all-NaN) is left unchanged. Turns a raw signal into explicit portfolio
    /// weights — e.g. `normalize_row(sig / std(close, 20))` is inverse-vol
    /// weighting.
    pub fn normalize_row(&self) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut data = self.data.clone();
        for r in 0..nrows {
            let total: f64 = (0..ncols)
                .map(|c| data[[r, c]])
                .filter(|v| !v.is_nan())
                .map(f64::abs)
                .sum();
            if total > 0.0 {
                for c in 0..ncols {
                    data[[r, c]] /= total;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }
}

#[cfg(test)]
mod normalize_row_tests {
    use crate::panel::Panel;
    use ndarray::array;

    #[test]
    fn scales_rows_to_unit_gross_preserving_nan_and_zero_rows() {
        let p = Panel::new(
            vec![20240102, 20240103, 20240104],
            vec!["A".into(), "B".into(), "C".into()],
            array![
                [1.0, 3.0, f64::NAN], // gross 4 -> 0.25, 0.75, NaN
                [-1.0, 1.0, 2.0],     // gross 4 -> -0.25, 0.25, 0.5 (long/short)
                [0.0, 0.0, f64::NAN]  // gross 0 -> unchanged
            ],
        )
        .unwrap();
        let n = p.normalize_row();
        assert_eq!(n.data[[0, 0]], 0.25);
        assert_eq!(n.data[[0, 1]], 0.75);
        assert!(n.data[[0, 2]].is_nan());
        assert_eq!(n.data[[1, 0]], -0.25);
        assert_eq!(n.data[[1, 2]], 0.5);
        assert_eq!(n.data[[2, 0]], 0.0);
        assert!(n.data[[2, 2]].is_nan());
    }
}
