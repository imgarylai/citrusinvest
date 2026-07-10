//! Standard performance metrics (CAGR, Sharpe, Sortino, max drawdown, etc.). All
//! functions operate on a daily equity curve (`&[f64]`, base 1.0) or a daily
//! returns slice. Conventions: annualization 252, rf = 0, std ddof = 1.

use chrono::{Datelike, NaiveDate};

pub fn to_returns(equity: &[f64]) -> Vec<f64> {
    let mut out = vec![f64::NAN; equity.len()];
    for i in 1..equity.len() {
        out[i] = equity[i] / equity[i - 1] - 1.0;
    }
    out
}

pub fn total_return(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return f64::NAN;
    }
    equity[equity.len() - 1] / equity[0] - 1.0
}

pub fn drawdown_series(equity: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; equity.len()];
    let mut peak = f64::NEG_INFINITY;
    for (i, &e) in equity.iter().enumerate() {
        if e > peak {
            peak = e;
        }
        out[i] = e / peak - 1.0;
    }
    out
}

pub fn max_drawdown(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return f64::NAN;
    }
    drawdown_series(equity).into_iter().fold(0.0, f64::min)
}

fn to_naive(yyyymmdd: i32) -> NaiveDate {
    let y = yyyymmdd / 10000;
    let m = (yyyymmdd / 100 % 100) as u32;
    let d = (yyyymmdd % 100) as u32;
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

pub fn year_frac(start: i32, end: i32) -> f64 {
    let secs = (to_naive(end) - to_naive(start)).num_seconds() as f64;
    secs / 31_557_600.0
}

/// sample mean + std (ddof=1) over the non-NaN entries.
fn mean_std(xs: &[f64]) -> (f64, f64) {
    let v: Vec<f64> = xs.iter().copied().filter(|x| !x.is_nan()).collect();
    let n = v.len() as f64;
    if n == 0.0 {
        return (f64::NAN, f64::NAN);
    }
    let mean = v.iter().sum::<f64>() / n;
    if n < 2.0 {
        return (mean, f64::NAN);
    }
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, var.sqrt())
}

pub fn cagr(equity: &[f64], dates: &[i32]) -> f64 {
    if equity.len() < 2 || dates.len() < 2 {
        return f64::NAN;
    }
    let yf = year_frac(dates[0], dates[dates.len() - 1]);
    (equity[equity.len() - 1] / equity[0]).powf(1.0 / yf) - 1.0
}

pub fn ann_volatility(equity: &[f64]) -> f64 {
    let (_, std) = mean_std(&to_returns(equity));
    std * 252.0_f64.sqrt()
}

pub fn sharpe(equity: &[f64]) -> f64 {
    let r = to_returns(equity);
    let (mean, std) = mean_std(&r);
    (mean / std.max(1e-6)) * 252.0_f64.sqrt()
}

pub fn sortino(equity: &[f64]) -> f64 {
    let r = to_returns(equity);
    // ffn: er = returns (rf=0); negative_returns = min(er[1:], 0).
    let (mean, _) = mean_std(&r);
    let downside: Vec<f64> = r
        .iter()
        .skip(1)
        .map(|&x| if x < 0.0 { x } else { 0.0 })
        .collect();
    let (_, dstd) = mean_std(&downside);
    if dstd <= 0.0 || dstd.is_nan() {
        return f64::NAN;
    }
    (mean / dstd) * 252.0_f64.sqrt()
}

pub fn calmar(equity: &[f64], dates: &[i32]) -> f64 {
    let c = cagr(equity, dates);
    // Short/empty input: cagr already returns NaN ("not enough data"); keep that,
    // don't let the zero-drawdown guard below reinterpret it as "no drawdown".
    if c.is_nan() {
        return f64::NAN;
    }
    let mdd = max_drawdown(equity).abs();
    if mdd == 0.0 {
        return f64::INFINITY;
    }
    c / mdd
}

pub fn recovery_factor(equity: &[f64]) -> f64 {
    let mdd = max_drawdown(equity).abs();
    if mdd == 0.0 {
        return f64::INFINITY;
    }
    total_return(equity) / mdd
}

