//! Time-series + cross-sectional indicators: `average` (rolling mean with
//! `min_periods=floor(n/2)`), `rise`/`fall` (shift comparisons),
//! `rank_cs` (cross-sectional rank), `quantile_row` (per-row quantile),
//! `rsi` (Wilder's RSI, TA-Lib compatible).

use crate::ops::stat::{average_ranks, sort_f64s, sorted_quantile};
use crate::panel::Panel;
use ndarray::Array2;

/// RSI from running average gain/loss. `avg_loss == 0` yields 100 — TA-Lib's
/// convention (also covers a flat series, where both averages are 0).
fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        100.0
    } else {
        100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
    }
}

impl Panel {
    pub fn average(&self, n: usize) -> Panel {
        let min_periods = n / 2;
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        for c in 0..ncols {
            for r in 0..nrows {
                let lo = r.saturating_sub(n - 1);
                let mut sum = 0.0;
                let mut cnt = 0usize;
                for k in lo..=r {
                    let v = self.data[[k, c]];
                    if !v.is_nan() {
                        sum += v;
                        cnt += 1;
                    }
                }
                if cnt >= min_periods.max(1) {
                    out[[r, c]] = sum / cnt as f64;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Rolling-window maximum over `n` periods (`min_periods = n`): the first
    /// finite value is at row `n-1`; any `NaN` inside a window yields `NaN`.
    /// `close == rolling_max(close, n)` flags a new N-day high.
    pub fn rolling_max(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data: out,
            };
        }
        for c in 0..ncols {
            for r in (n - 1)..nrows {
                let lo = r + 1 - n;
                let mut m = f64::NEG_INFINITY;
                let mut ok = true;
                for k in lo..=r {
                    let v = self.data[[k, c]];
                    if v.is_nan() {
                        ok = false;
                        break;
                    }
                    if v > m {
                        m = v;
                    }
                }
                if ok {
                    out[[r, c]] = m;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Rolling-window minimum over `n` periods (`min_periods = n`): the first
    /// finite value is at row `n-1`; any `NaN` inside a window yields `NaN`.
    pub fn rolling_min(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data: out,
            };
        }
        for c in 0..ncols {
            for r in (n - 1)..nrows {
                let lo = r + 1 - n;
                let mut m = f64::INFINITY;
                let mut ok = true;
                for k in lo..=r {
                    let v = self.data[[k, c]];
                    if v.is_nan() {
                        ok = false;
                        break;
                    }
                    if v < m {
                        m = v;
                    }
                }
                if ok {
                    out[[r, c]] = m;
                }
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    pub fn rise(&self, n: usize) -> Panel {
        self.gt(&self.shift(n))
    }

    /// `(self - shift(n)) / shift(n)` — pandas-style `pct_change(n)`. The first `n`
    /// rows (and any cell whose `n`-ago value is missing) are `NaN`; a zero base
    /// yields `±inf`/`NaN` as in pandas.
    pub fn pct_change(&self, n: usize) -> Panel {
        let prev = self.shift(n);
        self.sub(&prev).div(&prev)
    }

    pub fn fall(&self, n: usize) -> Panel {
        self.lt(&self.shift(n))
    }

    pub fn rank_cs(&self, pct: bool, ascending: bool) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        for r in 0..nrows {
            let mut cols = Vec::new();
            let mut vals = Vec::new();
            for c in 0..ncols {
                let v = self.data[[r, c]];
                if !v.is_nan() {
                    cols.push(c);
                    // Negate for descending so shared average_ranks (ascending)
                    // yields low ranks for high values.
                    vals.push(if ascending { v } else { -v });
                }
            }
            if vals.is_empty() {
                continue;
            }
            let ranks = average_ranks(&vals);
            let count = vals.len() as f64;
            for (i, &c) in cols.iter().enumerate() {
                let avg = ranks[i];
                out[[r, c]] = if pct { avg / count } else { avg };
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Wilder's RSI over `n` periods, computed per symbol down the time axis —
    /// matches TA-Lib `RSI(timeperiod=n)`. The average
    /// gain/loss is seeded with the simple mean of the first `n` deltas, then
    /// Wilder-smoothed; the first finite value lands `n` rows after a symbol's
    /// first finite price (leading `NaN`s, before a stock lists, are skipped).
    // ponytail: assumes prices are contiguous after the first finite value (the
    // close panel is forward-filled); a non-finite step is treated as 0 change
    // rather than poisoning the running average.
    pub fn rsi(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data: out,
            };
        }
        let delta = |a: f64, b: f64| {
            let d = a - b;
            if d.is_finite() {
                d
            } else {
                0.0
            }
        };
        for c in 0..ncols {
            let Some(start) = (0..nrows).find(|&r| self.data[[r, c]].is_finite()) else {
                continue;
            };
            if start + n >= nrows {
                continue; // not enough history for even one value
            }
            // Seed: simple mean of the first n deltas (gains vs losses).
            let mut avg_gain = 0.0;
            let mut avg_loss = 0.0;
            for r in (start + 1)..=(start + n) {
                let d = delta(self.data[[r, c]], self.data[[r - 1, c]]);
                if d > 0.0 {
                    avg_gain += d;
                } else {
                    avg_loss += -d;
                }
            }
            avg_gain /= n as f64;
            avg_loss /= n as f64;
            out[[start + n, c]] = rsi_from(avg_gain, avg_loss);
            // Wilder smoothing for every subsequent row.
            for r in (start + n + 1)..nrows {
                let d = delta(self.data[[r, c]], self.data[[r - 1, c]]);
                let (g, l) = if d > 0.0 { (d, 0.0) } else { (0.0, -d) };
                avg_gain = (avg_gain * (n as f64 - 1.0) + g) / n as f64;
                avg_loss = (avg_loss * (n as f64 - 1.0) + l) / n as f64;
                out[[r, c]] = rsi_from(avg_gain, avg_loss);
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// TA-Lib-compatible EMA over `n` periods: seeded with the SMA of the first
    /// `n` finite values, then `ema[i] = v*k + ema[i-1]*(1-k)` with `k = 2/(n+1)`.
    /// The first finite value lands at a symbol's `n`-th finite row (leading
    /// `NaN`s skipped). The MACD line is `ema(fast) - ema(slow)`.
    // ponytail: assumes prices are contiguous after the first finite value; a
    // non-finite step carries the previous EMA forward rather than corrupting it.
    pub fn ema(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data: out,
            };
        }
        let k = 2.0 / (n as f64 + 1.0);
        for c in 0..ncols {
            let Some(start) = (0..nrows).find(|&r| self.data[[r, c]].is_finite()) else {
                continue;
            };
            if start + n > nrows {
                continue;
            }
            let mut ema = (start..(start + n)).map(|r| self.data[[r, c]]).sum::<f64>() / n as f64;
            out[[start + n - 1, c]] = ema;
            for r in (start + n)..nrows {
                let v = self.data[[r, c]];
                let v = if v.is_finite() { v } else { ema };
                ema = v * k + ema * (1.0 - k);
                out[[r, c]] = ema;
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Rolling population standard deviation over `n` periods (`ddof=0`,
    /// `min_periods=n`) — the dispersion term TA-Lib's BBANDS uses. The first
    /// finite value is at row `n-1`; any `NaN` inside a window yields `NaN`.
    /// Bollinger bands are `average(n) ± k * rolling_std(n)`.
    pub fn rolling_std(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
        if n == 0 {
            return Panel {
                dates: self.dates.clone(),
                symbols: self.symbols.clone(),
                data: out,
            };
        }
        for c in 0..ncols {
            for r in (n - 1)..nrows {
                let lo = r + 1 - n;
                let window = (lo..=r).map(|k| self.data[[k, c]]);
                if window.clone().any(|v| !v.is_finite()) {
                    continue;
                }
                let mean = window.clone().sum::<f64>() / n as f64;
                let var = window.map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
                out[[r, c]] = var.sqrt();
            }
        }
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    /// Donchian channel upper band: the rolling `n`-day high. Alias of
    /// `rolling_max` under the named-indicator surface (`min_periods = n`; first
    /// finite at row `n-1`).
    pub fn donchian_high(&self, n: usize) -> Panel {
        self.rolling_max(n)
    }

    /// Donchian channel lower band: the rolling `n`-day low. Alias of
    /// `rolling_min` (`min_periods = n`).
    pub fn donchian_low(&self, n: usize) -> Panel {
        self.rolling_min(n)
    }

    /// Donchian channel mid-line: `(rolling_max(n) + rolling_min(n)) / 2`.
    /// Finite only where both bands are (row `n-1` onward).
    pub fn donchian_mid(&self, n: usize) -> Panel {
        self.rolling_max(n)
            .add(&self.rolling_min(n))
            .scalar_mul(0.5)
    }

    /// Bollinger mid band: the `n`-day simple moving average (`average`, so
    /// `min_periods = n/2`).
    pub fn bollinger_mid(&self, n: usize) -> Panel {
        self.average(n)
    }

    /// Bollinger upper band: `average(n) + k * rolling_std(n)`. The dispersion
    /// term warms up at row `n-1` (`rolling_std` uses `min_periods = n`), so the
    /// band is `NaN` before then even where the mid line (`min_periods = n/2`) is
    /// already finite.
    pub fn bollinger_upper(&self, n: usize, k: f64) -> Panel {
        self.average(n).add(&self.rolling_std(n).scalar_mul(k))
    }

    /// Bollinger lower band: `average(n) - k * rolling_std(n)`.
    pub fn bollinger_lower(&self, n: usize, k: f64) -> Panel {
        self.average(n).sub(&self.rolling_std(n).scalar_mul(k))
    }

    /// MACD line: `ema(fast) - ema(slow)`. Finite once the slow EMA warms up (a
    /// symbol's `slow`-th finite row).
    pub fn macd(&self, fast: usize, slow: usize) -> Panel {
        self.ema(fast).sub(&self.ema(slow))
    }

    /// MACD signal line: the `signal`-day EMA of the MACD line (seeded, like all
    /// EMAs here, from the SMA of its first `signal` finite values).
    pub fn macd_signal(&self, fast: usize, slow: usize, signal: usize) -> Panel {
        self.macd(fast, slow).ema(signal)
    }

    /// MACD histogram: the MACD line minus its signal line.
    pub fn macd_hist(&self, fast: usize, slow: usize, signal: usize) -> Panel {
        self.macd(fast, slow)
            .sub(&self.macd_signal(fast, slow, signal))
    }

    pub fn quantile_row(&self, c: f64) -> Panel {
        let nrows = self.nrows();
        let mut out = Array2::from_elem((nrows, 1), f64::NAN);
        for r in 0..nrows {
            let mut vals: Vec<f64> = (0..self.ncols())
                .map(|j| self.data[[r, j]])
                .filter(|x| !x.is_nan())
                .collect();
            if vals.is_empty() {
                continue;
            }
            sort_f64s(&mut vals);
            out[[r, 0]] = sorted_quantile(&vals, c);
        }
        Panel {
            dates: self.dates.clone(),
            symbols: vec!["quantile".into()],
            data: out,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::panel::Panel;
    use proptest::prelude::*;

    #[test]
    fn average_min_periods_is_half_n() {
        // n=2 => min_periods=1, so row 0 = the value itself
        let p = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![10.0], vec![12.0], vec![14.0]],
        )
        .unwrap();
        let a = p.average(2);
        assert_eq!(a.data[[0, 0]], 10.0);
        assert_eq!(a.data[[1, 0]], 11.0);
        assert_eq!(a.data[[2, 0]], 13.0);
    }

    #[test]
    fn rsi_matches_wilder_definition() {
        // n=3, closes 10,11,10,12,13,12 — deltas +1,-1,+2,+1,-1.
        // Seed (first 3 deltas +1,-1,+2): avg_gain=1.0, avg_loss=1/3 -> RSI=75.
        // Wilder step (+1): gain=(1*2+1)/3=1.0, loss=(.3333*2)/3=.2222 -> 81.81818.
        // Wilder step (-1): gain=(1*2)/3=.6667, loss=(.2222*2+1)/3=.4815 -> 58.06452.
        let p = Panel::from_rows(
            (0..6).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![
                vec![10.0],
                vec![11.0],
                vec![10.0],
                vec![12.0],
                vec![13.0],
                vec![12.0],
            ],
        )
        .unwrap();
        let r = p.rsi(3);
        for row in 0..3 {
            assert!(r.data[[row, 0]].is_nan(), "warm-up row {row} should be NaN");
        }
        assert!((r.data[[3, 0]] - 75.0).abs() < 1e-6);
        assert!((r.data[[4, 0]] - 81.818181).abs() < 1e-6);
        assert!((r.data[[5, 0]] - 58.064516).abs() < 1e-6);
    }

    #[test]
    fn rsi_monotonic_extremes_and_leading_nan() {
        // Column A strictly rising -> 100; B strictly falling -> 0; both have a
        // leading NaN so the first finite RSI is at row start+n = 1+2 = 3.
        let p = Panel::from_rows(
            (0..5).map(|i| 20240102 + i).collect(),
            vec!["A".into(), "B".into()],
            vec![
                vec![f64::NAN, f64::NAN],
                vec![10.0, 50.0],
                vec![11.0, 40.0],
                vec![12.0, 30.0],
                vec![13.0, 20.0],
            ],
        )
        .unwrap();
        let r = p.rsi(2);
        assert!(r.data[[2, 0]].is_nan()); // still warming up
        assert_eq!(r.data[[3, 0]], 100.0);
        assert_eq!(r.data[[3, 1]], 0.0);
        assert_eq!(r.data[[4, 0]], 100.0);
        assert_eq!(r.data[[4, 1]], 0.0);
    }

    #[test]
    fn pct_change_basic_and_warmup() {
        // [10,11,12] pct_change(1): row0 NaN, (11-10)/10=0.1, (12-11)/11=.090909
        let p = Panel::from_rows(
            (0..3).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![10.0], vec![11.0], vec![12.0]],
        )
        .unwrap();
        let r = p.pct_change(1);
        assert!(r.data[[0, 0]].is_nan());
        assert!((r.data[[1, 0]] - 0.1).abs() < 1e-12);
        assert!((r.data[[2, 0]] - 1.0 / 11.0).abs() < 1e-12);
    }

    #[test]
    fn ema_matches_talib_sma_seed() {
        // n=3, [1,2,3,4,5], k=0.5. seed@idx2 = mean(1,2,3)=2.0;
        // idx3 = 4*.5 + 2*.5 = 3.0; idx4 = 5*.5 + 3*.5 = 4.0.
        let p = Panel::from_rows(
            (0..5).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]],
        )
        .unwrap();
        let e = p.ema(3);
        assert!(e.data[[0, 0]].is_nan());
        assert!(e.data[[1, 0]].is_nan());
        assert_eq!(e.data[[2, 0]], 2.0);
        assert_eq!(e.data[[3, 0]], 3.0);
        assert_eq!(e.data[[4, 0]], 4.0);
    }

    #[test]
    fn rolling_std_population_min_periods_n() {
        // n=3, [1,2,3,4]: idx2 window[1,2,3] mean2 var=2/3 -> sqrt; idx3 same.
        let p = Panel::from_rows(
            (0..4).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0]],
        )
        .unwrap();
        let s = p.rolling_std(3);
        assert!(s.data[[1, 0]].is_nan()); // warm-up
        assert!((s.data[[2, 0]] - (2.0f64 / 3.0).sqrt()).abs() < 1e-12);
        assert!((s.data[[3, 0]] - (2.0f64 / 3.0).sqrt()).abs() < 1e-12);
    }

    #[test]
    fn rolling_max_window_and_warmup() {
        // n=3 over [10,12,11,15,9]: r2=12, r3=15, r4=15; r0,r1 NaN.
        let p = Panel::from_rows(
            (0..5).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![10.0], vec![12.0], vec![11.0], vec![15.0], vec![9.0]],
        )
        .unwrap();
        let m = p.rolling_max(3);
        assert!(m.data[[1, 0]].is_nan());
        assert_eq!(m.data[[2, 0]], 12.0);
        assert_eq!(m.data[[3, 0]], 15.0);
        assert_eq!(m.data[[4, 0]], 15.0);
    }

    #[test]
    fn rolling_min_window_and_warmup() {
        // n=3 over [10,12,11,15,9]: r2=10, r3=11, r4=9; r0,r1 NaN.
        let p = Panel::from_rows(
            (0..5).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![10.0], vec![12.0], vec![11.0], vec![15.0], vec![9.0]],
        )
        .unwrap();
        let m = p.rolling_min(3);
        assert!(m.data[[1, 0]].is_nan());
        assert_eq!(m.data[[2, 0]], 10.0);
        assert_eq!(m.data[[3, 0]], 11.0);
        assert_eq!(m.data[[4, 0]], 9.0);
    }

    /// Single-column panel from a slice of daily values, dates from 20240102.
    fn col(v: &[f64]) -> Panel {
        Panel::from_rows(
            (0..v.len() as i32).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            v.iter().map(|x| vec![*x]).collect(),
        )
        .unwrap()
    }

    #[test]
    fn donchian_bands_wrap_rolling_extremes() {
        // n=3 over [10,12,11,15,9]: high = rolling_max, low = rolling_min,
        // mid = (high+low)/2. r0,r1 warm up to NaN.
        let p = col(&[10.0, 12.0, 11.0, 15.0, 9.0]);
        let hi = p.donchian_high(3);
        let lo = p.donchian_low(3);
        let mid = p.donchian_mid(3);
        assert!(hi.data[[1, 0]].is_nan() && lo.data[[1, 0]].is_nan() && mid.data[[1, 0]].is_nan());
        assert_eq!(
            (hi.data[[2, 0]], hi.data[[3, 0]], hi.data[[4, 0]]),
            (12.0, 15.0, 15.0)
        );
        assert_eq!(
            (lo.data[[2, 0]], lo.data[[3, 0]], lo.data[[4, 0]]),
            (10.0, 11.0, 9.0)
        );
        assert_eq!(
            (mid.data[[2, 0]], mid.data[[3, 0]], mid.data[[4, 0]]),
            (11.0, 13.0, 12.0)
        );
    }

    #[test]
    fn bollinger_bands_are_sma_plus_minus_k_std() {
        // n=3, k=2 over [1,2,3,4]. average(min_periods=n/2=1): 1,1.5,2,3.
        // rolling_std(min_periods=3): NaN,NaN,sqrt(2/3),sqrt(2/3).
        let p = col(&[1.0, 2.0, 3.0, 4.0]);
        let mid = p.bollinger_mid(3);
        let up = p.bollinger_upper(3, 2.0);
        let lo = p.bollinger_lower(3, 2.0);
        let s = (2.0f64 / 3.0).sqrt();
        // Mid is the SMA and is finite from row 0 (min_periods = n/2).
        assert_eq!(mid.data[[0, 0]], 1.0);
        assert_eq!(mid.data[[2, 0]], 2.0);
        // Bands warm up with the std term: NaN until row n-1 even though mid is finite.
        assert!(up.data[[1, 0]].is_nan() && lo.data[[1, 0]].is_nan());
        assert!((up.data[[2, 0]] - (2.0 + 2.0 * s)).abs() < 1e-12);
        assert!((lo.data[[2, 0]] - (2.0 - 2.0 * s)).abs() < 1e-12);
        assert!((up.data[[3, 0]] - (3.0 + 2.0 * s)).abs() < 1e-12);
        assert!((lo.data[[3, 0]] - (3.0 - 2.0 * s)).abs() < 1e-12);
    }

    #[test]
    fn macd_line_signal_and_hist_compose_from_ema() {
        // fast=2, slow=3, signal=2 over [1,2,3,4,5].
        // ema(2)=[_,1.5,2.5,3.5,4.5]; ema(3)=[_,_,2,3,4].
        // macd = ema2-ema3 = [_,_,0.5,0.5,0.5]; signal = ema(macd,2) = [_,_,_,0.5,0.5];
        // hist = macd-signal = [_,_,_,0,0].
        let p = col(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        let macd = p.macd(2, 3);
        let sig = p.macd_signal(2, 3, 2);
        let hist = p.macd_hist(2, 3, 2);
        assert!(macd.data[[1, 0]].is_nan());
        for r in 2..5 {
            assert!((macd.data[[r, 0]] - 0.5).abs() < 1e-12);
        }
        assert!(sig.data[[2, 0]].is_nan());
        assert!((sig.data[[3, 0]] - 0.5).abs() < 1e-12);
        assert!((sig.data[[4, 0]] - 0.5).abs() < 1e-12);
        assert!(hist.data[[2, 0]].is_nan());
        assert!((hist.data[[3, 0]] - 0.0).abs() < 1e-12);
        assert!((hist.data[[4, 0]] - 0.0).abs() < 1e-12);
    }

    #[test]
    fn rank_cs_pct_ignores_nan() {
        let p = Panel::from_rows(
            vec![20240102],
            vec!["A".into(), "B".into(), "C".into()],
            vec![vec![10.0, 30.0, f64::NAN]],
        )
        .unwrap();
        let r = p.rank_cs(false, true); // ascending dense rank
        assert_eq!(r.data[[0, 0]], 1.0);
        assert_eq!(r.data[[0, 1]], 2.0);
        assert!(r.data[[0, 2]].is_nan());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Rolling min/max never panic; warm-up / NaN windows stay NaN; finite
        /// full windows yield a finite extremum from the window.
        #[test]
        fn rolling_min_max_edge_inputs(
            values in prop::collection::vec(
                prop_oneof![Just(f64::NAN), -1e3f64..1e3f64],
                0..16,
            ),
            n in 0usize..8,
        ) {
            let dates: Vec<i32> = (0..values.len() as i32).map(|i| 20240101 + i).collect();
            let rows: Vec<Vec<f64>> = values.iter().map(|&v| vec![v]).collect();
            let p = Panel::from_rows(dates, vec!["A".into()], rows).unwrap();
            let rmax = p.rolling_max(n);
            let rmin = p.rolling_min(n);
            prop_assert_eq!(rmax.nrows(), p.nrows());
            prop_assert_eq!(rmin.nrows(), p.nrows());
            if n == 0 || p.nrows() == 0 {
                for r in 0..p.nrows() {
                    prop_assert!(rmax.data[[r, 0]].is_nan());
                    prop_assert!(rmin.data[[r, 0]].is_nan());
                }
                return Ok(());
            }
            for r in 0..p.nrows() {
                if r < n - 1 {
                    prop_assert!(rmax.data[[r, 0]].is_nan());
                    prop_assert!(rmin.data[[r, 0]].is_nan());
                    continue;
                }
                let lo = r + 1 - n;
                let window = &values[lo..=r];
                if window.iter().any(|v| v.is_nan()) {
                    prop_assert!(rmax.data[[r, 0]].is_nan());
                    prop_assert!(rmin.data[[r, 0]].is_nan());
                } else {
                    let expect_max = window.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let expect_min = window.iter().copied().fold(f64::INFINITY, f64::min);
                    prop_assert!((rmax.data[[r, 0]] - expect_max).abs() < 1e-12);
                    prop_assert!((rmin.data[[r, 0]] - expect_min).abs() < 1e-12);
                }
            }
        }
    }
}
