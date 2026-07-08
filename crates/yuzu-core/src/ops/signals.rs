//! Boolean signal transforms: `is_entry`/`is_exit` (rising/falling edges via
//! `shift_false`), `sustain` (rolling-sum threshold), `exit_when` (entry/exit
//! forward-fill state machine).

use crate::panel::{bool_to_f64, is_true, Panel};
use ndarray::Array2;

impl Panel {
    pub fn shift_false(&self, n: usize) -> Panel {
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), 0.0);
        if n < nrows {
            out.slice_mut(ndarray::s![n.., ..])
                .assign(&self.data.slice(ndarray::s![..nrows - n, ..]));
        }
        // normalize to bool
        let data = out.mapv(|x| bool_to_f64(is_true(x)));
        Panel { dates: self.dates.clone(), symbols: self.symbols.clone(), data }
    }

    pub fn is_entry(&self) -> Panel {
        self.and(&self.shift_false(1).not())
    }

    pub fn is_exit(&self) -> Panel {
        self.not().and(&self.shift_false(1))
    }

    pub fn sustain(&self, nwindow: usize, nsatisfy: Option<usize>) -> Panel {
        let need = nsatisfy.unwrap_or(nwindow);
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), 0.0);
        for c in 0..ncols {
            for r in 0..nrows {
                if r + 1 < nwindow {
                    continue; // rolling window not full => NaN-sum < need => false
                }
                let mut sum = 0.0;
                for k in (r + 1 - nwindow)..=r {
                    if is_true(self.data[[k, c]]) {
                        sum += 1.0;
                    }
                }
                out[[r, c]] = bool_to_f64(sum >= need as f64);
            }
        }
        Panel { dates: self.dates.clone(), symbols: self.symbols.clone(), data: out }
    }

    pub fn exit_when(&self, exit: &Panel) -> Panel {
        let entry_sig = self.is_entry();
        let exit_sig = self.is_exit().or(exit);
        let (nrows, ncols) = self.data.dim();
        let mut out = Array2::from_elem((nrows, ncols), 0.0);
        for c in 0..ncols {
            let mut state = f64::NAN;
            for r in 0..nrows {
                if is_true(entry_sig.data[[r, c]]) {
                    state = 1.0;
                } else if is_true(exit_sig.data[[r, c]]) {
                    state = 0.0;
                }
                out[[r, c]] = bool_to_f64(state == 1.0);
            }
        }
        Panel { dates: self.dates.clone(), symbols: self.symbols.clone(), data: out }
    }
}
