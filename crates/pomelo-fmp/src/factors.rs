//! Vendor-specific rating mapper. The cross-vendor scoring math
//! (`percentile_rank` / `pe_industry_pctile` / `analyst_upside_pct`) lives in
//! [`pomelo_data::factors`]; only FMP's own rating-string adaptation is here.

/// Map an FMP consensus **string** onto citrusquant `consensus_rating`
/// (**lower = more bullish**, 1…5): Strong Buy=1 … Strong Sell=5. Unknown text
/// yields `None`.
pub fn consensus_to_rating(consensus: &str) -> Option<f64> {
    match consensus.trim().to_ascii_lowercase().as_str() {
        "strong buy" => Some(1.0),
        "buy" => Some(2.0),
        "hold" => Some(3.0),
        "sell" => Some(4.0),
        "strong sell" => Some(5.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_strings() {
        assert_eq!(consensus_to_rating("Strong Buy"), Some(1.0));
        assert_eq!(consensus_to_rating(" hold "), Some(3.0));
        assert_eq!(consensus_to_rating("Strong Sell"), Some(5.0));
        assert_eq!(consensus_to_rating("n/a"), None);
    }
}
