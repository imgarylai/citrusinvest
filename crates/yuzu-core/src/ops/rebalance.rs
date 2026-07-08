//! `rebalance`: downsample rows to the last observation within each period
//! (weekly / month-end / quarter-end) or reindex onto an explicit date list.
//! Equivalent to `resample(freq).last()`.

use crate::panel::Panel;
use chrono::{Datelike, NaiveDate};

#[derive(Clone, Copy)]
pub enum Freq {
    Weekly,
    MonthEnd,
    QuarterEnd,
    YearEnd,
}

fn to_naive(yyyymmdd: i32) -> NaiveDate {
    let y = yyyymmdd / 10000;
    let m = (yyyymmdd / 100 % 100) as u32;
    let d = (yyyymmdd % 100) as u32;
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn period_key(d: i32, freq: Freq) -> i64 {
    let nd = to_naive(d);
    match freq {
        Freq::Weekly => {
            let iso = nd.iso_week();
            iso.year() as i64 * 100 + iso.week() as i64
        }
        Freq::MonthEnd => nd.year() as i64 * 100 + nd.month() as i64,
        Freq::QuarterEnd => nd.year() as i64 * 10 + ((nd.month() as i64 - 1) / 3),
        Freq::YearEnd => nd.year() as i64,
    }
}

impl Panel {
    pub fn rebalance_freq(&self, freq: Freq) -> Panel {
        // keep the row with the max date within each period (last obs)
        let mut last_in_period: std::collections::HashMap<i64, usize> =
            std::collections::HashMap::new();
        for (i, &d) in self.dates.iter().enumerate() {
            let k = period_key(d, freq);
            let e = last_in_period.entry(k).or_insert(i);
            if self.dates[*e] < d {
                *e = i;
            }
        }
        let mut kept: Vec<usize> = last_in_period.into_values().collect();
        kept.sort_unstable();
        self.select_rows(&kept)
    }

    pub fn rebalance_dates(&self, dates: &[i32]) -> Panel {
        let pos: std::collections::HashMap<i32, usize> = self
            .dates
            .iter()
            .enumerate()
            .map(|(i, d)| (*d, i))
            .collect();
        let mut out = ndarray::Array2::from_elem((dates.len(), self.ncols()), f64::NAN);
        for (r, d) in dates.iter().enumerate() {
            if let Some(&sr) = pos.get(d) {
                out.row_mut(r).assign(&self.data.row(sr));
            }
        }
        Panel {
            dates: dates.to_vec(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }

    fn select_rows(&self, rows: &[usize]) -> Panel {
        let mut out = ndarray::Array2::from_elem((rows.len(), self.ncols()), f64::NAN);
        let mut dates = Vec::with_capacity(rows.len());
        for (r, &src) in rows.iter().enumerate() {
            out.row_mut(r).assign(&self.data.row(src));
            dates.push(self.dates[src]);
        }
        Panel {
            dates,
            symbols: self.symbols.clone(),
            data: out,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::panel::Panel;

    #[test]
    fn rebalance_dates_reindexes_and_nans_missing() {
        let p = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![2.0], vec![3.0]],
        )
        .unwrap();
        // keep 0103, request a missing date 0105
        let r = p.rebalance_dates(&[20240103, 20240105]);
        assert_eq!(r.dates, vec![20240103, 20240105]);
        assert_eq!(r.data[[0, 0]], 2.0);
        assert!(r.data[[1, 0]].is_nan()); // 0105 absent -> NaN
    }
}
