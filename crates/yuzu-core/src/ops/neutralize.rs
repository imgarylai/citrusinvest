//! Cross-sectional neutralization & industry grouping ops.
//! `neutralize` runs a per-date OLS and returns residuals; the industry ops
//! reduce to per-date group arithmetic (see each method).

use crate::align::align;
use crate::ops::linalg::solve_ols;
use crate::panel::{bool_to_f64, Panel};
use ndarray::Array2;
use std::collections::HashMap;

impl Panel {
    /// Reindex another panel's values onto `self`'s exact (dates, symbols) grid,
    /// NaN where a cell is absent. Lets per-row ops assume a shared axis.
    pub(crate) fn project_onto(&self, dates: &[i32], symbols: &[String]) -> Array2<f64> {
        let row_of: HashMap<i32, usize> = self
            .dates
            .iter()
            .enumerate()
            .map(|(i, &d)| (d, i))
            .collect();
        let col_of: HashMap<&str, usize> = self
            .symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (s.as_str(), i))
            .collect();
        let mut out = Array2::from_elem((dates.len(), symbols.len()), f64::NAN);
        for (r, d) in dates.iter().enumerate() {
            let Some(&sr) = row_of.get(d) else { continue };
            for (c, s) in symbols.iter().enumerate() {
                if let Some(&sc) = col_of.get(s.as_str()) {
                    out[[r, c]] = self.data[[sr, sc]];
                }
            }
        }
        out
    }

    /// `neutralize`: per date, OLS-regress the factor on `neutralizers`
    /// (plus an intercept when `add_const`), return residuals. A date is all-NaN
    /// when fewer than `neutralizers.len() + 1` cells are valid across the factor
    /// and every neutralizer.
    pub fn neutralize(&self, neutralizers: &[Panel], add_const: bool) -> Panel {
        // Project every neutralizer onto self's grid (align first so a differing
        // axis still resolves by label, then snap to self's exact grid).
        let projected: Vec<Array2<f64>> = neutralizers
            .iter()
            .map(|n| {
                let (_, n2) = align(self, n);
                n2.project_onto(&self.dates, &self.symbols)
            })
            .collect();

        let (nrows, ncols) = self.data.dim();
        let nfac = neutralizers.len();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        let kcols = nfac + usize::from(add_const);

        for r in 0..nrows {
            // valid columns: factor and every neutralizer present
            let valid: Vec<usize> = (0..ncols)
                .filter(|&c| {
                    self.data[[r, c]].is_finite() && projected.iter().all(|p| p[[r, c]].is_finite())
                })
                .collect();
            if valid.len() < nfac + 1 {
                continue; // row stays NaN
            }
            let m = valid.len();
            let mut x = Array2::<f64>::zeros((m, kcols));
            let mut y = vec![0.0f64; m];
            for (i, &c) in valid.iter().enumerate() {
                let mut k = 0;
                if add_const {
                    x[[i, 0]] = 1.0;
                    k = 1;
                }
                for (f, p) in projected.iter().enumerate() {
                    x[[i, k + f]] = p[[r, c]];
                }
                y[i] = self.data[[r, c]];
            }
            let Some(beta) = solve_ols(&x, &y) else {
                continue;
            };
            for (i, &c) in valid.iter().enumerate() {
                let mut fitted = 0.0;
                for j in 0..kcols {
                    fitted += x[[i, j]] * beta[j];
                }
                out[[r, c]] = y[i] - fitted;
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Group `valid_cols` by their industry. Columns missing from `industry`
    /// fall into "其他" (the default "other" bucket) —
    /// but for US single-sector data every tracked symbol has a sector.
    /// Returns groups in first-seen column order (stable).
    pub(crate) fn group_cols_by_industry<'a>(
        &self,
        industry: &'a HashMap<String, String>,
        valid_cols: &[usize],
    ) -> Vec<(&'a str, Vec<usize>)> {
        const OTHER: &str = "其他";
        let mut order: Vec<&str> = Vec::new();
        let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
        for &c in valid_cols {
            let cat = industry
                .get(&self.symbols[c])
                .map(String::as_str)
                .unwrap_or(OTHER);
            groups
                .entry(cat)
                .or_insert_with(|| {
                    order.push(cat);
                    Vec::new()
                })
                .push(c);
        }
        order
            .into_iter()
            .filter_map(|cat| groups.remove(cat).map(|cols| (cat, cols)))
            .collect()
    }

    /// `neutralize_industry`: residual of regressing the factor on industry
    /// dummies. That residual is algebraically the within-industry demean, so we
    /// compute it directly. A date is all-NaN when fewer than 2 cells are valid or
    /// fewer than 2 distinct industries are present.
    /// ponytail: industry-dummy OLS residual == within-group demean for any
    /// `add_const`; `add_const` is kept for API parity but does not change values.
    pub fn neutralize_industry(
        &self,
        industry: &HashMap<String, String>,
        _add_const: bool,
    ) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        for r in 0..nrows {
            let valid: Vec<usize> = (0..ncols)
                .filter(|&c| self.data[[r, c]].is_finite())
                .collect();
            if valid.len() < 2 {
                continue;
            }
            let groups = self.group_cols_by_industry(industry, &valid);
            if groups.len() < 2 {
                continue;
            }
            for (_, cols) in &groups {
                let mean = cols.iter().map(|&c| self.data[[r, c]]).sum::<f64>() / cols.len() as f64;
                for &c in cols {
                    out[[r, c]] = self.data[[r, c]] - mean;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// `industry_rank`: per date, percentile-rank within each industry
    /// (pandas `rank(pct=True)`, average ties → `avg_rank / group_size`). When
    /// `categories` is `Some`, only those industries are ranked; every other
    /// symbol (and any NaN cell) is NaN.
    pub fn industry_rank(
        &self,
        industry: &HashMap<String, String>,
        categories: Option<&[String]>,
    ) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let allow = |cat: &str| categories.is_none_or(|cs| cs.iter().any(|c| c == cat));
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        for r in 0..nrows {
            let valid: Vec<usize> = (0..ncols)
                .filter(|&c| self.data[[r, c]].is_finite())
                .collect();
            for (cat, cols) in self.group_cols_by_industry(industry, &valid) {
                if !allow(cat) {
                    continue;
                }
                let n = cols.len() as f64;
                for &c in &cols {
                    let v = self.data[[r, c]];
                    // average rank: (#less + (#equal + 1)/2) ; pct = rank / n
                    let mut less = 0.0;
                    let mut equal = 0.0;
                    for &c2 in &cols {
                        let v2 = self.data[[r, c2]];
                        if v2 < v {
                            less += 1.0;
                        } else if v2 == v {
                            equal += 1.0;
                        }
                    }
                    out[[r, c]] = (less + (equal + 1.0) / 2.0) / n;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// `cap_industry`: per date, cap each industry's **gross** weight (Σ|w| over
    /// the group) at `max_weight` by scaling that group's names down by
    /// `max_weight / gross` (sign-preserving, so long/short books stay balanced
    /// within the group). Groups already at or under the cap are untouched; the
    /// residual freed by a scaled-down group is left as cash — no redistribution
    /// (the NAV loop's row-normalize takes the book from there). NaN cells stay
    /// NaN and don't count toward a group's gross.
    ///
    /// Grouping reuses the internal `group_cols_by_industry` helper, so symbols
    /// missing from `industry` share the single "其他" bucket. With an
    /// **empty industry map** the whole cross-section is that one bucket, so the
    /// op caps total gross exposure at `max_weight`.
    ///
    /// `max_weight <= 0` (or NaN) is a no-op — a degenerate cap must not zero the
    /// book.
    pub fn cap_industry(&self, industry: &HashMap<String, String>, max_weight: f64) -> Panel {
        if max_weight <= 0.0 || max_weight.is_nan() {
            return self.clone();
        }
        let (nrows, ncols) = self.data.dim();
        let mut data = self.data.clone();
        for r in 0..nrows {
            let valid: Vec<usize> = (0..ncols)
                .filter(|&c| self.data[[r, c]].is_finite())
                .collect();
            if valid.is_empty() {
                continue;
            }
            for (_, cols) in self.group_cols_by_industry(industry, &valid) {
                let gross: f64 = cols.iter().map(|&c| self.data[[r, c]].abs()).sum();
                if gross > max_weight {
                    let factor = max_weight / gross;
                    for &c in &cols {
                        data[[r, c]] *= factor;
                    }
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// `groupby_category().<agg>()`: group columns by sector and aggregate
    /// per date. Result columns are the sorted distinct sectors. NaN cells are
    /// skipped; a group with no valid cell that date yields NaN. `std` is the
    /// sample standard deviation (ddof=1), matching pandas.
    pub fn groupby_category(
        &self,
        industry: &HashMap<String, String>,
        agg: &str,
    ) -> Result<Panel, crate::error::EngineError> {
        const OTHER: &str = "其他";
        // sorted distinct categories present among self's symbols
        let mut cats: Vec<String> = self
            .symbols
            .iter()
            .map(|s| {
                industry
                    .get(s)
                    .cloned()
                    .unwrap_or_else(|| OTHER.to_string())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        let cat_cols: Vec<Vec<usize>> = cats
            .iter()
            .map(|cat| {
                (0..self.ncols())
                    .filter(|&c| {
                        industry
                            .get(&self.symbols[c])
                            .map(String::as_str)
                            .unwrap_or(OTHER)
                            == cat
                    })
                    .collect()
            })
            .collect();

        let nrows = self.nrows();
        let mut data = Array2::from_elem((nrows, cats.len()), f64::NAN);
        for r in 0..nrows {
            for (g, cols) in cat_cols.iter().enumerate() {
                let vals: Vec<f64> = cols
                    .iter()
                    .map(|&c| self.data[[r, c]])
                    .filter(|v| v.is_finite())
                    .collect();
                if vals.is_empty() {
                    continue;
                }
                data[[r, g]] = aggregate(agg, &vals)?;
            }
        }
        Panel::new(self.dates.clone(), cats, data)
    }

    /// Boolean membership mask for a named sector: shape matches `self`
    /// (dates × symbols). Cell is `1.0` when `industry[symbol]` **exactly**
    /// equals `name`, else `0.0`. Symbols missing from the map are `0.0` (false)
    /// so `mask(signal, in_sector(...))` drops them rather than propagating NaN.
    /// Sector matching is case-sensitive.
    pub fn in_sector(&self, industry: &HashMap<String, String>, name: &str) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut data = Array2::from_elem((nrows, ncols), 0.0);
        for c in 0..ncols {
            let hit = industry
                .get(&self.symbols[c])
                .map(|s| s.as_str() == name)
                .unwrap_or(false);
            let v = bool_to_f64(hit);
            for r in 0..nrows {
                data[[r, c]] = v;
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }
}

fn aggregate(agg: &str, vals: &[f64]) -> Result<f64, crate::error::EngineError> {
    let n = vals.len() as f64;
    Ok(match agg {
        "sum" => vals.iter().sum(),
        "mean" => vals.iter().sum::<f64>() / n,
        "std" => {
            if vals.len() < 2 {
                return Ok(f64::NAN); // pandas std of one value is NaN (ddof=1)
            }
            let mean = vals.iter().sum::<f64>() / n;
            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
            var.sqrt()
        }
        other => {
            return Err(crate::error::EngineError::BadGroupbyAgg {
                agg: other.to_string(),
            });
        }
    })
}

#[cfg(test)]
mod cap_industry_tests {
    use super::*;
    use ndarray::array;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(s, i)| (s.to_string(), i.to_string()))
            .collect()
    }

    #[test]
    fn scales_only_over_cap_groups_and_preserves_nan() {
        // Tech {AAA,BBB} gross 0.4 > 0.3 -> ×0.75; Energy {CCC} single valid
        // (DDD is NaN) gross 0.1 <= 0.3 -> untouched; NaN stays NaN.
        let p = Panel::new(
            vec![20240102],
            vec!["AAA".into(), "BBB".into(), "CCC".into(), "DDD".into()],
            array![[0.2, 0.2, 0.1, f64::NAN]],
        )
        .unwrap();
        let ind = map(&[
            ("AAA", "Tech"),
            ("BBB", "Tech"),
            ("CCC", "Energy"),
            ("DDD", "Energy"),
        ]);
        let got = p.cap_industry(&ind, 0.3);
        assert!((got.data[[0, 0]] - 0.15).abs() < 1e-12);
        assert!((got.data[[0, 1]] - 0.15).abs() < 1e-12);
        assert!((got.data[[0, 2]] - 0.1).abs() < 1e-12); // Energy under cap
        assert!(got.data[[0, 3]].is_nan()); // NaN preserved
    }

    #[test]
    fn preserves_signs_for_long_short_groups() {
        // Tech gross |0.3|+|-0.3| = 0.6 > 0.3 -> ×0.5, signs kept.
        let p = Panel::new(
            vec![20240102],
            vec!["AAA".into(), "BBB".into()],
            array![[0.3, -0.3]],
        )
        .unwrap();
        let ind = map(&[("AAA", "Tech"), ("BBB", "Tech")]);
        let got = p.cap_industry(&ind, 0.3);
        assert!((got.data[[0, 0]] - 0.15).abs() < 1e-12);
        assert!((got.data[[0, 1]] + 0.15).abs() < 1e-12);
    }

    #[test]
    fn empty_industry_map_caps_the_whole_book() {
        // No map -> every symbol shares the "其他" bucket -> one group whose
        // gross (0.5) is capped at 0.3 (×0.6).
        let p = Panel::new(
            vec![20240102],
            vec!["AAA".into(), "BBB".into()],
            array![[0.25, 0.25]],
        )
        .unwrap();
        let got = p.cap_industry(&HashMap::new(), 0.3);
        assert!((got.data[[0, 0]] - 0.15).abs() < 1e-12);
        assert!((got.data[[0, 1]] - 0.15).abs() < 1e-12);
    }

    #[test]
    fn non_positive_cap_is_a_noop() {
        let p = Panel::new(
            vec![20240102],
            vec!["AAA".into(), "BBB".into()],
            array![[0.4, 0.4]],
        )
        .unwrap();
        let ind = map(&[("AAA", "Tech"), ("BBB", "Tech")]);
        for cap in [0.0, -1.0, f64::NAN] {
            let got = p.cap_industry(&ind, cap);
            assert_eq!(got.data[[0, 0]], 0.4);
            assert_eq!(got.data[[0, 1]], 0.4);
        }
    }
}

#[cfg(test)]
mod in_sector_tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn exact_match_false_for_missing_and_case() {
        let p = Panel::new(
            vec![20240102, 20240103],
            vec!["AAPL".into(), "XOM".into(), "ZZZ".into()],
            array![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        )
        .unwrap();
        let mut industry = HashMap::new();
        industry.insert("AAPL".into(), "Technology".into());
        industry.insert("XOM".into(), "Energy".into());
        // ZZZ missing from map

        let m = p.in_sector(&industry, "Technology");
        assert_eq!(m.data[[0, 0]], 1.0);
        assert_eq!(m.data[[1, 0]], 1.0); // constant over dates
        assert_eq!(m.data[[0, 1]], 0.0);
        assert_eq!(m.data[[0, 2]], 0.0); // missing → false
                                         // case-sensitive
        let m2 = p.in_sector(&industry, "technology");
        assert_eq!(m2.data[[0, 0]], 0.0);
    }
}
