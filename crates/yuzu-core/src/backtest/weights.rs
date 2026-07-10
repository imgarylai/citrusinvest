//! Per-row weight normalization and caps.

pub(crate) fn normalize_weights_row(row: &mut [f64]) {
    let total = row.iter().map(|w| w.abs()).sum::<f64>().max(1.0);
    for w in row.iter_mut() {
        *w /= total;
    }
}

/// Clamp each position's weight to `±limit` (sign-preserving), leaving the
/// residual in cash (a per-position weight cap). `limit <= 0` disables.
pub(crate) fn cap_weights_row(row: &mut [f64], limit: f64) {
    if limit <= 0.0 {
        return;
    }
    for w in row.iter_mut() {
        *w = w.clamp(-limit, limit);
    }
}

/// Cap each weight by the symbol's share of tradable dollar volume:
/// `|w[c]| <= max_participation * dollar_vol[c] / initial_capital` (sign-
/// preserving; residual stays in cash). A NaN dollar volume (missing volume or
/// price data) leaves the weight unchanged — data gaps aren't liquidity.
pub(crate) fn cap_weights_by_liquidity(
    row: &mut [f64],
    dollar_vol: &[f64],
    max_participation: f64,
    initial_capital: f64,
) {
    if max_participation <= 0.0 || initial_capital <= 0.0 {
        return;
    }
    for (w, dv) in row.iter_mut().zip(dollar_vol) {
        if dv.is_nan() {
            continue;
        }
        let cap = max_participation * dv / initial_capital;
        *w = w.clamp(-cap, cap);
    }
}
