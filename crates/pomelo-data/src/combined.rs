//! Combined per-field panel files: one wide gzip CSV per series
//! (`panels/{name}.csv.gz`, header `day,SYM1,SYM2,…`, one row per trading day,
//! empty cell = NaN), holding the full universe. The container reads ONE object
//! per series instead of hundreds of per-symbol files. Same gzip-CSV conventions
//! as `csv_io`/`fundamentals`.

use crate::csv_io::{parse_series, Field};
use crate::date::{date_to_i32, i32_to_date};
use crate::error::DataError;
use crate::format::CANDIDATE_EXTS;
use crate::fundamentals::{parse_fundamentals, FUNDAMENTAL_FIELDS, REPORT_EVENT_FIELD};
use crate::source::{ObjectSink, ObjectSource};
use flate2::write::GzEncoder;
use flate2::Compression;
use ndarray::Array2;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use yuzu_core::panel::Panel;

/// Default object-key directory for combined per-field panel files.
pub const PANELS_DIR: &str = "panels";

/// Serialize a Panel to combined gzip CSV (`day,<sym…>`, empty cell for NaN).
pub fn write_combined_panel(panel: &Panel) -> Result<Vec<u8>, DataError> {
    let mut buf = String::from("day");
    for s in &panel.symbols {
        buf.push(',');
        buf.push_str(s);
    }
    buf.push('\n');
    for (r, day) in panel.dates.iter().enumerate() {
        buf.push_str(&i32_to_date(*day));
        for c in 0..panel.symbols.len() {
            buf.push(',');
            let v = panel.data[[r, c]];
            if !v.is_nan() {
                buf.push_str(&v.to_string());
            }
        }
        buf.push('\n');
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(buf.as_bytes())
        .map_err(|e| DataError::Io(e.to_string()))?;
    enc.finish().map_err(|e| DataError::Io(e.to_string()))
}

/// Read the combined `{dir}/{name}` file (probing `.csv.gz`/`.parquet`/`.csv`)
/// and assemble a Panel for `symbols` (in the given order; a symbol absent from
/// the file gets a NaN column) over `[from, to]` inclusive. The format is
/// detected from content. `Ok(None)` if the combined file does not exist in any
/// supported format.
pub fn load_combined_panel<S: ObjectSource>(
    source: &S,
    name: &str,
    symbols: &[String],
    from: i32,
    to: i32,
    dir: &str,
) -> Result<Option<Panel>, DataError> {
    let mut bytes = None;
    for ext in CANDIDATE_EXTS {
        if let Some(b) = source.get(&format!("{dir}/{name}{ext}"))? {
            bytes = Some(b);
            break;
        }
    }
    let Some(bytes) = bytes else {
        return Ok(None);
    };

    #[cfg(feature = "parquet")]
    if crate::format::Format::detect(&bytes) == crate::format::Format::Parquet {
        return load_combined_parquet(&bytes, symbols, from, to).map(Some);
    }

    let text = crate::format::read_csv_text(&bytes)?;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    // file column index (>=1; col 0 is "day") for every symbol in the file
    let file_cols: HashMap<&str, usize> = header
        .split(',')
        .enumerate()
        .skip(1)
        .map(|(i, s)| (s.trim(), i))
        .collect();
    // requested symbol → file column (None ⇒ NaN column)
    let col_of: Vec<Option<usize>> = symbols
        .iter()
        .map(|s| file_cols.get(s.as_str()).copied())
        .collect();

    let mut dates: Vec<i32> = Vec::new();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let day = date_to_i32(cells[0].trim())?;
        if day < from || day > to {
            continue;
        }
        let mut row = Vec::with_capacity(symbols.len());
        for col in &col_of {
            let v = match col {
                Some(c) if *c < cells.len() => {
                    let cell = cells[*c].trim();
                    if cell.is_empty() {
                        f64::NAN
                    } else {
                        cell.parse()
                            .map_err(|_| DataError::Parse(format!("bad value '{cell}'")))?
                    }
                }
                _ => f64::NAN,
            };
            row.push(v);
        }
        dates.push(day);
        rows.push(row);
    }
    let panel = Panel::from_rows(dates, symbols.to_vec(), rows)
        .map_err(|e| DataError::Parse(e.to_string()))?;
    Ok(Some(panel))
}

/// Assemble a Panel from a wide combined-panel Parquet buffer: one column per
/// symbol (absent symbol ⇒ NaN column), windowed to `[from, to]` inclusive.
#[cfg(feature = "parquet")]
fn load_combined_parquet(
    bytes: &[u8],
    symbols: &[String],
    from: i32,
    to: i32,
) -> Result<Panel, DataError> {
    let (all_dates, all_rows) = crate::parquet_io::read_wide(bytes, symbols)?;
    let mut dates = Vec::new();
    let mut rows = Vec::new();
    for (day, row) in all_dates.into_iter().zip(all_rows) {
        if day >= from && day <= to {
            dates.push(day);
            rows.push(row);
        }
    }
    Panel::from_rows(dates, symbols.to_vec(), rows).map_err(|e| DataError::Parse(e.to_string()))
}

