//! [`align`] reshapes two panels onto a shared index before an elementwise op:
//! rows = union of dates (sorted), columns = intersection of symbols (left's order).
//! Absent cells become `NaN` (standard reshape/align semantics).

use crate::panel::Panel;
use ndarray::Array2;
use std::collections::BTreeSet;

pub fn align(a: &Panel, b: &Panel) -> (Panel, Panel) {
    // union of dates, sorted ascending
    let mut date_set: BTreeSet<i32> = BTreeSet::new();
    date_set.extend(a.dates.iter().copied());
    date_set.extend(b.dates.iter().copied());
    let dates: Vec<i32> = date_set.into_iter().collect();

    // intersection of symbols, preserving a's order
    let bset: std::collections::HashSet<&String> = b.symbols.iter().collect();
    let symbols: Vec<String> = a
        .symbols
        .iter()
        .filter(|s| bset.contains(*s))
        .cloned()
        .collect();

    (project(a, &dates, &symbols), project(b, &dates, &symbols))
}

fn project(p: &Panel, dates: &[i32], symbols: &[String]) -> Panel {
    let row_idx: std::collections::HashMap<i32, usize> =
        p.dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let col_idx: std::collections::HashMap<&String, usize> =
        p.symbols.iter().enumerate().map(|(i, s)| (s, i)).collect();

    let mut data = Array2::from_elem((dates.len(), symbols.len()), f64::NAN);
    for (r, d) in dates.iter().enumerate() {
        let Some(&sr) = row_idx.get(d) else { continue };
        for (c, s) in symbols.iter().enumerate() {
            if let Some(&sc) = col_idx.get(s) {
                data[[r, c]] = p.data[[sr, sc]];
            }
        }
    }
    Panel {
        dates: dates.to_vec(),
        symbols: symbols.to_vec(),
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Panel;

    #[test]
    fn union_rows_intersect_cols() {
        let a = Panel::from_rows(
            vec![20240102, 20240103],
            vec!["A".into(), "B".into()],
            vec![vec![1.0, 2.0], vec![3.0, 4.0]],
        )
        .unwrap();
        let b = Panel::from_rows(
            vec![20240103, 20240104],
            vec!["B".into(), "C".into()],
            vec![vec![5.0, 6.0], vec![7.0, 8.0]],
        )
        .unwrap();

        let (ra, rb) = align(&a, &b);
        assert_eq!(ra.dates, vec![20240102, 20240103, 20240104]);
        assert_eq!(ra.symbols, vec!["B".to_string()]); // intersection
                                                       // a had B=2.0 at 0103; row 0102 present in a but not b
        assert_eq!(ra.data[[1, 0]], 4.0);
        assert!(ra.data[[2, 0]].is_nan()); // a missing 0104
        assert!(rb.data[[0, 0]].is_nan()); // b missing 0102
        assert_eq!(rb.data[[1, 0]], 5.0); // b B=5.0 at 0103
    }
}
