use crate::csv_io::{parse_series, Field};
use crate::error::DataError;
use crate::source::ObjectSource;
use ndarray::Array2;
use std::collections::{BTreeSet, HashMap};
use yuzu_core::panel::Panel;

/// Default object-key directory for per-symbol price files. Override at the entry
/// point (e.g. `YUZU_PRICES_DIR`) for a custom layout — the engine stays generic.
pub const PRICES_DIR: &str = "prices";

/// Read the per-symbol price file for each symbol (probing `.csv.gz`, then
/// `.parquet` when that feature is on, then `.csv`; format detected from
/// content), extract `field`, filter to `[from, to]` (inclusive, YYYYMMDD), and
/// assemble a Panel: rows = sorted union of all kept days, columns = `symbols` in
/// the given order. Missing cells (and symbols with no file) are NaN. `dir`
/// defaults to [`PRICES_DIR`] at call sites.
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
        let dir = std::env::temp_dir().join(format!("pomelo_data_loader_{tag}"));
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
    fn loads_plain_csv_files_alongside_gzip() {
        // A `.csv` (uncompressed) file must load via the same path (probed after
        // `.csv.gz`), so a mixed mirror works.
        let dir = std::env::temp_dir().join("pomelo_data_loader_plaincsv");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("prices")).unwrap();
        fs::write(
            dir.join("prices/AAPL.csv"),
            "day,adj_open,adj_high,adj_low,adj_close,volume\n\
             2024-01-02,9,11,9,10,100\n\
             2024-01-03,10,12,10,11,100\n",
        )
        .unwrap();
        let src = LocalSource::new(&dir);
        let syms = vec!["AAPL".to_string()];
        let close =
            load_panel(&src, &syms, Field::AdjClose, 20240102, 20240103, PRICES_DIR).unwrap();
        assert_eq!(close.dates, vec![20240102, 20240103]);
        assert_eq!(close.data[[0, 0]], 10.0);
        assert_eq!(close.data[[1, 0]], 11.0);
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn loads_parquet_files_through_the_same_path() {
        use arrow_array::{ArrayRef, Float64Array, Int32Array, RecordBatch};
        use arrow_schema::{DataType, Field as AField, Schema};
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;

        let dir = std::env::temp_dir().join("pomelo_data_loader_parquet");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("prices")).unwrap();

        let schema = Arc::new(Schema::new(vec![
            AField::new("day", DataType::Int32, false),
            AField::new("adj_close", DataType::Float64, true),
            AField::new("volume", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![20240102, 20240103])) as ArrayRef,
                Arc::new(Float64Array::from(vec![10.0, 11.0])) as ArrayRef,
                Arc::new(Float64Array::from(vec![100.0, 200.0])) as ArrayRef,
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut w = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();
        fs::write(dir.join("prices/NVDA.parquet"), buf).unwrap();

        let src = LocalSource::new(&dir);
        let syms = vec!["NVDA".to_string()];
        let close =
            load_panel(&src, &syms, Field::AdjClose, 20240102, 20240103, PRICES_DIR).unwrap();
        assert_eq!(close.dates, vec![20240102, 20240103]);
        assert_eq!(close.data[[0, 0]], 10.0);
        assert_eq!(close.data[[1, 0]], 11.0);
        // a different field comes from the same parquet file
        let vol = load_panel(&src, &syms, Field::Volume, 20240102, 20240103, PRICES_DIR).unwrap();
        assert_eq!(vol.data[[1, 0]], 200.0);
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