/// What a rebuild wrote: number of series files and the max day-count across them.
pub struct RebuildSummary {
    pub fields: usize,
    pub days: usize,
}

/// Build `panels/{series}.csv.gz` for every OHLCV field + fundamental field from
/// the per-symbol archives. Reads each per-symbol file once (concurrently), then
/// transposes per field. Full overwrite, idempotent. A missing/corrupt per-symbol
/// file leaves NaN cells (same fail-soft as the per-symbol loaders).
pub fn rebuild_combined_panels<S: ObjectSource + ObjectSink + Sync>(
    source: &S,
    symbols: &[String],
    prices_dir: &str,
    fundamentals_dir: &str,
    panels_dir: &str,
) -> Result<RebuildSummary, DataError> {
    let mut fields = 0usize;
    let mut max_days = 0usize;

    // --- OHLCV: read every per-symbol price file once, transpose per field.
    // Probes .csv.gz/.parquet/.csv so a mirror in any supported format rebuilds. ---
    let price_bytes = crate::parallel::fetch_symbols(source, prices_dir, symbols)?;
    let price_series: &[(&str, Field)] = &[
        ("open", Field::AdjOpen),
        ("high", Field::AdjHigh),
        ("low", Field::AdjLow),
        ("close", Field::AdjClose),
        ("volume", Field::Volume),
    ];
    for (name, field) in price_series {
        let per_symbol: Vec<Vec<(i32, f64)>> = price_bytes
            .iter()
            .map(|b| match b {
                Some(bytes) => parse_series(bytes, *field).unwrap_or_default(),
                None => Vec::new(),
            })
            .collect();
        let panel = assemble(symbols, &per_symbol)?;
        max_days = max_days.max(panel.dates.len());
        source.put(
            &format!("{panels_dir}/{name}.csv.gz"),
            &write_combined_panel(&panel)?,
        )?;
        fields += 1;
    }

    drop(price_bytes); // free the price bytes before holding the fundamentals bytes (halves peak RAM)

    // --- Fundamentals: read every per-symbol fundamentals file once, transpose ---
    let fund_bytes = crate::parallel::fetch_symbols(source, fundamentals_dir, symbols)?;
    for name in FUNDAMENTAL_FIELDS
        .iter()
        .chain(std::iter::once(&REPORT_EVENT_FIELD))
    {
        let per_symbol: Vec<Vec<(i32, f64)>> = fund_bytes
            .iter()
            .map(|b| match b {
                Some(bytes) => parse_fundamentals(bytes, name).unwrap_or_default(),
                None => Vec::new(),
            })
            .collect();
        let panel = assemble(symbols, &per_symbol)?;
        max_days = max_days.max(panel.dates.len());
        source.put(
            &format!("{panels_dir}/{name}.csv.gz"),
            &write_combined_panel(&panel)?,
        )?;
        fields += 1;
    }

    Ok(RebuildSummary {
        fields,
        days: max_days,
    })
}

