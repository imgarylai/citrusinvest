//! Shared numeric kernels used by cross-section ops, research, and metrics.
//!
//! Conventions (documented so call sites cannot silently disagree):
//!
//! - **argsort / average ranks**: ascending order; ties share the mean of their
//!   1-based ranks (pandas `"average"`). Argsort breaks residual ties by
//!   original index (stable). Sorting uses [`f64::total_cmp`] so NaN/Inf never
//!   panic; callers that need finite-only ranking should filter first.
//! - **mean_std**: finite entries only (`is_finite`); empty → `(NaN, NaN)`.
//!   `ddof = 0` is population (`/ n`); `ddof = 1` is sample (`/ (n − 1)`), with
//!   std `NaN` when `n < 2`.
//! - **sorted_quantile**: linear interpolation on a **pre-sorted** non-empty
//!   slice (pandas default); `q` is clamped to `[0, 1]`.

use std::cmp::Ordering;

/// Total order on `f64` for sorting; never panics on NaN/Inf.
#[inline]
pub(crate) fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

/// Sort `xs` ascending with a total order (NaN-safe).
#[inline]
pub(crate) fn sort_f64s(xs: &mut [f64]) {
    xs.sort_by(|a, b| a.total_cmp(b));
}

/// Indices that would sort `xs` ascending; ties broken by original index.
/// Uses [`f64::total_cmp`] so NaN never panics the sort.
#[inline]
pub(crate) fn argsort_stable(xs: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|&a, &b| cmp_f64(xs[a], xs[b]).then(a.cmp(&b)));
    idx
}

/// Average (fractional) 1-based ranks of `xs`. Ties share the mean rank.
/// Callers should pass finite values only (NaN/Inf ranking is undefined here).
pub(crate) fn average_ranks(xs: &[f64]) -> Vec<f64> {
    let order = argsort_stable(xs);
    let n = xs.len();
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && xs[order[j]] == xs[order[i]] {
            j += 1;
        }
        // ranks i..j (0-based positions in sorted order) share the average of
        // (i+1..=j) 1-based ranks.
        let avg = ((i + 1 + j) as f64) / 2.0;
        for &o in &order[i..j] {
            ranks[o] = avg;
        }
        i = j;
    }
    ranks
}

/// Mean and standard deviation of finite entries.
///
/// - `ddof = 0` — population: variance `/ n` (z-score, rolling_std convention)
/// - `ddof = 1` — sample: variance `/ (n − 1)`; std is `NaN` when `n < 2`
///   (metrics / IC std)
///
/// Empty input → `(NaN, NaN)`. Non-finite inputs are skipped.
pub(crate) fn mean_std(xs: &[f64], ddof: usize) -> (f64, f64) {
    let v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    let n = v.len();
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    let nf = n as f64;
    let mean = v.iter().sum::<f64>() / nf;
    if n <= ddof {
        return (mean, f64::NAN);
    }
    let denom = (n - ddof) as f64;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / denom;
    (mean, var.sqrt())
}

/// Linear-interpolation quantile of a **sorted** non-empty slice (pandas default).
/// `q` is clamped to `[0, 1]`. Panics if `sorted` is empty.
pub(crate) fn sorted_quantile(sorted: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    let pos = q.clamp(0.0, 1.0) * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argsort_stable_breaks_ties_by_index() {
        let xs = [3.0, 1.0, 1.0, 2.0];
        assert_eq!(argsort_stable(&xs), vec![1, 2, 3, 0]);
    }

    #[test]
    fn argsort_stable_and_sort_f64s_tolerate_nan() {
        let xs = [3.0, f64::NAN, 1.0];
        // Must not panic; finite values keep ascending order among themselves.
        let order = argsort_stable(&xs);
        assert_eq!(order.len(), 3);
        assert_eq!(xs[order[0]], 1.0);
        assert_eq!(xs[order[1]], 3.0);
        assert!(xs[order[2]].is_nan());

        let mut s = [2.0, f64::NAN, 0.0, f64::INFINITY];
        sort_f64s(&mut s);
        assert_eq!(s[0], 0.0);
        assert_eq!(s[1], 2.0);
        assert!(s[2].is_infinite() && s[2] > 0.0);
        assert!(s[3].is_nan());
    }

    #[test]
    fn average_ranks_ties_share_mean() {
        // values 1, 2, 2, 4 → ranks 1, 2.5, 2.5, 4
        let r = average_ranks(&[1.0, 2.0, 2.0, 4.0]);
        assert_eq!(r, vec![1.0, 2.5, 2.5, 4.0]);
    }

    #[test]
    fn mean_std_sample_and_population() {
        let xs = [1.0, 2.0, 3.0];
        let (m0, s0) = mean_std(&xs, 0);
        let (m1, s1) = mean_std(&xs, 1);
        assert!((m0 - 2.0).abs() < 1e-12);
        assert!((m1 - 2.0).abs() < 1e-12);
        // pop var = 2/3, sample var = 1
        assert!((s0 - (2.0_f64 / 3.0).sqrt()).abs() < 1e-12);
        assert!((s1 - 1.0).abs() < 1e-12);
        let (m, s) = mean_std(&[5.0], 1);
        assert_eq!(m, 5.0);
        assert!(s.is_nan());
    }

    #[test]
    fn sorted_quantile_endpoints_and_mid() {
        let s = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(sorted_quantile(&s, 0.0), 1.0);
        assert_eq!(sorted_quantile(&s, 1.0), 4.0);
        assert_eq!(sorted_quantile(&s, 0.5), 2.5);
    }
}
