//! Pure snapshot-factor scoring helpers (no I/O), shared by every `pomelo-*`
//! vendor sync crate so a `pe_industry_pctile` / `analyst_upside_pct` panel means
//! the same thing regardless of which vendor produced it (conventions:
//! citrusquant issue #211).
//!
//! These operate on values a vendor has **already extracted** — the
//! vendor-specific field maps and the rating-count/string mappers stay in each
//! vendor crate. Only the cross-vendor scoring math lives here.

/// Midrank percentile of `v` within `cohort` in `[0, 1]`.
pub fn percentile_rank(cohort: &[f64], v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    let mut n = 0usize;
    let mut below = 0usize;
    let mut equal = 0usize;
    for &c in cohort {
        if !c.is_finite() {
            continue;
        }
        n += 1;
        if c < v {
            below += 1;
        } else if c == v {
            equal += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    (below as f64 + 0.5 * equal as f64) / n as f64
}

/// Minimum cohort size for a meaningful industry percentile.
pub const MIN_COHORT: usize = 5;

/// P/E industry percentile in `[0, 100]`; `None` for a non-positive P/E or a
/// cohort thinner than [`MIN_COHORT`].
pub fn pe_industry_pctile(pe: f64, cohort: &[f64]) -> Option<f64> {
    if !pe.is_finite() || pe <= 0.0 {
        return None;
    }
    let finite = cohort.iter().filter(|c| c.is_finite()).count();
    if finite < MIN_COHORT {
        return None;
    }
    Some(percentile_rank(cohort, pe) * 100.0)
}

/// `(target − close) / close × 100`; `None` when `close <= 0` or inputs aren't finite.
pub fn analyst_upside_pct(target: f64, close: f64) -> Option<f64> {
    if !target.is_finite() || !close.is_finite() || close <= 0.0 {
        return None;
    }
    Some((target - close) / close * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_rank_midrank_and_edges() {
        let cohort = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert!((percentile_rank(&cohort, 30.0) - 0.5).abs() < 1e-9);
        assert_eq!(percentile_rank(&[], 1.0), 0.0);
        assert_eq!(percentile_rank(&cohort, f64::NAN), 0.0);
    }

    #[test]
    fn pe_pctile_scale_and_thin_cohort() {
        let cohort = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(pe_industry_pctile(30.0, &cohort), Some(50.0));
        assert_eq!(pe_industry_pctile(30.0, &[10.0, 20.0]), None);
        assert_eq!(pe_industry_pctile(-1.0, &cohort), None);
    }

    #[test]
    fn upside() {
        assert_eq!(analyst_upside_pct(120.0, 100.0), Some(20.0));
        assert!(analyst_upside_pct(1.0, 0.0).is_none());
        assert!(analyst_upside_pct(f64::NAN, 100.0).is_none());
    }
}
