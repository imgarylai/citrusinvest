use crate::csv_io::{parse_series, Field};
use crate::error::DataError;
use crate::source::ObjectSource;
use ndarray::Array2;
use std::collections::{BTreeSet, HashMap};
use yuzu_core::panel::Panel;

/// Default object-key directory for per-symbol price files. Override at the entry
/// point (e.g. `YUZU_PRICES_DIR`) for a custom layout — the engine stays generic.
pub const PRICES_DIR: &str = "prices";

/// Read `{dir}/{symbol}.csv.gz` for each symbol, extract `field`, filter to
/// `[from, to]` (inclusive, YYYYMMDD), and assemble a Panel: rows = sorted union
/// of all kept days, columns = `symbols` in the given order. Missing cells (and
/// symbols with no file) are NaN. `dir` defaults to [`PRICES_DIR`] at call sites.
pub fn load_panel<S: ObjectSource + Sync>(
    source: &S,
    symbols: &[String],
    field: Field,
    from: i32,
    to: i32,
    dir: &str,
) -> Result<Panel, DataError> {
    // Fetch + parse every symbol concurrently (network-bound). A corrupt file is
    // treated as missing (its column stays NaN) rather than sinking the batch.
    let per_symbol =
        crate::parallel::fetch_series(source, symbols, dir, from, to, |b| parse_series(b, field))?;

    let mut date_set: BTreeSet<i32> = BTreeSet::new();
    for map in &per_symbol {
        date_set.extend(map.keys().copied());
    }
    let dates: Vec<i32> = date_set.into_iter().collect();
    let row_of: HashMap<i32, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut data = Array2::from_elem((dates.len(), symbols.len()), f64::NAN);
    for (c, map) in per_symbol.iter().enumerate() {
        for (d, v) in map {
            data[[row_of[d], c]] = *v;
        }
    }

    Panel::new(dates, symbols.to_vec(), data).map_err(|e| DataError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csv_io::{write_series, OhlcvRow};
    use crate::source::LocalSource;
    use std::fs;

    fn r(day: i32, c: f64) -> OhlcvRow {
        OhlcvRow {
            day,
            adj_open: c,
            adj_high: c + 1.0,
            adj_low: c - 1.0,
            adj_close: c,
            volume: 100.0,
        }
    }

    fn fixture_dir(tag: &str) -> std::path::PathBuf {
        // per-test dir (tests run in parallel) — a shared name races on remove/write.
        let dir = std::env::temp_dir().join(format!("yuzu_data_loader_{tag}"));
        // Clean up any existing files
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("prices")).unwrap();
        fs::write(
            dir.join("prices/AAPL.csv.gz"),
            write_series(&[r(20240102, 10.0), r(20240103, 11.0), r(20240104, 12.0)]).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("prices/MSFT.csv.gz"),
            write_series(&[r(20240103, 50.0), r(20240105, 52.0)]).unwrap(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn loads_chosen_field_with_union_dates_and_nan() {
        let dir = fixture_dir("union");
        let src = LocalSource::new(&dir);
        let syms = vec!["AAPL".to_string(), "MSFT".to_string(), "ZZZ".to_string()];

        let close =
            load_panel(&src, &syms, Field::AdjClose, 20240102, 20240104, PRICES_DIR).unwrap();
        assert_eq!(close.dates, vec![20240102, 20240103, 20240104]);
        assert_eq!(close.data[[0, 0]], 10.0); // AAPL close
        assert!(close.data[[0, 1]].is_nan()); // MSFT absent on 0102
        assert_eq!(close.data[[1, 1]], 50.0); // MSFT close on 0103
        assert!(close.data[[2, 2]].is_nan()); // ZZZ no file

        // a different field comes from the same files
        let high = load_panel(&src, &syms, Field::AdjHigh, 20240102, 20240104, PRICES_DIR).unwrap();
        assert_eq!(high.data[[0, 0]], 11.0); // AAPL high = close + 1
    }

    #[test]
    fn skips_a_corrupt_file_instead_of_failing_the_batch() {
        let dir = fixture_dir("corrupt");
        // A truncated/non-gzip file (e.g. a half-finished R2 sync) must not sink the batch.
        fs::write(dir.join("prices/BAD.csv.gz"), b"not gzip at all").unwrap();
        let src = LocalSource::new(&dir);
        let syms = vec!["AAPL".to_string(), "BAD".to_string()];

        let close =
            load_panel(&src, &syms, Field::AdjClose, 20240102, 20240104, PRICES_DIR).unwrap();
        assert_eq!(close.data[[0, 0]], 10.0); // AAPL still loads
        assert!(close.data[[0, 1]].is_nan()); // BAD column stays NaN — skipped, not an error
        assert!(close.data[[2, 1]].is_nan());
    }
}
