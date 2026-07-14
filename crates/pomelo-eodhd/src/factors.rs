//! Vendor-specific rating mapper. The cross-vendor scoring math
//! (`percentile_rank` / `pe_industry_pctile` / `analyst_upside_pct`) lives in
//! [`pomelo_data::factors`]; only EODHD's own rating adaptation is here.

/// Map an EODHD analyst rating onto citrusquant `consensus_rating`
/// (**lower = more bullish**, 1…5). EODHD's scale runs opposite to ours, so flip
/// it: `6 − rating`, clamped to `[1, 5]`.
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
    fn rating_flip() {
        assert_eq!(eodhd_rating_to_consensus(1.0), Some(5.0));
        assert_eq!(eodhd_rating_to_consensus(5.0), Some(1.0));
        assert_eq!(eodhd_rating_to_consensus(3.0), Some(3.0));
        assert!(eodhd_rating_to_consensus(f64::NAN).is_none());
    }
}
