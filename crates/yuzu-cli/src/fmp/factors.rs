//! Pure, I/O-free snapshot-factor math — the native port of the web app's
//! `factor-snapshot-panels.ts` transforms. Given already-fetched FMP numbers,
//! these functions compute the six `FACTOR_PANEL_FIELDS` values
//! (`piotroski_score`, `altman_z`, `fcf_yield`, `pe_industry_pctile`,
//! `analyst_upside_pct`, `consensus_rating`).
//!
//! Deliberately free of any FMP / HTTP / filesystem dependency so it can later
//! be lifted into a shared `yuzu-factors` crate and compiled to wasm (web) /
//! PyO3 (Python) as the single source of truth for factor formulas. Keep it
//! pure: numbers in, numbers out, no `Fetcher`, no `serde_json`.

/// Midrank percentile of `v` within `cohort`: the fraction of cohort values
/// strictly below `v`, plus half of those equal to `v`, in `[0, 1]`.
///
/// Non-finite cohort entries are ignored; a `v` that is non-finite, or an empty
/// finite cohort, returns `0.0`. Mirrors `percentileRank` in the web app's
/// `lib/quant/percentile.ts` (e.g. `percentile_rank([10,20,30,40], 30) == 0.625`).
pub(crate) fn percentile_rank(cohort: &[f64], v: f64) -> f64 {
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

/// `pe_industry_pctile` = midrank percentile of `pe` within its industry P/E
/// `cohort`, scaled to `[0, 100]`. Returns `None` (→ NaN panel cell) when the
/// P/E is non-positive/non-finite or the finite cohort has fewer than
/// [`MIN_COHORT`] members — matching the web builder's thin-cohort suppression.
pub(crate) fn pe_industry_pctile(pe: f64, cohort: &[f64]) -> Option<f64> {
    if !pe.is_finite() || pe <= 0.0 {
        return None;
    }
    let finite = cohort.iter().filter(|c| c.is_finite()).count();
    if finite < MIN_COHORT {
        return None;
    }
    Some(percentile_rank(cohort, pe) * 100.0)
}

/// Minimum finite cohort size below which `pe_industry_pctile` is suppressed.
pub(crate) const MIN_COHORT: usize = 5;

/// `analyst_upside_pct` = `(target − close) / close × 100`. Returns `None` when
/// the close price is non-positive or either input is non-finite.
pub(crate) fn analyst_upside_pct(target: f64, close: f64) -> Option<f64> {
    if !target.is_finite() || !close.is_finite() || close <= 0.0 {
        return None;
    }
    Some((target - close) / close * 100.0)
}

/// Map an FMP `grades-summary` consensus label to the numeric `consensus_rating`
/// (lower = more bullish): Strong Buy = 1 … Strong Sell = 5. Case- and
/// whitespace-insensitive; unrecognised labels → `None`. Mirrors
/// `consensusToRating` in the web app's `db/queries/factor-snapshots.ts`.
pub(crate) fn consensus_to_rating(consensus: &str) -> Option<f64> {
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
    fn percentile_rank_midrank_matches_reference() {
        // The documented example from the TS source.
        assert!((percentile_rank(&[10.0, 20.0, 30.0, 40.0], 30.0) - 0.625).abs() < 1e-12);
        // Below everything / above everything.
        assert_eq!(percentile_rank(&[10.0, 20.0, 30.0], 5.0), 0.0);
        assert_eq!(percentile_rank(&[10.0, 20.0, 30.0], 99.0), 1.0);
        // A single equal value → 0.5 (0 below + half of one equal).
        assert_eq!(percentile_rank(&[42.0], 42.0), 0.5);
    }

    #[test]
    fn percentile_rank_ignores_non_finite_and_handles_empty() {
        // NaNs in the cohort are dropped from the denominator.
        assert_eq!(percentile_rank(&[f64::NAN, 10.0, 30.0], 20.0), 0.5);
        // Non-finite value, or an all-NaN / empty cohort → 0.
        assert_eq!(percentile_rank(&[1.0, 2.0], f64::NAN), 0.0);
        assert_eq!(percentile_rank(&[f64::NAN, f64::INFINITY], 1.0), 0.0);
        assert_eq!(percentile_rank(&[], 1.0), 0.0);
    }

    #[test]
    fn pe_industry_pctile_scales_and_suppresses() {
        // 5-member cohort (== MIN_COHORT); 30 sits at midrank 2.5/5 = 0.5 → 50.0.
        let cohort = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(pe_industry_pctile(30.0, &cohort), Some(50.0));
        // Non-positive / non-finite P/E → suppressed.
        assert_eq!(pe_industry_pctile(0.0, &cohort), None);
        assert_eq!(pe_industry_pctile(-5.0, &cohort), None);
        assert_eq!(pe_industry_pctile(f64::NAN, &cohort), None);
        // Thin cohort (< 5 finite) → suppressed even for a valid P/E.
        assert_eq!(pe_industry_pctile(30.0, &[10.0, 20.0, 30.0, 40.0]), None);
        assert_eq!(
            pe_industry_pctile(30.0, &[10.0, 20.0, 30.0, 40.0, f64::NAN]),
            None
        );
    }

    #[test]
    fn analyst_upside_pct_computes_and_guards() {
        assert_eq!(analyst_upside_pct(120.0, 100.0), Some(20.0));
        assert_eq!(analyst_upside_pct(80.0, 100.0), Some(-20.0));
        // Guard rails: non-positive close, non-finite inputs.
        assert_eq!(analyst_upside_pct(120.0, 0.0), None);
        assert_eq!(analyst_upside_pct(120.0, -1.0), None);
        assert_eq!(analyst_upside_pct(f64::NAN, 100.0), None);
    }

    #[test]
    fn consensus_to_rating_maps_labels() {
        assert_eq!(consensus_to_rating("Strong Buy"), Some(1.0));
        assert_eq!(consensus_to_rating("  buy "), Some(2.0)); // trims + case-insensitive
        assert_eq!(consensus_to_rating("HOLD"), Some(3.0));
        assert_eq!(consensus_to_rating("Sell"), Some(4.0));
        assert_eq!(consensus_to_rating("Strong Sell"), Some(5.0));
        assert_eq!(consensus_to_rating("Overweight"), None); // unrecognised
        assert_eq!(consensus_to_rating(""), None);
    }
}