/// Union-dates assembly: rows = sorted union of all symbols' days, cols = symbols
/// in order, missing cells NaN. (Mirrors the per-symbol loaders' assembly.)
/// Public so producers (e.g. the CLI's FMP snapshot-factor writer) can build a
/// combined [`Panel`] from per-symbol `(day, value)` rows before serializing it
/// with [`write_combined_panel`].
pub fn assemble(symbols: &[String], per_symbol: &[Vec<(i32, f64)>]) -> Result<Panel, DataError> {
    let mut date_set: BTreeSet<i32> = BTreeSet::new();
    for rows in per_symbol {
        for (d, _) in rows {
            date_set.insert(*d);
        }
    }
    let dates: Vec<i32> = date_set.into_iter().collect();
    let row_of: HashMap<i32, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut data = Array2::from_elem((dates.len(), symbols.len()), f64::NAN);
    for (c, rows) in per_symbol.iter().enumerate() {
        for (d, v) in rows {
            data[[row_of[d], c]] = *v;
        }
    }
    Panel::new(dates, symbols.to_vec(), data).map_err(|e| DataError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::LocalSource;
    use ndarray::array;
    use std::fs;
    use yuzu_core::panel::Panel;

    #[test]
    fn rebuild_writes_combined_files_loadable_with_same_values() {
        use crate::csv_io::{write_series, OhlcvRow};
        use crate::fundamentals::{write_fundamentals, FundamentalRow, FUNDAMENTAL_FIELDS};
        use crate::source::LocalSource;
        use std::fs;

        let dir = std::env::temp_dir().join("yuzu_combined_rebuild");
        let _ = fs::remove_dir_all(&dir);
        for d in ["prices", "fundamentals", "panels"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        let ohlcv = |day, c| OhlcvRow {
            day,
            adj_open: c,
            adj_high: c,
            adj_low: c,
            adj_close: c,
            volume: 100.0,
        };
        fs::write(
            dir.join("prices/AAA.csv.gz"),
            write_series(&[ohlcv(20240102, 10.0), ohlcv(20240103, 11.0)]).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("prices/BBB.csv.gz"),
            write_series(&[ohlcv(20240103, 20.0)]).unwrap(),
        )
        .unwrap();
        let frow = |day, pe| {
            let mut values = vec![f64::NAN; FUNDAMENTAL_FIELDS.len()];
            values[0] = pe; // "pe" is column 0
            FundamentalRow {
                day,
                values,
                report_event: 0.0,
            }
        };
        fs::write(
            dir.join("fundamentals/AAA.csv.gz"),
            write_fundamentals(&[frow(20240102, 8.0), frow(20240103, 8.0)]).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("fundamentals/BBB.csv.gz"),
            write_fundamentals(&[frow(20240103, 15.0)]).unwrap(),
        )
        .unwrap();

        let src = LocalSource::new(&dir);
        let syms = vec!["AAA".to_string(), "BBB".to_string(), "CCC".to_string()];
        let summary =
            rebuild_combined_panels(&src, &syms, "prices", "fundamentals", "panels").unwrap();
        assert_eq!(summary.fields, 5 + FUNDAMENTAL_FIELDS.len() + 1); // OHLCV + factors + report_event
        assert_eq!(summary.days, 2);

        let close = load_combined_panel(&src, "close", &syms, 20240102, 20240103, "panels")
            .unwrap()
            .unwrap();
        assert_eq!(close.dates, vec![20240102, 20240103]);
        assert_eq!(close.data[[0, 0]], 10.0); // AAA 0102
        assert!(close.data[[0, 1]].is_nan()); // BBB absent 0102
        assert!(close.data[[0, 2]].is_nan()); // CCC absent (no file)
        assert!(close.data[[1, 2]].is_nan()); // CCC absent (no file)
        assert_eq!(close.data[[1, 1]], 20.0); // BBB 0103
        let pe = load_combined_panel(&src, "pe", &syms, 20240102, 20240103, "panels")
            .unwrap()
            .unwrap();
        assert_eq!(pe.data[[0, 0]], 8.0); // AAA pe
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn loads_a_wide_parquet_combined_panel() {
        use arrow_array::{ArrayRef, Float64Array, Int32Array, RecordBatch};
        use arrow_schema::{DataType, Field as AField, Schema};
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![
            AField::new("day", DataType::Int32, false),
            AField::new("AAA", DataType::Float64, true),
            AField::new("BBB", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int32Array::from(vec![20240102, 20240103, 20240104])) as ArrayRef,
                Arc::new(Float64Array::from(vec![Some(10.0), Some(11.0), Some(12.0)])) as ArrayRef,
                Arc::new(Float64Array::from(vec![None, Some(20.0), Some(21.0)])) as ArrayRef,
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        let mut w = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        w.write(&batch).unwrap();
        w.close().unwrap();

        let dir = std::env::temp_dir().join("yuzu_combined_parquet");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("panels")).unwrap();
        fs::write(dir.join("panels/close.parquet"), buf).unwrap();
        let src = LocalSource::new(&dir);

        // reorder + windowing + absent symbol, same contract as the CSV path
        let syms = vec!["BBB".to_string(), "AAA".to_string(), "ZZZ".to_string()];
        let p = load_combined_panel(&src, "close", &syms, 20240103, 20240104, PANELS_DIR)
            .unwrap()
            .unwrap();
        assert_eq!(p.dates, vec![20240103, 20240104]);
        assert_eq!(p.symbols, syms);
        assert_eq!(p.data[[0, 0]], 20.0); // BBB 0103
        assert_eq!(p.data[[0, 1]], 11.0); // AAA 0103
        assert!(p.data[[0, 2]].is_nan()); // ZZZ absent column
    }

    #[test]
    fn write_then_load_selects_subset_reorders_and_windows() {
        let data = array![
            [10.0, 20.0, f64::NAN],
            [11.0, 21.0, 31.0],
            [12.0, 22.0, 32.0],
        ];
        let panel = Panel::new(
            vec![20240102, 20240103, 20240104],
            vec!["AAA".into(), "BBB".into(), "CCC".into()],
            data,
        )
        .unwrap();
        let bytes = write_combined_panel(&panel).unwrap();

        let dir = std::env::temp_dir().join("yuzu_combined_rw");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("panels")).unwrap();
        fs::write(dir.join("panels/close.csv.gz"), bytes).unwrap();
        let src = LocalSource::new(&dir);

        // request CCC, AAA (reordered) + ZZZ (absent) over the inner window
        let syms = vec!["CCC".to_string(), "AAA".to_string(), "ZZZ".to_string()];
        let p = load_combined_panel(&src, "close", &syms, 20240103, 20240104, PANELS_DIR)
            .unwrap()
            .unwrap();
        assert_eq!(p.dates, vec![20240103, 20240104]);
        assert_eq!(p.symbols, syms);
        assert_eq!(p.data[[0, 0]], 31.0); // CCC 0103
        assert_eq!(p.data[[0, 1]], 11.0); // AAA 0103
        assert!(p.data[[0, 2]].is_nan()); // ZZZ absent from file
                                          // absent combined file → Ok(None) (caller falls back)
        assert!(
            load_combined_panel(&src, "missing", &syms, 0, 99999999, PANELS_DIR)
                .unwrap()
                .is_none()
        );
    }
}
