//! Pure snapshot-factor helpers (no I/O). Same math as the FMP / AV crates so a
//! `consensus_rating` or `pe_industry_pctile` panel means the same thing across
//! vendors.

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

/// Map analyst **count** buckets onto citrusquant `consensus_rating`
/// (**lower = more bullish**, 1…5): weighted mean of StrongBuy=1 … StrongSell=5.
pub fn consensus_from_rating_counts(
    strong_buy: f64,
    buy: f64,
    hold: f64,
    sell: f64,
    strong_sell: f64,
) -> Option<f64> {
    let counts = [strong_buy, buy, hold, sell, strong_sell];
    if counts.iter().any(|c| !c.is_finite() || *c < 0.0) {
        return None;
    }
    let total: f64 = counts.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let score =
        (1.0 * strong_buy + 2.0 * buy + 3.0 * hold + 4.0 * sell + 5.0 * strong_sell) / total;
    Some(score.clamp(1.0, 5.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upside_and_consensus() {
        assert_eq!(analyst_upside_pct(120.0, 100.0), Some(20.0));
        assert_eq!(
            consensus_from_rating_counts(10.0, 0.0, 0.0, 0.0, 0.0),
            Some(1.0)
        );
        assert_eq!(
            consensus_from_rating_counts(0.0, 0.0, 0.0, 0.0, 5.0),
            Some(5.0)
        );
        assert!(
            (consensus_from_rating_counts(1.0, 1.0, 1.0, 1.0, 1.0).unwrap() - 3.0).abs() < 1e-9
        );
        assert!(consensus_from_rating_counts(0.0, 0.0, 0.0, 0.0, 0.0).is_none());
        assert!(consensus_from_rating_counts(-1.0, 0.0, 0.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn pe_pctile() {
        let cohort = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(pe_industry_pctile(30.0, &cohort), Some(50.0));
        assert_eq!(pe_industry_pctile(30.0, &[10.0, 20.0]), None);
        assert_eq!(pe_industry_pctile(-1.0, &cohort), None);
        assert!(analyst_upside_pct(1.0, 0.0).is_none());
    }

    #[test]
    fn percentile_rank_edges() {
        assert_eq!(percentile_rank(&[], 1.0), 0.0);
        assert_eq!(percentile_rank(&[1.0, 2.0, 3.0], f64::NAN), 0.0);
    }
}
