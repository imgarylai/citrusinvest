//! Binary elementwise ops (arithmetic, comparison, logical) — all [`align`] first —
//! plus unary `not`/scalar helpers. Arithmetic propagates `NaN`; comparisons with
//! `NaN` yield `false`.

use crate::align::align;
use crate::panel::{bool_to_f64, is_true, Panel};
use ndarray::Zip;

impl Panel {
    fn binary(&self, other: &Panel, f: impl Fn(f64, f64) -> f64) -> Panel {
        let (a, b) = align(self, other);
        let mut out = a.data.clone();
        Zip::from(&mut out)
            .and(&b.data)
            .for_each(|o, &y| *o = f(*o, y));
        Panel {
            dates: a.dates,
            symbols: a.symbols,
            data: out,
        }
    }

    pub fn add(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| x + y)
    }
    pub fn sub(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| x - y)
    }
    pub fn mul(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| x * y)
    }
    pub fn div(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| x / y)
    }

    pub fn gt(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x > y))
    }
    pub fn ge(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x >= y))
    }
    pub fn lt(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x < y))
    }
    pub fn le(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x <= y))
    }
    pub fn eq_p(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x == y))
    }
    pub fn ne_p(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(x != y))
    }

    pub fn and(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(is_true(x) && is_true(y)))
    }
    pub fn or(&self, o: &Panel) -> Panel {
        self.binary(o, |x, y| bool_to_f64(is_true(x) || is_true(y)))
    }

    pub fn not(&self) -> Panel {
        let data = self.data.mapv(|x| bool_to_f64(!is_true(x)));
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    pub fn scalar_gt(&self, v: f64) -> Panel {
        let data = self.data.mapv(|x| bool_to_f64(x > v));
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }
    pub fn scalar_mul(&self, v: f64) -> Panel {
        let data = self.data.mapv(|x| x * v);
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }
    pub fn neg(&self) -> Panel {
        self.scalar_mul(-1.0)
    }

    /// Aligned elementwise combinator exposed for the eval layer's generic
    /// scalar-broadcasting helper (panel `op` panel).
    pub fn ewise(&self, other: &Panel, f: impl Fn(f64, f64) -> f64) -> Panel {
        self.binary(other, f)
    }

    /// `cell <op> v` for every cell — panel on the left, scalar on the right.
    pub fn scalar_rhs(&self, v: f64, f: impl Fn(f64, f64) -> f64) -> Panel {
        let data = self.data.mapv(|x| f(x, v));
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// `v <op> cell` for every cell — scalar on the left, panel on the right
    /// (needed for non-commutative ops like `1 / pe`).
    pub fn scalar_lhs(&self, v: f64, f: impl Fn(f64, f64) -> f64) -> Panel {
        let data = self.data.mapv(|x| f(v, x));
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// Round each cell up to the nearest integer; `NaN` stays `NaN` (like
    /// `np.ceil`, used for qcut bucketing).
    pub fn ceil(&self) -> Panel {
        let data = self.data.mapv(|x| x.ceil());
        Panel {
            dates: self.dates.clone(),
            symbols: self.symbols.clone(),
            data,
        }
    }

    /// Keep cells of `self` where `by` is truthy (`1.0`), else `NaN` — a
    /// boolean mask. Aligns first; downstream cross-section ops ignore the `NaN`s.
    pub fn mask(&self, by: &Panel) -> Panel {
        let (a, b) = align(self, by);
        let mut out = a.data.clone();
        Zip::from(&mut out).and(&b.data).for_each(|o, &m| {
            if !is_true(m) {
                *o = f64::NAN;
            }
        });
        Panel {
            dates: a.dates,
            symbols: a.symbols,
            data: out,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::panel::Panel;

    fn p(rows: Vec<Vec<f64>>) -> Panel {
        let dates = (0..rows.len() as i32).map(|i| 20240102 + i).collect();
        Panel::from_rows(dates, vec!["A".into(), "B".into()], rows).unwrap()
    }

    #[test]
    fn gt_with_nan_is_false() {
        let a = p(vec![vec![2.0, f64::NAN]]);
        let b = p(vec![vec![1.0, 1.0]]);
        let r = a.gt(&b);
        assert_eq!(r.data[[0, 0]], 1.0);
        assert_eq!(r.data[[0, 1]], 0.0); // NaN > 1 => false
    }

    #[test]
    fn and_or_truthiness() {
        let a = p(vec![vec![1.0, 0.0]]);
        let b = p(vec![vec![1.0, 1.0]]);
        assert_eq!(a.and(&b).data[[0, 0]], 1.0);
        assert_eq!(a.and(&b).data[[0, 1]], 0.0);
        assert_eq!(a.or(&b).data[[0, 1]], 1.0);
    }

    #[test]
    fn sub_propagates_nan() {
        let a = p(vec![vec![5.0, f64::NAN]]);
        let b = p(vec![vec![2.0, 2.0]]);
        let r = a.sub(&b);
        assert_eq!(r.data[[0, 0]], 3.0);
        assert!(r.data[[0, 1]].is_nan());
    }

    #[test]
    fn add_mul_div() {
        let a = p(vec![vec![6.0, 8.0]]);
        let b = p(vec![vec![2.0, 4.0]]);
        assert_eq!(a.add(&b).data[[0, 0]], 8.0);
        assert_eq!(a.mul(&b).data[[0, 1]], 32.0);
        assert_eq!(a.div(&b).data[[0, 0]], 3.0);
    }

    #[test]
    fn remaining_comparisons() {
        let a = p(vec![vec![2.0, 2.0]]);
        let b = p(vec![vec![1.0, 2.0]]);
        assert_eq!(a.ge(&b).data[[0, 0]], 1.0);
        assert_eq!(a.ge(&b).data[[0, 1]], 1.0);
        assert_eq!(a.le(&b).data[[0, 0]], 0.0);
        assert_eq!(a.le(&b).data[[0, 1]], 1.0);
        assert_eq!(a.eq_p(&b).data[[0, 1]], 1.0);
        assert_eq!(a.eq_p(&b).data[[0, 0]], 0.0);
        assert_eq!(a.ne_p(&b).data[[0, 0]], 1.0);
    }

    #[test]
    fn not_treats_nan_as_false() {
        let a = p(vec![vec![1.0, f64::NAN]]);
        let r = a.not();
        assert_eq!(r.data[[0, 0]], 0.0); // !true
        assert_eq!(r.data[[0, 1]], 1.0); // !falsy(NaN)
    }

    #[test]
    fn scalar_ops_and_neg() {
        let a = p(vec![vec![3.0, -2.0]]);
        assert_eq!(a.scalar_gt(0.0).data[[0, 0]], 1.0);
        assert_eq!(a.scalar_gt(0.0).data[[0, 1]], 0.0);
        assert_eq!(a.scalar_mul(2.0).data[[0, 0]], 6.0);
        assert_eq!(a.neg().data[[0, 1]], 2.0);
    }

    #[test]
    fn scalar_lhs_and_rhs() {
        let a = p(vec![vec![2.0, 4.0]]);
        // panel op scalar: cell - 1
        assert_eq!(a.scalar_rhs(1.0, |x, y| x - y).data[[0, 0]], 1.0);
        // scalar op panel: 6 / cell  (non-commutative)
        assert_eq!(a.scalar_lhs(6.0, |x, y| x / y).data[[0, 1]], 1.5);
    }

    #[test]
    fn ceil_preserves_nan() {
        let a = p(vec![vec![1.2, f64::NAN]]);
        let r = a.ceil();
        assert_eq!(r.data[[0, 0]], 2.0);
        assert!(r.data[[0, 1]].is_nan());
    }

    #[test]
    fn mask_keeps_truthy_else_nan() {
        let v = p(vec![vec![3.0, 2.0]]);
        let m = p(vec![vec![1.0, 0.0]]);
        let r = v.mask(&m);
        assert_eq!(r.data[[0, 0]], 3.0);
        assert!(r.data[[0, 1]].is_nan());
    }
}
