//! Vendor-specific rating mapper. The cross-vendor scoring math
//! (`percentile_rank` / `pe_industry_pctile` / `analyst_upside_pct`) lives in
//! [`pomelo_data::factors`]; only Alpha Vantage's own rating-count adaptation is
//! here.

/// Map Alpha Vantage `AnalystRating*` **count** buckets onto citrusquant
/// `consensus_rating` (**lower = more bullish**, 1…5): weighted mean of
/// StrongBuy=1 … StrongSell=5.
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
    fn consensus_buckets() {
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
    }
}
