//! The [`Panel`] type: a dense `f64` matrix indexed by `dates` (rows, `YYYYMMDD`)
//! and `symbols` (columns). `NaN` = missing; booleans are `1.0`/`0.0` with `NaN`
//! falsy (see [`is_true`]).
//!
//! Construction validates shape: `data.nrows() == dates.len()` and
//! `data.ncols() == symbols.len()`, otherwise [`EngineError::ShapeMismatch`].

use crate::error::EngineError;
use ndarray::{Array2, Axis};

/// Dense dates × symbols matrix of `f64` cells.
///
/// # Conventions
///
/// - **Dates** are civil calendar codes `YYYYMMDD` as `i32`, sorted ascending.
/// - **Missing** values are `f64::NAN` (never optional cells).
/// - **Boolean / position** panels use `1.0` / `0.0`; treat other finite values
///   as truthy only via ops that define their own threshold (see [`is_true`]
///   for the strict `== 1.0` form).
///
/// # Example
///
/// ```
/// use yuzu_core::panel::Panel;
///
/// let p = Panel::from_rows(
///     vec![20240102, 20240103],
///     vec!["AAPL".into(), "MSFT".into()],
///     vec![vec![100.0, 200.0], vec![101.0, 201.0]],
/// )
/// .unwrap();
/// assert_eq!(p.nrows(), 2);
/// assert_eq!(p.ncols(), 2);
/// assert_eq!(p.data[[0, 0]], 100.0);
/// ```
#[derive(Debug, Clone)]
pub struct Panel {
    pub dates: Vec<i32>,
    pub symbols: Vec<String>,
    pub data: Array2<f64>,
}

/// Strict boolean cell: only exact `1.0` is true (`NaN` and `0.0` are false).
pub fn is_true(x: f64) -> bool {
    x == 1.0
}

/// Encode a Rust `bool` as a panel cell (`1.0` / `0.0`).
pub fn bool_to_f64(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

impl Panel {
    /// Build a panel from an already-shaped [`Array2`].
    ///
    /// Returns [`EngineError::ShapeMismatch`] when the matrix dimensions do not
    /// match `dates` / `symbols`.
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

    /// Build a panel from row-major `Vec<Vec<f64>>` (one inner vec per date).
    ///
    /// Each row must have length `symbols.len()`; otherwise shape mismatch.
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

    /// The sub-panel whose dates fall in `[from, to]` (inclusive, `YYYYMMDD`).
    /// Symbols are unchanged; an empty range yields a zero-row panel. Dates are
    /// sorted ascending, so the kept rows are one contiguous block.
    pub fn slice_dates(&self, from: i32, to: i32) -> Panel {
        let start = self.dates.partition_point(|&d| d < from);
        let end = self.dates.partition_point(|&d| d <= to);
        Panel {
            dates: self.dates[start..end].to_vec(),
            symbols: self.symbols.clone(),
            data: self.data.slice(ndarray::s![start..end, ..]).to_owned(),
        }
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
    fn slice_dates_keeps_inclusive_range() {
        let p = Panel::new(
            vec![20240102, 20240103, 20240104, 20240105],
            vec!["A".into()],
            array![[1.0], [2.0], [3.0], [4.0]],
        )
        .unwrap();
        let s = p.slice_dates(20240103, 20240104);
        assert_eq!(s.dates, vec![20240103, 20240104]);
        assert_eq!(s.data, array![[2.0], [3.0]]);
        // bounds outside the range clamp; inverted range is empty
        assert_eq!(p.slice_dates(0, 99999999).nrows(), 4);
        assert_eq!(p.slice_dates(20240106, 20240110).nrows(), 0);
    }

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
