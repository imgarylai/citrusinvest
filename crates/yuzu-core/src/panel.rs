//! The [`Panel`] type: a dense `f64` matrix indexed by `dates` (rows, `YYYYMMDD`)
//! and `symbols` (columns). `NaN` = missing; booleans are `1.0`/`0.0` with `NaN`
//! falsy (see [`is_true`]).

use crate::error::EngineError;
use ndarray::{Array2, Axis};

#[derive(Debug, Clone)]
pub struct Panel {
    pub dates: Vec<i32>,
    pub symbols: Vec<String>,
    pub data: Array2<f64>,
}

pub fn is_true(x: f64) -> bool {
    x == 1.0
}

pub fn bool_to_f64(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

impl Panel {
    pub fn new(
        dates: Vec<i32>,
        symbols: Vec<String>,
        data: Array2<f64>,
    ) -> Result<Panel, EngineError> {
        if data.nrows() != dates.len() || data.ncols() != symbols.len() {
            return Err(EngineError::ShapeMismatch {
                rows: dates.len(),
                cols: symbols.len(),
                data_len: data.len(),
            });
        }
        Ok(Panel {
            dates,
            symbols,
            data,
        })
    }

    pub fn from_rows(
        dates: Vec<i32>,
        symbols: Vec<String>,
        rows: Vec<Vec<f64>>,
    ) -> Result<Panel, EngineError> {
        let nrows = rows.len();
        let ncols = symbols.len();
        let flat: Vec<f64> = rows.into_iter().flatten().collect();
        let data = Array2::from_shape_vec((nrows, ncols), flat).map_err(|_| {
            EngineError::ShapeMismatch {
                rows: nrows,
                cols: ncols,
                data_len: 0,
            }
        })?;
        Panel::new(dates, symbols, data)
    }

    pub fn nrows(&self) -> usize {
        self.dates.len()
    }

    pub fn ncols(&self) -> usize {
        self.symbols.len()
    }

    pub fn shift(&self, n: usize) -> Panel {
        let mut out = Array2::from_elem(self.data.dim(), f64::NAN);
        if n < self.nrows() {
            out.slice_mut(ndarray::s![n.., ..])
                .assign(&self.data.slice(ndarray::s![..self.nrows() - n, ..]));
        }
        let _ = Axis(0); // keep import used if slicing form changes
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data: out,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn new_rejects_shape_mismatch() {
        let err = Panel::new(vec![20240102], vec!["A".into(), "B".into()], array![[1.0]]);
        assert!(err.is_err());
    }

    #[test]
    fn shift_pushes_rows_down_and_nans_the_top() {
        let p = Panel::from_rows(
            vec![20240102, 20240103, 20240104],
            vec!["A".into()],
            vec![vec![1.0], vec![2.0], vec![3.0]],
        )
        .unwrap();
        let s = p.shift(1);
        assert!(s.data[[0, 0]].is_nan());
        assert_eq!(s.data[[1, 0]], 1.0);
        assert_eq!(s.data[[2, 0]], 2.0);
        assert_eq!(s.dates, p.dates);
    }
}
