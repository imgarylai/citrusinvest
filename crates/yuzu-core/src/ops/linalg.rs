//! Pure-Rust ordinary least squares via normal equations. Kept LAPACK-free so
//! `yuzu-core` compiles to WASM. k (factors / dummies) is small in practice,
//! so the k×k solve is cheap and well-conditioned for full-rank designs.
//! ponytail: normal equations, fine for small full-rank k; swap to a pure-Rust
//! QR/SVD only if rank-deficient neutralizers ever matter.

use ndarray::Array2;

/// Least-squares `β` for `X β ≈ y`. `None` if `XᵀX` is singular.
pub fn solve_ols(x: &Array2<f64>, y: &[f64]) -> Option<Vec<f64>> {
    let (m, k) = x.dim();
    debug_assert_eq!(m, y.len());

    // Normal equations: A = XᵀX (k×k), b = Xᵀy (k).
    let mut a = vec![vec![0.0f64; k]; k];
    let mut b = vec![0.0f64; k];
    for i in 0..k {
        for j in 0..k {
            let mut s = 0.0;
            for r in 0..m {
                s += x[[r, i]] * x[[r, j]];
            }
            a[i][j] = s;
        }
        let mut s = 0.0;
        for r in 0..m {
            s += x[[r, i]] * y[r];
        }
        b[i] = s;
    }
    gaussian_solve(a, b)
}

/// Solve `A β = b` (A is k×k) by Gaussian elimination with partial pivoting.
fn gaussian_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let k = b.len();
    for col in 0..k {
        // partial pivot: largest |a[row][col]| at or below the diagonal
        let mut piv = col;
        for r in (col + 1)..k {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        if a[piv][col].abs() < 1e-12 {
            return None; // singular
        }
        a.swap(col, piv);
        b.swap(col, piv);
        // eliminate below
        for r in (col + 1)..k {
            let f = a[r][col] / a[col][col];
            for c in col..k {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    // back-substitution
    let mut beta = vec![0.0f64; k];
    for col in (0..k).rev() {
        let mut s = b[col];
        for c in (col + 1)..k {
            s -= a[col][c] * beta[c];
        }
        beta[col] = s / a[col][col];
    }
    Some(beta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn exact_fit_recovers_coefficients() {
        // y = 2 + 3*x1 ; design [1, x1] ; exact -> residual 0, beta = [2, 3].
        let x = array![[1.0, 1.0], [1.0, 2.0], [1.0, 3.0]];
        let y = [5.0, 8.0, 11.0];
        let b = solve_ols(&x, &y).unwrap();
        assert!((b[0] - 2.0).abs() < 1e-9, "intercept {}", b[0]);
        assert!((b[1] - 3.0).abs() < 1e-9, "slope {}", b[1]);
    }

    #[test]
    fn singular_design_returns_none() {
        // two identical columns -> XtX singular.
        let x = array![[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]];
        let y = [1.0, 2.0, 3.0];
        assert!(solve_ols(&x, &y).is_none());
    }
}
