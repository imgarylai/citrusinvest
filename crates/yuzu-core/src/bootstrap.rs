//! Circular block bootstrap over daily returns: resample the return series in
//! blocks (preserving short-range autocorrelation), rebuild synthetic equity
//! curves, and report percentile confidence intervals for the headline
//! metrics. Deterministic — a built-in SplitMix64 PRNG with a fixed seed, no
//! external dependencies (WASM-safe).

use crate::metrics;

/// SplitMix64 — tiny deterministic PRNG. Good enough for resampling indices;
/// NOT cryptographic.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0). Modulo bias is negligible for n << 2^64.
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Percentile band from the bootstrap distribution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BootstrapCi {
    pub p05: f64,
    pub p50: f64,
    pub p95: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BootstrapSummary {
    pub n_samples: usize,
    pub block_len: usize,
    pub sharpe: BootstrapCi,
    pub cagr: BootstrapCi,
    pub max_drawdown: BootstrapCi,
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn ci(mut samples: Vec<f64>) -> BootstrapCi {
    samples.retain(|x| !x.is_nan());
    crate::ops::stat::sort_f64s(&mut samples);
    BootstrapCi {
        p05: percentile(&samples, 0.05),
        p50: percentile(&samples, 0.50),
        p95: percentile(&samples, 0.95),
    }
}

/// Run the bootstrap: `n_samples` synthetic equity curves resampled from the
/// daily returns of `equity` in circular blocks of `block_len` (`0` = auto,
/// `⌊√n⌋`). `dates` must be the equity curve's date axis (used for CAGR).
/// Returns `None` when `n_samples == 0` or the curve is too short.
pub fn bootstrap(
    dates: &[i32],
    equity: &[f64],
    n_samples: usize,
    block_len: usize,
    seed: u64,
) -> Option<BootstrapSummary> {
    if n_samples == 0 || equity.len() < 3 || dates.len() != equity.len() {
        return None;
    }
    // daily returns, skipping the base row
    let rets: Vec<f64> = (1..equity.len())
        .map(|i| equity[i] / equity[i - 1] - 1.0)
        .collect();
    let n = rets.len();
    let block = if block_len == 0 {
        ((n as f64).sqrt().floor() as usize).max(1)
    } else {
        block_len.max(1)
    };
    let mut rng = SplitMix64(seed);
    let mut sharpes = Vec::with_capacity(n_samples);
    let mut cagrs = Vec::with_capacity(n_samples);
    let mut mdds = Vec::with_capacity(n_samples);
    let mut eq = vec![0.0_f64; equity.len()];
    for _ in 0..n_samples {
        eq.clear();
        eq.push(1.0);
        while eq.len() < equity.len() {
            let start = rng.below(n);
            for j in 0..block {
                if eq.len() >= equity.len() {
                    break;
                }
                let r = rets[(start + j) % n]; // circular wrap
                // `eq` always starts with base 1.0 before this loop.
                let prev = eq[eq.len() - 1];
                eq.push(prev * (1.0 + r));
            }
        }
        sharpes.push(metrics::sharpe(&eq));
        cagrs.push(metrics::cagr(&eq, dates));
        mdds.push(metrics::max_drawdown(&eq));
    }
    Some(BootstrapSummary {
        n_samples,
        block_len: block,
        sharpe: ci(sharpes),
        cagr: ci(cagrs),
        max_drawdown: ci(mdds),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yyyymmdd(i: usize) -> i32 {
        let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Days::new(i as u64);
        use chrono::Datelike;
        d.year() * 10000 + d.month() as i32 * 100 + d.day() as i32
    }

    fn geometric_equity(n: usize, daily: f64) -> (Vec<i32>, Vec<f64>) {
        let mut eq = vec![1.0];
        let mut dates = vec![yyyymmdd(0)];
        for i in 1..n {
            let prev = *eq.last().unwrap();
            eq.push(prev * (1.0 + daily));
            dates.push(yyyymmdd(i));
        }
        (dates, eq)
    }

    #[test]
    fn deterministic_and_gated() {
        let (dates, eq) = geometric_equity(300, 0.001);
        assert!(bootstrap(&dates, &eq, 0, 0, 42).is_none()); // off
        assert!(bootstrap(&dates, &eq[..2], 100, 0, 42).is_none()); // too short
        let a = bootstrap(&dates, &eq, 200, 0, 42).unwrap();
        let b = bootstrap(&dates, &eq, 200, 0, 42).unwrap();
        assert_eq!(a.sharpe.p50, b.sharpe.p50); // same seed -> same answer
        assert_eq!(a.block_len, 17); // auto = floor(sqrt(299))
    }

    #[test]
    fn constant_returns_collapse_the_band() {
        // Every resample of a constant-return series is the same curve, so the
        // CI collapses to a point and max drawdown is 0.
        let (dates, eq) = geometric_equity(100, 0.001);
        let s = bootstrap(&dates, &eq, 50, 5, 7).unwrap();
        assert!((s.cagr.p05 - s.cagr.p95).abs() < 1e-12);
        assert!(s.max_drawdown.p50.abs() < 1e-12);
        assert!(s.sharpe.p50 > 0.0);
    }

    #[test]
    fn band_orders_and_brackets_reality() {
        // Alternating +2%/-1% has positive drift and real variance: the band
        // must be ordered p05 <= p50 <= p95 and strictly wider than a point.
        let mut eq = vec![1.0];
        let mut dates = vec![yyyymmdd(0)];
        for i in 1..260 {
            let r = if i % 2 == 0 { 0.02 } else { -0.01 };
            let prev = *eq.last().unwrap();
            eq.push(prev * (1.0 + r));
            dates.push(yyyymmdd(i));
        }
        let s = bootstrap(&dates, &eq, 300, 10, 1).unwrap();
        assert!(s.sharpe.p05 <= s.sharpe.p50 && s.sharpe.p50 <= s.sharpe.p95);
        assert!(s.max_drawdown.p05 <= s.max_drawdown.p95);
        assert!(s.sharpe.p95 - s.sharpe.p05 > 0.0);
    }
}
