//! Pure snapshot-factor helpers (no I/O).

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

pub const MIN_COHORT: usize = 5;

/// P/E industry percentile in `[0, 100]`.
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

/// `(target − close) / close × 100`.
pub fn analyst_upside_pct(target: f64, close: f64) -> Option<f64> {
    if !target.is_finite() || !close.is_finite() || close <= 0.0 {
        return None;
    }
    Some((target - close) / close * 100.0)
}

/// Map EODHD `AnalystRatings.Rating` (typically ~1–5, **higher ≈ more bullish**
/// given StrongBuy-heavy averages) onto citrusquant `consensus_rating`
/// (**lower = more bullish**, 1…5) as `6 − rating`, clamped.
pub fn eodhd_rating_to_consensus(rating: f64) -> Option<f64> {
    if !rating.is_finite() {
        return None;
    }
    let v = (6.0 - rating).clamp(1.0, 5.0);
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upside_and_rating() {
        assert_eq!(analyst_upside_pct(120.0, 100.0), Some(20.0));
        assert_eq!(eodhd_rating_to_consensus(5.0), Some(1.0));
        assert_eq!(eodhd_rating_to_consensus(1.0), Some(5.0));
        assert_eq!(eodhd_rating_to_consensus(4.0), Some(2.0));
    }

    #[test]
    fn pe_pctile() {
        let cohort = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(pe_industry_pctile(30.0, &cohort), Some(50.0));
        assert_eq!(pe_industry_pctile(30.0, &[10.0, 20.0]), None);
    }
}
