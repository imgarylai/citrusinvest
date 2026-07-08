//! Multi-input OHLCV technical indicators (ATR/NATR, ADX/DMI, Stochastic KD,
//! Williams %R, CCI, Aroon, OBV, MFI). Each is a free function over panels that
//! share the close grid, computed per symbol down the time axis. Warm-up mirrors
//! `rsi`/`ema`: leading NaNs (before a symbol lists) are skipped; the first
//! finite output lands once the window is full.
use crate::panel::Panel;
use ndarray::Array2;

/// First finite row in column `c` (a symbol's listing date), or None.
pub(crate) fn col_start(p: &Panel, c: usize) -> Option<usize> {
    (0..p.nrows()).find(|&r| p.data[[r, c]].is_finite())
}

/// ATR-style Wilder smoothing of a per-column raw series. `raw[r]` is the bar
/// quantity (TR, +DM, …), finite for `r` in `(first+1)..len`; `first` is the
/// column's listing row. Output is NaN until row `first+period` (seed = simple
/// mean of `raw[first+1..=first+period]`), then `(prev*(period-1)+raw[r])/period`.
pub(crate) fn wilder_col(raw: &[f64], first: usize, period: usize) -> Vec<f64> {
    let len = raw.len();
    let mut out = vec![f64::NAN; len];
    if period == 0 || first + period >= len {
        return out;
    }
    let seed = (first + 1..=first + period).map(|r| raw[r]).sum::<f64>() / period as f64;
    out[first + period] = seed;
    let mut prev = seed;
    for r in (first + period + 1)..len {
        prev = (prev * (period as f64 - 1.0) + raw[r]) / period as f64;
        out[r] = prev;
    }
    out
}

