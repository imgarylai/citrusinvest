//! Per-row cross-sectional selection and transforms:
//! `is_largest`/`is_smallest` pick the top/bottom `n` non-NaN cells in each row.
//! NaN is never selected; ties keep original column order (Rust's stable `sort_by`).
//! Preprocess toolkit: `winsorize`, `zscore`, `bucket`, `demean` (all NaN-aware).

use crate::align::align;
use crate::ops::stat::{average_ranks, cmp_f64, mean_std, sort_f64s, sorted_quantile};
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
            // total_cmp: NaN was filtered above; Inf is ordered without panic.
            valid.sort_by(|a, b| {
                let ord = cmp_f64(a.1, b.1);
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

    /// `vol_target`: scale each row of this weight panel so the implied
    /// portfolio's annualized realized volatility targets `target`, capping the
    /// scale at 1 (deleverage only — never lever up). The scale for row `t` is
    /// `min(1, target / realized_vol[t])`, where `realized_vol[t]` is the
    /// annualized (×√252) sample std (ddof=1) of the portfolio's daily returns
    /// over the trailing `n`-return window ending at `t`.
    ///
    /// The portfolio's daily return on row `t` is `Σ_c w[t,c] · (p[t,c]/p[t-1,c]
    /// − 1)` over `prices` (`p`), aligned onto this panel's grid; a cell with a
    /// missing weight or price contributes nothing.
    ///
    /// Warmup: until `n` finite portfolio returns are available in the window
    /// (i.e. before row `n`, and across any price gap that voids a window),
    /// `realized_vol` is undefined and the row's weights pass through unscaled.
    /// A zero realized vol also passes through (no finite scale to apply).
    /// `n < 2` (std undefined) and `target <= 0` / NaN are no-ops — a degenerate
    /// target must not zero or flip the book. NaN weight cells stay NaN.
    ///
    /// The window ends at the current row, so it includes row `t`'s own return;
    /// for strictly-causal sizing, lag the weights (e.g. `shift`) relative to the
    /// prices. I/O-free and composable — prices come in as an explicit argument.
    pub fn vol_target(&self, prices: &Panel, target: f64, n: usize) -> Panel {
        let unchanged = || Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: self.data.clone(),
        };
        if n < 2 || target <= 0.0 || target.is_nan() {
            return unchanged();
        }
        // Align prices onto this panel's exact (dates × symbols) grid.
        let (_, px_aligned) = align(self, prices);
        let px = px_aligned.project_onto(&self.dates, &self.symbols);

        let (nrows, ncols) = self.data.dim();
        // Implied-portfolio daily returns (NaN where no cell contributes).
        let mut pret = vec![f64::NAN; nrows];
        for t in 1..nrows {
            let mut acc = 0.0;
            let mut any = false;
            for c in 0..ncols {
                let w = self.data[[t, c]];
                let (p0, p1) = (px[[t - 1, c]], px[[t, c]]);
                if w.is_finite() && p0.is_finite() && p1.is_finite() && p0 != 0.0 {
                    acc += w * (p1 / p0 - 1.0);
                    any = true;
                }
            }
            if any {
                pret[t] = acc;
            }
        }

        let ann = 252.0_f64.sqrt();
        let m = n as f64;
        let mut data = self.data.clone();
        for t in 0..nrows {
            if t + 1 < n {
                continue; // window would underflow — fewer than n rows so far
            }
            let window = &pret[t + 1 - n..=t];
            if window.iter().any(|v| !v.is_finite()) {
                continue; // warmup or a price gap voids the window → passthrough
            }
            let mean = window.iter().sum::<f64>() / m;
            let var = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (m - 1.0);
            let vol = var.sqrt() * ann;
            if vol > 0.0 {
                let scale = (target / vol).min(1.0);
                for c in 0..ncols {
                    data[[t, c]] *= scale;
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
            sort_f64s(&mut vals);
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
            let (mean, std) = mean_std(&vals, 0);
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
            let mut cols = Vec::new();
            let mut vals = Vec::new();
            for c in 0..ncols {
                let v = self.data[[r, c]];
                if !v.is_nan() {
                    cols.push(c);
                    vals.push(v);
                }
            }
            if vals.is_empty() {
                continue;
            }
            let ranks = average_ranks(&vals);
            let count = vals.len() as f64;
            for (i, &c) in cols.iter().enumerate() {
                let avg_rank = ranks[i];
                let mut b = ((avg_rank - 1.0) / count * n as f64).floor() as usize + 1;
                if b > n {
                    b = n;
                }
                if b == 0 {
                    b = 1;
                }
                data[[r, c]] = b as f64;
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
            let vals: Vec<f64> = (0..ncols)
                .map(|c| data[[r, c]])
                .filter(|v| !v.is_nan())
                .collect();
            if vals.is_empty() {
                continue;
            }
            let (mean, _) = mean_std(&vals, 0);
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
mod vol_target_tests {
    use crate::panel::Panel;
    use ndarray::array;

    // 1 symbol, weights 1.0; prices give returns +.10,-.10,+.10; n=2.
    fn fixture() -> (Panel, Panel) {
        let dates = vec![20240102, 20240103, 20240104, 20240105];
        let w = Panel::new(
            dates.clone(),
            vec!["A".into()],
            array![[1.0], [1.0], [1.0], [1.0]],
        )
        .unwrap();
        let px = Panel::new(
            dates,
            vec!["A".into()],
            array![[100.0], [110.0], [99.0], [108.9]],
        )
        .unwrap();
        (w, px)
    }

    #[test]
    fn warmup_passes_through_then_deleverages_to_target() {
        let (w, px) = fixture();
        let out = w.vol_target(&px, 0.10, 2);
        // Rows 0-1 warmup: no full finite window (row0 return is NaN).
        assert_eq!(out.data[[0, 0]], 1.0);
        assert_eq!(out.data[[1, 0]], 1.0);
        // Row 2: vol = sqrt(0.02 * 252) = 2.244994; scale = 0.10 / vol.
        let vol = (0.02f64 * 252.0).sqrt();
        let scale = 0.10 / vol;
        assert!((out.data[[2, 0]] - scale).abs() < 1e-12);
        assert!((out.data[[3, 0]] - scale).abs() < 1e-12);
    }

    #[test]
    fn low_vol_never_levers_up() {
        // Tiny returns -> target/vol > 1 -> scale capped at 1 (weights unchanged).
        let dates = vec![20240102, 20240103, 20240104];
        let w = Panel::new(dates.clone(), vec!["A".into()], array![[1.0], [1.0], [1.0]]).unwrap();
        let px = Panel::new(dates, vec!["A".into()], array![[100.0], [100.1], [100.0]]).unwrap();
        let out = w.vol_target(&px, 0.50, 2);
        assert_eq!(out.data[[2, 0]], 1.0);
    }

    #[test]
    fn degenerate_args_are_noops() {
        let (w, px) = fixture();
        // n < 2, non-positive target, and NaN target all pass weights through.
        for out in [
            w.vol_target(&px, 0.10, 1),
            w.vol_target(&px, 0.0, 2),
            w.vol_target(&px, -0.1, 2),
            w.vol_target(&px, f64::NAN, 2),
        ] {
            assert!(out.data.iter().all(|&v| v == 1.0));
        }
    }

    #[test]
    fn nan_weight_cells_stay_nan() {
        let dates = vec![20240102, 20240103, 20240104];
        let w = Panel::new(
            dates.clone(),
            vec!["A".into(), "B".into()],
            array![[1.0, f64::NAN], [1.0, f64::NAN], [1.0, f64::NAN]],
        )
        .unwrap();
        let px = Panel::new(
            dates,
            vec!["A".into(), "B".into()],
            array![[100.0, 100.0], [110.0, 100.0], [99.0, 100.0]],
        )
        .unwrap();
        let out = w.vol_target(&px, 0.10, 2);
        assert!(out.data[[2, 1]].is_nan()); // scaled column-B weight was NaN
        assert!(out.data[[2, 0]] < 1.0); // column A got deleveraged
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
