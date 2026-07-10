//! Delisting detection from consecutive missing prices.

use crate::panel::Panel;
use ndarray::Array2;

/// Delisting scan over the price panel. Returns `(dead, confirm)`, both
/// dates × symbols booleans: `confirm` is true on the row where a symbol's
/// NaN-price run first reaches `delist_after` (the forced-exit day); `dead` is
/// true from that row until prices resume. `None` when `delist_after == 0`.
pub(crate) fn scan_delistings(
    px: &Panel,
    delist_after: usize,
) -> Option<(Array2<bool>, Array2<bool>)> {
    if delist_after == 0 {
        return None;
    }
    let (nrows, n) = (px.nrows(), px.ncols());
    let mut dead = Array2::from_elem((nrows, n), false);
    let mut confirm = Array2::from_elem((nrows, n), false);
    for c in 0..n {
        let mut nan_run = 0usize;
        for r in 0..nrows {
            if px.data[[r, c]].is_nan() {
                nan_run += 1;
                if nan_run == delist_after {
                    confirm[[r, c]] = true;
                }
                if nan_run >= delist_after {
                    dead[[r, c]] = true;
                }
            } else {
                nan_run = 0;
            }
        }
    }
    Some((dead, confirm))
}