/// Longest run of consecutive rows strictly below the running peak (drawdown < 0),
/// counted in trading-day rows.
pub fn max_drawdown_duration(equity: &[f64]) -> f64 {
    let (mut max, mut cur) = (0u32, 0u32);
    for d in drawdown_series(equity) {
        if d < 0.0 {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    max as f64
}

use crate::backtest::Trade;

fn closed(trades: &[Trade]) -> Vec<&Trade> {
    trades.iter().filter(|t| t.exit_date.is_some()).collect()
}

pub fn win_rate(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    c.iter().filter(|t| t.ret > 0.0).count() as f64 / c.len() as f64
}

pub fn profit_factor(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    let gains: f64 = c.iter().filter(|t| t.ret > 0.0).map(|t| t.ret).sum();
    let losses: f64 = c.iter().filter(|t| t.ret < 0.0).map(|t| t.ret).sum();
    if losses == 0.0 {
        return f64::INFINITY;
    }
    gains / losses.abs()
}

pub fn expectancy(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    c.iter().map(|t| t.ret).sum::<f64>() / c.len() as f64
}

pub fn avg_holding_period(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    c.iter().map(|t| t.period as f64).sum::<f64>() / c.len() as f64
}

pub fn num_trades(trades: &[Trade]) -> f64 {
    closed(trades).len() as f64
}

pub fn avg_win(trades: &[Trade]) -> f64 {
    let w: Vec<f64> = closed(trades)
        .iter()
        .map(|t| t.ret)
        .filter(|&r| r > 0.0)
        .collect();
    if w.is_empty() {
        return f64::NAN;
    }
    w.iter().sum::<f64>() / w.len() as f64
}

pub fn avg_loss(trades: &[Trade]) -> f64 {
    let l: Vec<f64> = closed(trades)
        .iter()
        .map(|t| t.ret)
        .filter(|&r| r < 0.0)
        .collect();
    if l.is_empty() {
        return f64::NAN;
    }
    l.iter().sum::<f64>() / l.len() as f64
}

pub fn payoff_ratio(trades: &[Trade]) -> f64 {
    let (aw, al) = (avg_win(trades), avg_loss(trades));
    if aw.is_nan() || al.is_nan() {
        return f64::NAN;
    }
    aw / al.abs()
}

pub fn best_trade(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    c.iter().map(|t| t.ret).fold(f64::NEG_INFINITY, f64::max)
}

pub fn worst_trade(trades: &[Trade]) -> f64 {
    let c = closed(trades);
    if c.is_empty() {
        return f64::NAN;
    }
    c.iter().map(|t| t.ret).fold(f64::INFINITY, f64::min)
}

pub fn max_consecutive_losses(trades: &[Trade]) -> f64 {
    let mut c = closed(trades);
    c.sort_by_key(|t| t.exit_date); // closed -> Some; chronological
    let (mut max, mut cur) = (0u32, 0u32);
    for t in c {
        if t.ret < 0.0 {
            cur += 1;
            max = max.max(cur);
        } else {
            cur = 0;
        }
    }
    max as f64
}

pub fn time_in_market(exposure: &[f64]) -> f64 {
    if exposure.is_empty() {
        return f64::NAN;
    }
    exposure.iter().filter(|&&e| e > 0.0).count() as f64 / exposure.len() as f64
}

pub fn avg_exposure(exposure: &[f64]) -> f64 {
    if exposure.is_empty() {
        return f64::NAN;
    }
    exposure.iter().sum::<f64>() / exposure.len() as f64
}

// ---- calendar-period and rolling metrics -----------------------------------

/// One calendar bucket's return: `period` is `"2024-01"` (monthly) or `"2024"`
/// (yearly); `ret` is the equity return over the bucket, chained off the
/// previous bucket's closing equity (the first bucket chains off `equity[0]`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PeriodReturn {
    pub period: String,
    pub ret: f64,
}

fn period_returns(dates: &[i32], equity: &[f64], monthly: bool) -> Vec<PeriodReturn> {
    let key = |d: i32| if monthly { d / 100 } else { d / 10000 };
    let label = |k: i32| {
        if monthly {
            format!("{}-{:02}", k / 100, k % 100)
        } else {
            k.to_string()
        }
    };
    let mut out = Vec::new();
    if dates.is_empty() || equity.len() != dates.len() {
        return out;
    }
    let mut baseline = equity[0];
    let mut cur = key(dates[0]);
    for i in 0..dates.len() {
        let k = key(dates[i]);
        if k != cur {
            // row i-1 closed the previous bucket
            out.push(PeriodReturn {
                period: label(cur),
                ret: equity[i - 1] / baseline - 1.0,
            });
            baseline = equity[i - 1];
            cur = k;
        }
    }
    out.push(PeriodReturn {
        period: label(cur),
        ret: equity[equity.len() - 1] / baseline - 1.0,
    });
    out
}

pub fn monthly_returns(dates: &[i32], equity: &[f64]) -> Vec<PeriodReturn> {
    period_returns(dates, equity, true)
}

pub fn yearly_returns(dates: &[i32], equity: &[f64]) -> Vec<PeriodReturn> {
    period_returns(dates, equity, false)
}

/// Rolling annualized volatility over a `window` of daily returns; NaN until
/// `window` returns are available (row `window` onward, since row 0 has none).
pub fn rolling_volatility(equity: &[f64], window: usize) -> Vec<f64> {
    let r = to_returns(equity);
    let mut out = vec![f64::NAN; r.len()];
    for i in window..r.len() {
        let (_, std) = mean_std(&r[i + 1 - window..=i]);
        out[i] = std * 252.0_f64.sqrt();
    }
    out
}

/// Rolling annualized Sharpe (rf = 0) over a `window` of daily returns; NaN
/// until `window` returns are available.
pub fn rolling_sharpe(equity: &[f64], window: usize) -> Vec<f64> {
    let r = to_returns(equity);
    let mut out = vec![f64::NAN; r.len()];
    for i in window..r.len() {
        let (mean, std) = mean_std(&r[i + 1 - window..=i]);
        out[i] = (mean / std.max(1e-6)) * 252.0_f64.sqrt();
    }
    out
}

// ---- benchmark-relative metrics ------------------------------------------
// All take the strategy and benchmark equity curves (same length, aligned by
// row); daily-return pairs where either side is NaN are dropped.

/// Paired daily returns (strategy, benchmark), NaN pairs removed.
fn paired_returns(equity: &[f64], bench: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let r = to_returns(equity);
    let b = to_returns(bench);
    let mut rs = Vec::new();
    let mut bs = Vec::new();
    for i in 0..r.len().min(b.len()) {
        if !r[i].is_nan() && !b[i].is_nan() {
            rs.push(r[i]);
            bs.push(b[i]);
        }
    }
    (rs, bs)
}

/// CAPM beta: cov(r, b) / var(b) over paired daily returns (ddof = 1).
pub fn beta(equity: &[f64], bench: &[f64]) -> f64 {
    let (rs, bs) = paired_returns(equity, bench);
    let n = rs.len() as f64;
    if n < 2.0 {
        return f64::NAN;
    }
    let (rm, _) = mean_std(&rs);
    let (bm, bstd) = mean_std(&bs);
    let cov = rs
        .iter()
        .zip(&bs)
        .map(|(r, b)| (r - rm) * (b - bm))
        .sum::<f64>()
        / (n - 1.0);
    let var = bstd * bstd;
    if var == 0.0 {
        return f64::NAN;
    }
    cov / var
}

/// Annualized CAPM alpha (rf = 0): `(mean(r) - beta * mean(b)) * 252`.
pub fn alpha(equity: &[f64], bench: &[f64]) -> f64 {
    let (rs, bs) = paired_returns(equity, bench);
    let beta = beta(equity, bench);
    if beta.is_nan() {
        return f64::NAN;
    }
    let (rm, _) = mean_std(&rs);
    let (bm, _) = mean_std(&bs);
    (rm - beta * bm) * 252.0
}

/// Annualized tracking error: `std(r - b, ddof = 1) * sqrt(252)`.
pub fn tracking_error(equity: &[f64], bench: &[f64]) -> f64 {
    let (rs, bs) = paired_returns(equity, bench);
    let diff: Vec<f64> = rs.iter().zip(&bs).map(|(r, b)| r - b).collect();
    let (_, std) = mean_std(&diff);
    std * 252.0_f64.sqrt()
}

/// Information ratio: `mean(r - b) / std(r - b, ddof = 1) * sqrt(252)`.
pub fn information_ratio(equity: &[f64], bench: &[f64]) -> f64 {
    let (rs, bs) = paired_returns(equity, bench);
    let diff: Vec<f64> = rs.iter().zip(&bs).map(|(r, b)| r - b).collect();
    let (mean, std) = mean_std(&diff);
    (mean / std.max(1e-6)) * 252.0_f64.sqrt()
}

/// Benchmark total return over its first/last non-NaN observations.
pub fn benchmark_return(bench: &[f64]) -> f64 {
    let first = bench.iter().copied().find(|x| !x.is_nan());
    let last = bench.iter().rev().copied().find(|x| !x.is_nan());
    match (first, last) {
        (Some(f), Some(l)) if f != 0.0 => l / f - 1.0,
        _ => f64::NAN,
    }
}

// ---- distribution / tail metrics ------------------------------------------
// All operate on the non-NaN daily returns of the equity curve.

/// Non-NaN daily returns (drops the leading NaN that `to_returns` emits).
fn clean_returns(equity: &[f64]) -> Vec<f64> {
    to_returns(equity)
        .into_iter()
        .filter(|x| !x.is_nan())
        .collect()
}

/// Largest single-day return; NaN when there are no returns.
pub fn best_day(equity: &[f64]) -> f64 {
    let r = clean_returns(equity);
    if r.is_empty() {
        return f64::NAN;
    }
    r.into_iter().fold(f64::NEG_INFINITY, f64::max)
}

/// Smallest (most negative) single-day return; NaN when there are no returns.
pub fn worst_day(equity: &[f64]) -> f64 {
    let r = clean_returns(equity);
    if r.is_empty() {
        return f64::NAN;
    }
    r.into_iter().fold(f64::INFINITY, f64::min)
}

/// Population central moments `(n, mean, m2, m3, m4)` of `r` (divide by `n`).
fn central_moments(r: &[f64]) -> (usize, f64, f64, f64, f64) {
    let n = r.len();
    if n == 0 {
        return (0, f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    let nf = n as f64;
    let mean = r.iter().sum::<f64>() / nf;
    let (mut m2, mut m3, mut m4) = (0.0, 0.0, 0.0);
    for &x in r {
        let d = x - mean;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
    }
    (n, mean, m2 / nf, m3 / nf, m4 / nf)
}

/// Skewness of daily returns (population Fisher-Pearson `m3 / m2^1.5`); NaN for
/// fewer than two returns or a zero-variance (degenerate) distribution.
pub fn skew(equity: &[f64]) -> f64 {
    let (n, _, m2, m3, _) = central_moments(&clean_returns(equity));
    if n < 2 || m2 == 0.0 {
        return f64::NAN;
    }
    m3 / m2.powf(1.5)
}

/// Excess kurtosis of daily returns (population `m4 / m2^2 − 3`); NaN under the
/// same degenerate conditions as [`skew`].
pub fn kurtosis(equity: &[f64]) -> f64 {
    let (n, _, m2, _, m4) = central_moments(&clean_returns(equity));
    if n < 2 || m2 == 0.0 {
        return f64::NAN;
    }
    m4 / (m2 * m2) - 3.0
}

/// `q`-quantile (`0..=1`) of `xs` by linear interpolation between order
/// statistics (NumPy's default 'linear' method); NaN for empty input.
fn percentile(xs: &[f64], q: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if s.len() == 1 {
        return s[0];
    }
    let pos = q * (s.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
}

/// Historical Value-at-Risk at 95%: the 5th-percentile daily return. A loss
/// shows as a negative number. NaN when there are no returns.
pub fn var_95(equity: &[f64]) -> f64 {
    percentile(&clean_returns(equity), 0.05)
}

/// Conditional VaR / expected shortfall at 95%: mean of the daily returns at or
/// below [`var_95`]. NaN when there are no returns.
pub fn cvar_95(equity: &[f64]) -> f64 {
    let r = clean_returns(equity);
    if r.is_empty() {
        return f64::NAN;
    }
    let v = percentile(&r, 0.05);
    let tail: Vec<f64> = r.iter().copied().filter(|&x| x <= v).collect();
    if tail.is_empty() {
        return v;
    }
    tail.iter().sum::<f64>() / tail.len() as f64
}

/// Mean of the drawdown series (zeros at new highs included); a non-positive
/// fraction. NaN for an empty curve.
pub fn avg_drawdown(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return f64::NAN;
    }
    let dd = drawdown_series(equity);
    dd.iter().sum::<f64>() / dd.len() as f64
}

/// Ulcer index: root-mean-square drawdown over the curve, reported as a
/// fraction (consistent with [`max_drawdown`]; not scaled to percent). NaN for
/// an empty curve.
pub fn ulcer_index(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return f64::NAN;
    }
    let dd = drawdown_series(equity);
    (dd.iter().map(|d| d * d).sum::<f64>() / dd.len() as f64).sqrt()
}

// ---- lookback returns ------------------------------------------------------

/// Shift a `YYYYMMDD` date by `delta` years, clamping an invalid Feb-29 target
/// to Feb-28. Anchors the trailing-return windows.
fn shift_years(yyyymmdd: i32, delta: i32) -> i32 {
    let d = to_naive(yyyymmdd);
    let y = d.year() + delta;
    let shifted = d
        .with_year(y)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(y, d.month(), 28).unwrap());
    shifted.year() * 10000 + shifted.month() as i32 * 100 + shifted.day() as i32
}

/// Return from the last equity point on or before `target` (YYYYMMDD) to the
/// final point. `None` when the series starts after `target` (window not
/// covered) or the anchor equity is zero.
fn return_since(dates: &[i32], equity: &[f64], target: i32) -> Option<f64> {
    if dates.is_empty() || dates.len() != equity.len() {
        return None;
    }
    let base = dates.iter().rposition(|&d| d <= target)?;
    let last = equity.len() - 1;
    if equity[base] == 0.0 {
        return None;
    }
    Some(equity[last] / equity[base] - 1.0)
}

/// Year-to-date return: from the last close of the prior calendar year to the
/// final point. `None` when the backtest does not reach into a prior year.
pub fn ytd_return(dates: &[i32], equity: &[f64]) -> Option<f64> {
    let last = *dates.last()?;
    let prior_year_end = (last / 10000 - 1) * 10000 + 1231;
    return_since(dates, equity, prior_year_end)
}

/// Trailing return over the last `years` calendar years (e.g. `1`, `3`); `None`
/// when the backtest is shorter than the window.
pub fn trailing_return(dates: &[i32], equity: &[f64], years: i32) -> Option<f64> {
    let last = *dates.last()?;
    return_since(dates, equity, shift_years(last, -years))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_returns_bucket_by_month_and_year() {
        // Dec 2023 (2 rows) -> Jan 2024 (2 rows) -> Feb 2024 (1 row).
        let dates = [20231228, 20231229, 20240102, 20240131, 20240201];
        let eq = [1.0, 1.1, 1.1, 1.32, 1.32];
        let m = monthly_returns(&dates, &eq);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].period, "2023-12");
        assert!((m[0].ret - 0.1).abs() < 1e-12);
        assert_eq!(m[1].period, "2024-01");
        assert!((m[1].ret - 0.2).abs() < 1e-12); // 1.32/1.1 - 1
        assert_eq!(m[2].period, "2024-02");
        assert!(m[2].ret.abs() < 1e-12);
        let y = yearly_returns(&dates, &eq);
        assert_eq!(y.len(), 2);
        assert_eq!(y[0].period, "2023");
        assert!((y[0].ret - 0.1).abs() < 1e-12);
        assert_eq!(y[1].period, "2024");
        assert!((y[1].ret - 0.2).abs() < 1e-12);
    }

    #[test]
    fn rolling_metrics_warm_up_then_fill() {
        // Constant +1% daily: vol 0, sharpe huge; window 3 -> NaN rows 0..=2.
        let mut eq = vec![1.0];
        for _ in 0..5 {
            let prev = *eq.last().unwrap();
            eq.push(prev * 1.01);
        }
        let vol = rolling_volatility(&eq, 3);
        let sh = rolling_sharpe(&eq, 3);
        assert!(vol[2].is_nan() && sh[2].is_nan());
        assert!(vol[3].abs() < 1e-9, "constant returns -> zero vol");
        assert!(sh[3] > 0.0);
        assert_eq!(vol.len(), eq.len());
    }

    #[test]
    fn benchmark_relative_metrics() {
        // Strategy daily returns are exactly 2x the benchmark's.
        let bench = [1.0, 1.01, 1.0201, 1.01, 1.0201];
        let mut eq = vec![1.0];
        for i in 1..bench.len() {
            let b = bench[i] / bench[i - 1] - 1.0;
            let prev = *eq.last().unwrap();
            eq.push(prev * (1.0 + 2.0 * b));
        }
        assert!((beta(&eq, &bench) - 2.0).abs() < 1e-9, "beta");
        assert!(alpha(&eq, &bench).abs() < 1e-9, "alpha ~ 0");

        // Identical curves: beta 1, zero tracking error and IR.
        assert!((beta(&bench, &bench) - 1.0).abs() < 1e-12);
        assert!(tracking_error(&bench, &bench).abs() < 1e-12);
        assert!(information_ratio(&bench, &bench).abs() < 1e-9);

        // Flat benchmark: no variance -> beta NaN.
        let flat = [1.0, 1.0, 1.0];
        assert!(beta(&bench[..3], &flat).is_nan());

        // Benchmark total return ignores leading/trailing NaN.
        let with_nan = [f64::NAN, 1.0, 1.1, f64::NAN];
        assert!((benchmark_return(&with_nan) - 0.1).abs() < 1e-12);
        assert!(benchmark_return(&[f64::NAN]).is_nan());
    }

    #[test]
    fn returns_drawdown_and_totals() {
        let eq = [1.0, 1.02, 1.01, 1.05];
        let r = to_returns(&eq);
        assert!(r[0].is_nan());
        assert!((r[1] - 0.02).abs() < 1e-12);
        assert!((total_return(&eq) - 0.05).abs() < 1e-12);
        // drawdown: peak 1.02 then 1.01 -> -0.009803...
        let dd = drawdown_series(&eq);
        assert_eq!(dd[0], 0.0);
        assert!((dd[2] - (1.01 / 1.02 - 1.0)).abs() < 1e-12);
        assert!((max_drawdown(&eq) - (1.01 / 1.02 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn empty_and_single_inputs() {
        assert!(max_drawdown(&[]).is_nan());
        assert!(total_return(&[]).is_nan());
        let one = to_returns(&[1.0]);
        assert_eq!(one.len(), 1);
        assert!(one[0].is_nan());
        assert_eq!(drawdown_series(&[1.0]), vec![0.0]);
    }

    #[test]
    fn cagr_and_calmar_guard_short_input() {
        assert!(cagr(&[1.0], &[20240102]).is_nan());
        assert!(calmar(&[1.0], &[20240102]).is_nan());
    }

    #[test]
    fn calmar_guards_zero_drawdown() {
        // flat curve: cagr=0 and max_drawdown=0 -> 0/0 = NaN without a guard.
        // mirror recovery_factor and return +inf for the no-drawdown case.
        assert!(calmar(&[1.0, 1.0, 1.0], &[20240102, 20240103, 20240104]).is_infinite());
    }

    #[test]
    fn trade_level_metrics() {
        use crate::backtest::Trade;
        let t = |ret: f64, period: u32| Trade {
            symbol: "X".into(),
            entry_date: 20240102,
            exit_date: Some(20240105),
            ret,
            period,
            mae: None,
            mfe: None,
        };
        let trades = vec![t(0.10, 3), t(-0.05, 2), t(0.20, 5)];
        assert!((win_rate(&trades) - 2.0 / 3.0).abs() < 1e-12);
        assert!((profit_factor(&trades) - (0.30 / 0.05)).abs() < 1e-12);
        assert!((expectancy(&trades) - (0.25 / 3.0)).abs() < 1e-12);
        assert!((avg_holding_period(&trades) - (10.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn extended_trade_level_metrics() {
        use crate::backtest::Trade;
        let t = |ret: f64, exit: i32| Trade {
            symbol: "X".into(),
            entry_date: 20240102,
            exit_date: Some(exit),
            ret,
            period: 1,
            mae: None,
            mfe: None,
        };
        // chronological by exit_date: +0.10, -0.05, -0.20, +0.30, -0.10
        let trades = vec![
            t(0.10, 20240105),
            t(-0.05, 20240106),
            t(-0.20, 20240107),
            t(0.30, 20240108),
            t(-0.10, 20240109),
        ];
        assert_eq!(num_trades(&trades), 5.0);
        assert!((avg_win(&trades) - (0.40 / 2.0)).abs() < 1e-12); // (0.10+0.30)/2
        assert!((avg_loss(&trades) - (-0.35 / 3.0)).abs() < 1e-12); // (-0.05-0.20-0.10)/3
        assert!((payoff_ratio(&trades) - (0.20 / (0.35 / 3.0))).abs() < 1e-12);
        assert!((best_trade(&trades) - 0.30).abs() < 1e-12);
        assert!((worst_trade(&trades) + 0.20).abs() < 1e-12);
        // losses at exits 106,107 are consecutive (run 2); 109 is a lone run -> max 2.
        assert_eq!(max_consecutive_losses(&trades), 2.0);
    }

    #[test]
    fn trade_metrics_handle_empty_and_one_sided() {
        use crate::backtest::Trade;
        let win = vec![Trade {
            symbol: "X".into(),
            entry_date: 20240102,
            exit_date: Some(20240103),
            ret: 0.1,
            period: 1,
            mae: None,
            mfe: None,
        }];
        assert_eq!(num_trades(&[]), 0.0);
        assert!(avg_win(&[]).is_nan());
        assert!(avg_loss(&win).is_nan()); // no losers
        assert!(payoff_ratio(&win).is_nan()); // loss side empty
        assert!(best_trade(&[]).is_nan());
        assert_eq!(max_consecutive_losses(&win), 0.0);
    }

    #[test]
    fn max_consecutive_losses_sorts_by_exit_date() {
        use crate::backtest::Trade;
        // Array order is NOT chronological: exit dates 105, 103, 104.
        // Chronological by exit_date: 103 (win), 104 (loss), 105 (loss) -> streak 2.
        // Without the exit_date sort, array order gives loss, win(reset), loss -> 1.
        // Asserting 2 therefore fails if the sort is ever dropped.
        let t = |ret: f64, exit: i32| Trade {
            symbol: "X".into(),
            entry_date: 20240102,
            exit_date: Some(exit),
            ret,
            period: 1,
            mae: None,
            mfe: None,
        };
        let trades = vec![t(-0.1, 20240105), t(0.2, 20240103), t(-0.1, 20240104)];
        assert_eq!(max_consecutive_losses(&trades), 2.0);
    }

    #[test]
    fn equity_and_exposure_metrics() {
        // peak 1.0 then underwater rows 1,2,3 (0.9,0.8,0.9), recover at row4.
        let eq = [1.0, 0.9, 0.8, 0.9, 1.0];
        // total_return = 0.0; max_drawdown = 0.8/1.0 - 1 = -0.2 -> recovery 0/0.2 = 0.
        assert!((recovery_factor(&eq) - 0.0).abs() < 1e-12);
        assert_eq!(max_drawdown_duration(&eq), 3.0); // 3 consecutive rows below peak

        // recovery_factor returns +inf when there is no drawdown.
        assert!(recovery_factor(&[1.0, 1.1, 1.2]).is_infinite());

        let exposure = [1.0, 0.0, 0.5, 0.5];
        assert!((time_in_market(&exposure) - 0.75).abs() < 1e-12); // 3 of 4 rows > 0
        assert!((avg_exposure(&exposure) - 0.5).abs() < 1e-12); // (1+0+0.5+0.5)/4
        assert!(time_in_market(&[]).is_nan());
        assert!(avg_exposure(&[]).is_nan());
    }

    #[test]
    fn distribution_and_tail_metrics() {
        // equity -> daily returns [0.1, -0.1, 0.1]; hand-computed moments below.
        let eq = [100.0, 110.0, 99.0, 108.9];
        assert!((best_day(&eq) - 0.1).abs() < 1e-12);
        assert!((worst_day(&eq) + 0.1).abs() < 1e-12);
        // mean 1/30; m2 = 0.0088889, m3 = -0.000592593, m4 = 1.18519e-4.
        assert!((skew(&eq) + 0.5_f64.sqrt()).abs() < 1e-9); // -1/sqrt(2)
        assert!((kurtosis(&eq) + 1.5).abs() < 1e-9);
        // sorted returns [-0.1, 0.1, 0.1]; 5th pct = -0.1 + 0.2*0.1 = -0.08.
        assert!((var_95(&eq) + 0.08).abs() < 1e-12);
        // only -0.1 is <= -0.08 -> cvar = -0.1.
        assert!((cvar_95(&eq) + 0.1).abs() < 1e-12);
        // degenerate / empty guards.
        assert!(skew(&[1.0]).is_nan());
        assert!(best_day(&[1.0]).is_nan());
        assert!(var_95(&[1.0, 1.0, 1.0]).abs() < 1e-12); // all-zero returns
    }

    #[test]
    fn drawdown_shape_metrics() {
        // drawdown series [0, 0, -0.1, -0.01] for this curve.
        let eq = [100.0, 110.0, 99.0, 108.9];
        assert!((avg_drawdown(&eq) + 0.0275).abs() < 1e-12); // (0+0-0.1-0.01)/4
        let want = ((0.1_f64.powi(2) + 0.01_f64.powi(2)) / 4.0).sqrt();
        assert!((ulcer_index(&eq) - want).abs() < 1e-12);
        assert!(avg_drawdown(&[]).is_nan());
        assert!(ulcer_index(&[]).is_nan());
    }

    #[test]
    fn lookback_returns_anchor_and_gate_on_history() {
        let dates = [20210701, 20220701, 20230701, 20231231, 20240701];
        let eq = [100.0, 110.0, 120.0, 125.0, 132.0];
        // YTD anchors at the prior-year-end row (20231231 = 125).
        assert!((ytd_return(&dates, &eq).unwrap() - (132.0 / 125.0 - 1.0)).abs() < 1e-12);
        // 1y anchors at 20230701 = 120; 3y at 20210701 = 100.
        assert!((trailing_return(&dates, &eq, 1).unwrap() - (132.0 / 120.0 - 1.0)).abs() < 1e-12);
        assert!((trailing_return(&dates, &eq, 3).unwrap() - 0.32).abs() < 1e-12);

        // A backtest entirely within the current year: every window is None.
        let d2 = [20240102, 20240103];
        let e2 = [100.0, 110.0];
        assert!(ytd_return(&d2, &e2).is_none());
        assert!(trailing_return(&d2, &e2, 1).is_none());
        assert!(trailing_return(&d2, &e2, 3).is_none());
    }

    #[test]
    fn shift_years_clamps_leap_day() {
        assert_eq!(shift_years(20240229, -1), 20230228);
        assert_eq!(shift_years(20240701, -3), 20210701);
    }
}
