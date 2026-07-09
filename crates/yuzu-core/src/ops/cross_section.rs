//! Per-row cross-sectional selection and transforms:
//! `is_largest`/`is_smallest` pick the top/bottom `n` non-NaN cells in each row.
//! NaN is never selected; ties keep original column order (Rust's stable `sort_by`).
//! Preprocess toolkit: `winsorize`, `zscore`, `bucket`, `demean` (all NaN-aware).

use crate::panel::{bool_to_f64, Panel};
use ndarray::Array2;

/// Linear-interpolation quantile of a **sorted** non-empty slice (pandas default).
fn sorted_quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q.clamp(0.0, 1.0) * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

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

    /// Per-row winsorize: clip each finite value to the empirical `[lower, upper]`
    /// quantiles of that row (linear interpolation, same as `quantile_row`).
    /// `lower`/`upper` are clamped to `[0, 1]`; if `lower > upper` they are swapped.
    /// NaN stays NaN; empty rows stay all-NaN.
    pub fn winsorize(&self, lower: f64, upper: f64) -> Panel {
        let (lo_q, hi_q) = if lower <= upper {
            (lower.clamp(0.0, 1.0), upper.clamp(0.0, 1.0))
        } else {
            (upper.clamp(0.0, 1.0), lower.clamp(0.0, 1.0))
        };
        let (nrows, ncols) = self.data.dim();
        let mut data = self.data.clone();
        for r in 0..nrows {
            let mut vals: Vec<f64> = (0..ncols)
                .map(|c| data[[r, c]])
                .filter(|v| !v.is_nan())
                .collect();
            if vals.is_empty() {
                continue;
            }
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let lo = sorted_quantile(&vals, lo_q);
            let hi = sorted_quantile(&vals, hi_q);
            for c in 0..ncols {
                let v = data[[r, c]];
                if !v.is_nan() {
                    data[[r, c]] = v.clamp(lo, hi);
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// Per-row z-score: `(x − mean) / std` over non-NaN cells, **population** std
    /// (`ddof = 0`). Empty rows stay NaN. When `std == 0` (constant finite row),
    /// finite cells become `0.0`.
    pub fn zscore(&self) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut data = Array2::from_elem((nrows, ncols), f64::NAN);
        for r in 0..nrows {
            let vals: Vec<f64> = (0..ncols)
                .map(|c| self.data[[r, c]])
                .filter(|v| !v.is_nan())
                .collect();
            if vals.is_empty() {
                continue;
            }
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            let var = vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
            let std = var.sqrt();
            for c in 0..ncols {
                let v = self.data[[r, c]];
                if v.is_nan() {
                    continue;
                }
                data[[r, c]] = if std == 0.0 { 0.0 } else { (v - mean) / std };
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// Per-row quantile buckets labeled **1..=n** (NaN stays NaN). Uses average
    /// ranks for ties (same as `rank_cs`), then
    /// `bucket = floor((rank − 1) / count * n) + 1` capped at `n`.
    /// `n == 0` or empty rows → all NaN.
    pub fn bucket(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut data = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data,
            };
        }
        for r in 0..nrows {
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
            if valid.is_empty() {
                continue;
            }
            valid.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let count = valid.len() as f64;
            let mut i = 0usize;
            while i < valid.len() {
                let mut j = i + 1;
                while j < valid.len() && valid[j].1 == valid[i].1 {
                    j += 1;
                }
                // average 1-based ranks for the tie group
                let avg_rank = ((i + 1 + j) as f64) / 2.0;
                let mut b = ((avg_rank - 1.0) / count * n as f64).floor() as usize + 1;
                if b > n {
                    b = n;
                }
                if b == 0 {
                    b = 1;
                }
                for k in i..j {
                    data[[r, valid[k].0]] = b as f64;
                }
                i = j;
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// Per-row demean: subtract the mean of non-NaN cells. NaN stays NaN; empty
    /// rows stay all-NaN.
    pub fn demean(&self) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut data = self.data.clone();
        for r in 0..nrows {
            let mut sum = 0.0;
            let mut count = 0usize;
            for c in 0..ncols {
                let v = data[[r, c]];
                if !v.is_nan() {
                    sum += v;
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            let mean = sum / count as f64;
            for c in 0..ncols {
                if !data[[r, c]].is_nan() {
                    data[[r, c]] -= mean;
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

#[cfg(test)]
mod cs_preprocess_tests {
    use crate::panel::Panel;
    use ndarray::array;

    fn row3(a: f64, b: f64, c: f64) -> Panel {
        Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into()],
            array![[a, b, c]],
        )
        .unwrap()
    }

    #[test]
    fn demean_subtracts_row_mean_preserves_nan() {
        let p = row3(1.0, 3.0, f64::NAN);
        let d = p.demean();
        assert!((d.data[[0, 0]] - (-1.0)).abs() < 1e-12); // mean 2
        assert!((d.data[[0, 1]] - 1.0).abs() < 1e-12);
        assert!(d.data[[0, 2]].is_nan());
    }

    #[test]
    fn zscore_zero_std_is_zero_and_empty_is_nan() {
        let flat = row3(5.0, 5.0, 5.0);
        let z = flat.zscore();
        assert_eq!(z.data[[0, 0]], 0.0);
        assert_eq!(z.data[[0, 1]], 0.0);

        let empty = row3(f64::NAN, f64::NAN, f64::NAN);
        let ze = empty.zscore();
        assert!(ze.data[[0, 0]].is_nan());
    }

    #[test]
    fn zscore_matches_population_definition() {
        // values 1,2,3 mean=2, var=((1)+(0)+(1))/3=2/3, std=sqrt(2/3)
        let p = row3(1.0, 2.0, 3.0);
        let z = p.zscore();
        let s = (2.0f64 / 3.0).sqrt();
        assert!((z.data[[0, 0]] - (1.0 - 2.0) / s).abs() < 1e-12);
        assert!((z.data[[0, 1]] - 0.0).abs() < 1e-12);
        assert!((z.data[[0, 2]] - (3.0 - 2.0) / s).abs() < 1e-12);
    }

    #[test]
    fn winsorize_clips_to_empirical_quantiles() {
        // 1,2,3,4,100 — lower=0 upper=0.5 clips high end toward median
        let p = Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into(), "D".into(), "E".into()],
            array![[1.0, 2.0, 3.0, 4.0, 100.0]],
        )
        .unwrap();
        let w = p.winsorize(0.0, 0.5);
        // q0.5 of [1,2,3,4,100] = 3
        assert_eq!(w.data[[0, 0]], 1.0);
        assert_eq!(w.data[[0, 4]], 3.0);
    }

    #[test]
    fn bucket_labels_one_through_n() {
        let p = Panel::new(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into(), "D".into()],
            array![[1.0, 2.0, 3.0, 4.0]],
        )
        .unwrap();
        let b = p.bucket(2);
        // ranks 1,2,3,4 → buckets floor((r-1)/4*2)+1 → 1,1,2,2
        assert_eq!(b.data[[0, 0]], 1.0);
        assert_eq!(b.data[[0, 1]], 1.0);
        assert_eq!(b.data[[0, 2]], 2.0);
        assert_eq!(b.data[[0, 3]], 2.0);
    }
}