/// Average True Range (Wilder), priced off the high/low/close panels.
pub fn atr(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let mut tr = vec![f64::NAN; nrows];
        for r in (first + 1)..nrows {
            let h = high.data[[r, c]];
            let l = low.data[[r, c]];
            let pc = close.data[[r - 1, c]];
            tr[r] = (h - l).max((h - pc).abs()).max((l - pc).abs());
        }
        let sm = wilder_col(&tr, first, n);
        for r in 0..nrows {
            out[[r, c]] = sm[r];
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Normalized ATR: `100 * ATR / close`.
pub fn natr(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let a = atr(high, low, close, n);
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        for r in 0..nrows {
            let v = a.data[[r, c]];
            let cl = close.data[[r, c]];
            if v.is_finite() && cl != 0.0 {
                out[[r, c]] = 100.0 * v / cl;
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Commodity Channel Index over typical price `TP=(H+L+C)/3`, constant 0.015.
pub fn cci(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let tp: Vec<f64> = (0..nrows)
            .map(|r| (high.data[[r, c]] + low.data[[r, c]] + close.data[[r, c]]) / 3.0)
            .collect();
        for r in (first + n - 1)..nrows {
            let lo = r + 1 - n;
            let w: Vec<f64> = (lo..=r).map(|k| tp[k]).collect();
            if w.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let sma = w.iter().sum::<f64>() / n as f64;
            let md = w.iter().map(|v| (v - sma).abs()).sum::<f64>() / n as f64;
            if md != 0.0 {
                out[[r, c]] = (tp[r] - sma) / (0.015 * md);
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Aroon Up/Down core: over a window of `n+1` bars ending at `r`, find the most
/// recent extreme (`>=`/`<=` keeps the latest on ties) and map days-since to
/// `100*(n - daysSince)/n`. `want_max` = Aroon Up (highs); else Down (lows).
fn aroon_dir(p: &Panel, n: usize, want_max: bool) -> Panel {
    let (nrows, ncols) = p.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    if n == 0 {
        return Panel { dates: p.dates.clone(), symbols: p.symbols.clone(), data: out };
    }
    for c in 0..ncols {
        let Some(first) = col_start(p, c) else { continue };
        for r in (first + n)..nrows {
            let lo = r - n;
            let mut ext = p.data[[lo, c]];
            let mut idx = lo;
            let mut ok = ext.is_finite();
            for k in lo..=r {
                let v = p.data[[k, c]];
                if !v.is_finite() {
                    ok = false;
                    break;
                }
                if (want_max && v >= ext) || (!want_max && v <= ext) {
                    ext = v;
                    idx = k;
                }
            }
            if ok {
                let days_since = (r - idx) as f64;
                out[[r, c]] = 100.0 * (n as f64 - days_since) / n as f64;
            }
        }
    }
    Panel { dates: p.dates.clone(), symbols: p.symbols.clone(), data: out }
}

pub fn aroon_up(high: &Panel, n: usize) -> Panel {
    aroon_dir(high, n, true)
}

pub fn aroon_down(low: &Panel, n: usize) -> Panel {
    aroon_dir(low, n, false)
}

/// Fast stochastic %K: `100 * (C - LLₙ) / (HHₙ - LLₙ)`.
pub fn stoch_k(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let hh = high.rolling_max(n);
    let ll = low.rolling_min(n);
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        for r in 0..nrows {
            let (h, l, cl) = (hh.data[[r, c]], ll.data[[r, c]], close.data[[r, c]]);
            if h.is_finite() && l.is_finite() && cl.is_finite() && h != l {
                out[[r, c]] = 100.0 * (cl - l) / (h - l);
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// %D: full-window `d`-period SMA of %K (leading-NaN aware, `min_periods = d`).
pub fn stoch_d(high: &Panel, low: &Panel, close: &Panel, n: usize, d: usize) -> Panel {
    let k = stoch_k(high, low, close, n);
    let (nrows, ncols) = k.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    if d == 0 {
        return Panel { dates: k.dates.clone(), symbols: k.symbols.clone(), data: out };
    }
    for c in 0..ncols {
        let Some(first) = col_start(&k, c) else { continue };
        for r in (first + d - 1)..nrows {
            let lo = r + 1 - d;
            let w: Vec<f64> = (lo..=r).map(|i| k.data[[i, c]]).collect();
            if w.iter().all(|v| v.is_finite()) {
                out[[r, c]] = w.iter().sum::<f64>() / d as f64;
            }
        }
    }
    Panel { dates: k.dates.clone(), symbols: k.symbols.clone(), data: out }
}

/// Per-column ±DI series (Wilder-smoothed +DM/−DM over smoothed TR).
fn di_cols(high: &Panel, low: &Panel, close: &Panel, c: usize, n: usize) -> (Vec<f64>, Vec<f64>) {
    let nrows = close.nrows();
    let mut pdi = vec![f64::NAN; nrows];
    let mut mdi = vec![f64::NAN; nrows];
    let Some(first) = col_start(close, c) else { return (pdi, mdi) };
    let mut pdm = vec![f64::NAN; nrows];
    let mut mdm = vec![f64::NAN; nrows];
    let mut tr = vec![f64::NAN; nrows];
    for r in (first + 1)..nrows {
        let up = high.data[[r, c]] - high.data[[r - 1, c]];
        let dn = low.data[[r - 1, c]] - low.data[[r, c]];
        pdm[r] = if up > dn && up > 0.0 { up } else { 0.0 };
        mdm[r] = if dn > up && dn > 0.0 { dn } else { 0.0 };
        let pc = close.data[[r - 1, c]];
        let (h, l) = (high.data[[r, c]], low.data[[r, c]]);
        tr[r] = (h - l).max((h - pc).abs()).max((l - pc).abs());
    }
    let sp = wilder_col(&pdm, first, n);
    let sm = wilder_col(&mdm, first, n);
    let st = wilder_col(&tr, first, n);
    for r in 0..nrows {
        if st[r].is_finite() && st[r] != 0.0 {
            pdi[r] = 100.0 * sp[r] / st[r];
            mdi[r] = 100.0 * sm[r] / st[r];
        }
    }
    (pdi, mdi)
}

fn di_panel(high: &Panel, low: &Panel, close: &Panel, n: usize, plus: bool) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let (pdi, mdi) = di_cols(high, low, close, c, n);
        let src = if plus { &pdi } else { &mdi };
        for r in 0..nrows {
            out[[r, c]] = src[r];
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

pub fn plus_di(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    di_panel(high, low, close, n, true)
}

pub fn minus_di(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    di_panel(high, low, close, n, false)
}

/// ADX: Wilder smoothing of `DX = 100*|+DI − −DI|/(+DI + −DI)`. DX is finite from
/// `first+n`; ADX seeds `n` rows later (first finite at `first+2n-1`).
pub fn adx(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let (pdi, mdi) = di_cols(high, low, close, c, n);
        let mut dx = vec![f64::NAN; nrows];
        for r in 0..nrows {
            let (p, m) = (pdi[r], mdi[r]);
            if p.is_finite() && m.is_finite() {
                let sum = p + m;
                dx[r] = if sum != 0.0 { 100.0 * (p - m).abs() / sum } else { 0.0 };
            }
        }
        // DX's first finite row is first+n; offset wilder so seed = mean of first
        // n DX values (placed at first+2n-1).
        if first + n >= 1 {
            let adxc = wilder_col(&dx, first + n - 1, n);
            for r in 0..nrows {
                out[[r, c]] = adxc[r];
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// On-Balance Volume: cumulative ±volume by sign of the close change (0 on flat).
/// Seeded at a column's listing row with that bar's volume (TA-Lib convention).
pub fn obv(close: &Panel, volume: &Panel) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let mut acc = volume.data[[first, c]];
        out[[first, c]] = acc;
        for r in (first + 1)..nrows {
            let d = close.data[[r, c]] - close.data[[r - 1, c]];
            let v = volume.data[[r, c]];
            if d > 0.0 {
                acc += v;
            } else if d < 0.0 {
                acc -= v;
            }
            out[[r, c]] = acc;
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Money Flow Index over `n` periods. TP=(H+L+C)/3; raw money flow TP*V is summed
/// positive (TP rising) vs negative (TP falling) over the window; an all-positive
/// window yields 100. First finite at `first+n`.
pub fn mfi(high: &Panel, low: &Panel, close: &Panel, volume: &Panel, n: usize) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let tp: Vec<f64> = (0..nrows)
            .map(|r| (high.data[[r, c]] + low.data[[r, c]] + close.data[[r, c]]) / 3.0)
            .collect();
        let mut pos = vec![0.0; nrows];
        let mut neg = vec![0.0; nrows];
        for r in (first + 1)..nrows {
            let rmf = tp[r] * volume.data[[r, c]];
            if tp[r] > tp[r - 1] {
                pos[r] = rmf;
            } else if tp[r] < tp[r - 1] {
                neg[r] = rmf;
            }
        }
        for r in (first + n)..nrows {
            let lo = r + 1 - n;
            let p: f64 = (lo..=r).map(|k| pos[k]).sum();
            let ng: f64 = (lo..=r).map(|k| neg[k]).sum();
            out[[r, c]] = if ng == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + p / ng) };
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Volume-Weighted Average Price over `n` periods. Typical price TP=(H+L+C)/3;
/// VWAP = Σ(TP*V) / Σ(V) over the rolling window. First finite at first+n-1; a
/// window with any non-finite cell, or zero total volume, yields NaN.
pub fn vwap(high: &Panel, low: &Panel, close: &Panel, volume: &Panel, n: usize) -> Panel {
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    if n == 0 {
        return Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out };
    }
    for c in 0..ncols {
        let Some(first) = col_start(close, c) else { continue };
        let tp: Vec<f64> = (0..nrows)
            .map(|r| (high.data[[r, c]] + low.data[[r, c]] + close.data[[r, c]]) / 3.0)
            .collect();
        for r in (first + n - 1)..nrows {
            let lo = r + 1 - n;
            let (mut num, mut den, mut ok) = (0.0, 0.0, true);
            for k in lo..=r {
                let v = volume.data[[k, c]];
                if !tp[k].is_finite() || !v.is_finite() {
                    ok = false;
                    break;
                }
                num += tp[k] * v;
                den += v;
            }
            if ok && den != 0.0 {
                out[[r, c]] = num / den;
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

/// Williams %R: `-100 * (HHₙ - C) / (HHₙ - LLₙ)`.
pub fn willr(high: &Panel, low: &Panel, close: &Panel, n: usize) -> Panel {
    let hh = high.rolling_max(n);
    let ll = low.rolling_min(n);
    let (nrows, ncols) = close.data.dim();
    let mut out = Array2::from_elem((nrows, ncols), f64::NAN);
    for c in 0..ncols {
        for r in 0..nrows {
            let (h, l, cl) = (hh.data[[r, c]], ll.data[[r, c]], close.data[[r, c]]);
            if h.is_finite() && l.is_finite() && cl.is_finite() && h != l {
                out[[r, c]] = -100.0 * (h - cl) / (h - l);
            }
        }
    }
    Panel { dates: close.dates.clone(), symbols: close.symbols.clone(), data: out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Panel;

    /// Standard H/L/C fixture reused by ATR/NATR/WillR/Stoch/CCI tests.
    /// rows: H 10,11,12,11,13,12 · L 8,9,10,9,10,11 · C 9,10,11,10,12,11.
    fn hlc() -> (Panel, Panel, Panel) {
        let d: Vec<i32> = (0..6).map(|i| 20240102 + i).collect();
        let col = |v: [f64; 6]| {
            Panel::from_rows(d.clone(), vec!["A".into()], v.iter().map(|x| vec![*x]).collect())
                .unwrap()
        };
        (
            col([10.0, 11.0, 12.0, 11.0, 13.0, 12.0]),
            col([8.0, 9.0, 10.0, 9.0, 10.0, 11.0]),
            col([9.0, 10.0, 11.0, 10.0, 12.0, 11.0]),
        )
    }

    #[test]
    fn atr_matches_wilder_true_range() {
        // TR(r>=1) = 2,2,2,3,1. ATR(3): seed@row3 = mean(2,2,2)=2.0;
        // row4 = (2*2+3)/3 = 7/3; row5 = (7/3*2+1)/3 = 17/9.
        let (h, l, c) = hlc();
        let a = atr(&h, &l, &c, 3);
        for r in 0..3 {
            assert!(a.data[[r, 0]].is_nan());
        }
        assert!((a.data[[3, 0]] - 2.0).abs() < 1e-9);
        assert!((a.data[[4, 0]] - 7.0 / 3.0).abs() < 1e-9);
        assert!((a.data[[5, 0]] - 17.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn natr_is_atr_over_close_pct() {
        // NATR(3) = 100*ATR/close: row3 = 100*2/10 = 20; row4 = 100*(7/3)/12.
        let (h, l, c) = hlc();
        let v = natr(&h, &l, &c, 3);
        assert!((v.data[[3, 0]] - 20.0).abs() < 1e-9);
        assert!((v.data[[4, 0]] - 100.0 * (7.0 / 3.0) / 12.0).abs() < 1e-9);
    }

    #[test]
    fn cci_matches_definition() {
        // TP = (H+L+C)/3 = 9,10,11,10,11.6667,11.3333. CCI(3):
        // row2=100, row3=-50, row4=87.5, row5=33.333.
        let (h, l, c) = hlc();
        let v = cci(&h, &l, &c, 3);
        assert!(v.data[[1, 0]].is_nan());
        assert!((v.data[[2, 0]] - 100.0).abs() < 1e-6);
        assert!((v.data[[3, 0]] + 50.0).abs() < 1e-6);
        assert!((v.data[[4, 0]] - 87.5).abs() < 1e-6);
        assert!((v.data[[5, 0]] - 100.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn aroon_up_down_match_definition() {
        // H 10,11,12,11,13,12 · L 8,9,7,9,10,11; n=3 (window of n+1=4 bars).
        // Up:   row3=66.667, row4=100, row5=66.667.
        // Down: row3=66.667, row4=33.333, row5=0.
        let d: Vec<i32> = (0..6).map(|i| 20240102 + i).collect();
        let col = |v: [f64; 6]| {
            Panel::from_rows(d.clone(), vec!["A".into()], v.iter().map(|x| vec![*x]).collect())
                .unwrap()
        };
        let high = col([10.0, 11.0, 12.0, 11.0, 13.0, 12.0]);
        let low = col([8.0, 9.0, 7.0, 9.0, 10.0, 11.0]);
        let up = aroon_up(&high, 3);
        let down = aroon_down(&low, 3);
        assert!(up.data[[2, 0]].is_nan());
        assert!((up.data[[3, 0]] - 200.0 / 3.0).abs() < 1e-9);
        assert!((up.data[[4, 0]] - 100.0).abs() < 1e-9);
        assert!((up.data[[5, 0]] - 200.0 / 3.0).abs() < 1e-9);
        assert!((down.data[[3, 0]] - 200.0 / 3.0).abs() < 1e-9);
        assert!((down.data[[4, 0]] - 100.0 / 3.0).abs() < 1e-9);
        assert!((down.data[[5, 0]] - 0.0).abs() < 1e-9);
    }

    #[test]
    fn stoch_k_and_d_match_definition() {
        // %K(3) = 100*(C-LL)/(HH-LL) over rows r-2..=r:
        //   row2 (rows0-2: LL=8,HH=12,C=11) = 100*3/4 = 75
        //   row3 (rows1-3: LL=9,HH=12,C=10) = 100*1/3 = 100/3
        //   row4 (rows2-4: LL=9,HH=13,C=12) = 100*3/4 = 75
        //   row5 (rows3-5: LL=9,HH=13,C=11) = 100*2/4 = 50
        // %D = SMA(%K,3): row4 = mean(75,100/3,75) = 550/9; row5 = mean(100/3,75,50) = 475/9.
        let (h, l, c) = hlc();
        let k = stoch_k(&h, &l, &c, 3);
        assert!(k.data[[1, 0]].is_nan());
        assert!((k.data[[2, 0]] - 75.0).abs() < 1e-9);
        assert!((k.data[[3, 0]] - 100.0 / 3.0).abs() < 1e-9);
        assert!((k.data[[5, 0]] - 50.0).abs() < 1e-9);
        let dd = stoch_d(&h, &l, &c, 3, 3);
        assert!(dd.data[[3, 0]].is_nan());
        assert!((dd.data[[4, 0]] - 550.0 / 9.0).abs() < 1e-9);
        assert!((dd.data[[5, 0]] - 475.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn adx_di_on_pure_trends() {
        // Col A pure uptrend, col B pure downtrend, n=2. Every bar: |DM|=1, TR=1.5.
        // +DI(A)=100*1/1.5=66.667, -DI(A)=0; DX=100; ADX seeds row3 = 100.
        // Col B mirrors: -DI=66.667, +DI=0.
        let d: Vec<i32> = (0..6).map(|i| 20240102 + i).collect();
        let mk = |a: [f64; 6], b: [f64; 6]| {
            Panel::from_rows(
                d.clone(),
                vec!["A".into(), "B".into()],
                (0..6).map(|i| vec![a[i], b[i]]).collect(),
            )
            .unwrap()
        };
        let high = mk([10.0, 11.0, 12.0, 13.0, 14.0, 15.0], [15.0, 14.0, 13.0, 12.0, 11.0, 10.0]);
        let low = mk([9.0, 10.0, 11.0, 12.0, 13.0, 14.0], [14.0, 13.0, 12.0, 11.0, 10.0, 9.0]);
        let close =
            mk([9.5, 10.5, 11.5, 12.5, 13.5, 14.5], [14.5, 13.5, 12.5, 11.5, 10.5, 9.5]);

        let pdi = plus_di(&high, &low, &close, 2);
        let mdi = minus_di(&high, &low, &close, 2);
        let adxp = adx(&high, &low, &close, 2);

        assert!((pdi.data[[2, 0]] - 200.0 / 3.0).abs() < 1e-9);
        assert!((mdi.data[[2, 0]] - 0.0).abs() < 1e-9);
        assert!((pdi.data[[2, 1]] - 0.0).abs() < 1e-9);
        assert!((mdi.data[[2, 1]] - 200.0 / 3.0).abs() < 1e-9);
        assert!(adxp.data[[2, 0]].is_nan());
        assert!((adxp.data[[3, 0]] - 100.0).abs() < 1e-9);
        assert!((adxp.data[[5, 1]] - 100.0).abs() < 1e-9);
    }

    #[test]
    fn obv_accumulates_signed_volume() {
        // C 9,10,10,9,11 · V 100,200,150,120,300 → OBV 100,300,300,180,480.
        let d: Vec<i32> = (0..5).map(|i| 20240102 + i).collect();
        let close = Panel::from_rows(
            d.clone(),
            vec!["A".into()],
            vec![vec![9.0], vec![10.0], vec![10.0], vec![9.0], vec![11.0]],
        )
        .unwrap();
        let vol = Panel::from_rows(
            d,
            vec!["A".into()],
            vec![vec![100.0], vec![200.0], vec![150.0], vec![120.0], vec![300.0]],
        )
        .unwrap();
        let o = obv(&close, &vol);
        assert_eq!(o.data[[0, 0]], 100.0);
        assert_eq!(o.data[[1, 0]], 300.0);
        assert_eq!(o.data[[2, 0]], 300.0);
        assert_eq!(o.data[[3, 0]], 180.0);
        assert_eq!(o.data[[4, 0]], 480.0);
    }

    #[test]
    fn mfi_matches_definition() {
        // TP 9,11,10,12,11 · V 100,110,120,130,140; RMF 900,1210,1200,1560,1540.
        // MFI(3): row3 = 100-100/(1+2770/1200) = 69.7733; row4 = 36.2789.
        let d: Vec<i32> = (0..5).map(|i| 20240102 + i).collect();
        let col = |v: [f64; 5]| {
            Panel::from_rows(d.clone(), vec!["A".into()], v.iter().map(|x| vec![*x]).collect())
                .unwrap()
        };
        let high = col([10.0, 12.0, 11.0, 13.0, 12.0]);
        let low = col([8.0, 10.0, 9.0, 11.0, 10.0]);
        let close = col([9.0, 11.0, 10.0, 12.0, 11.0]);
        let vol = col([100.0, 110.0, 120.0, 130.0, 140.0]);
        let m = mfi(&high, &low, &close, &vol, 3);
        assert!(m.data[[2, 0]].is_nan());
        assert!((m.data[[3, 0]] - (100.0 - 100.0 / (1.0 + 2770.0 / 1200.0))).abs() < 1e-6);
        assert!((m.data[[4, 0]] - (100.0 - 100.0 / (1.0 + 1560.0 / 2740.0))).abs() < 1e-6);
    }

    #[test]
    fn willr_matches_definition() {
        // %R(3) = -100*(HH-C)/(HH-LL), window = rolling 3 bars ending at each row.
        // row2: HH=max(10,11,12)=12, LL=min(8,9,10)=8, C=11  => -100*(12-11)/(12-8)  = -25.
        // row3: HH=max(11,12,11)=12, LL=min(9,10, 9)=9, C=10  => -100*(12-10)/(12-9)  = -200/3.
        // row4: HH=max(12,11,13)=13, LL=min(10,9,10)=9, C=12  => -100*(13-12)/(13-9)  = -25.
        // row5: HH=max(11,13,12)=13, LL=min( 9,10,11)=9, C=11 => -100*(13-11)/(13-9)  = -50.
        // (Note: brief listed row3=-50; correct derivation gives -200/3 ≈ -66.667.)
        let (h, l, c) = hlc();
        let w = willr(&h, &l, &c, 3);
        assert!(w.data[[1, 0]].is_nan());
        assert!((w.data[[2, 0]] + 25.0).abs() < 1e-9);
        assert!((w.data[[3, 0]] - (-200.0 / 3.0)).abs() < 1e-9);
        assert!((w.data[[4, 0]] + 25.0).abs() < 1e-9);
        assert!((w.data[[5, 0]] + 50.0).abs() < 1e-9);
    }

    #[test]
    fn vwap_matches_rolling_volume_weighted_typical() {
        // TP = (H+L+C)/3 = 9,10,11,10,11.6667,11.3333 (reuses hlc()).
        // Volume 100,200,300,400,500,600; n=3.
        // row2 window k0..2: num = 9*100+10*200+11*300 = 6200; den = 600 -> 10.3333.
        let (h, l, c) = hlc();
        let vol = Panel::from_rows(
            (0..6).map(|i| 20240102 + i).collect(),
            vec!["A".into()],
            vec![vec![100.0], vec![200.0], vec![300.0], vec![400.0], vec![500.0], vec![600.0]],
        )
        .unwrap();
        let v = vwap(&h, &l, &c, &vol, 3);
        assert!(v.data[[1, 0]].is_nan()); // warm-up
        assert!((v.data[[2, 0]] - 6200.0 / 600.0).abs() < 1e-9);
    }
}
