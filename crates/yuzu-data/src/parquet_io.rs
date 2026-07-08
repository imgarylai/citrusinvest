//! Apache Parquet reading (behind the `parquet` feature). Mirrors the CSV layout:
//! a `day` column (YYYYMMDD `int`, `YYYY-MM-DD` string, or logical `date`) plus
//! one value column per series (per-symbol files: `adj_close`, `pe`, …; combined
//! panels: one column per symbol). Values coerce to `f64`; nulls become `NaN`.
//!
//! Read-only: writing stays gzip CSV. Column extraction is by name, so a Parquet
//! file with extra columns or a different order still parses.

use crate::error::DataError;
use arrow_array::{Array, Float64Array, StringArray};
use arrow_cast::cast;
use arrow_schema::{DataType, SchemaRef};
use bytes::Bytes;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn err(e: impl std::fmt::Display) -> DataError {
    DataError::Parse(format!("parquet: {e}"))
}

/// Parse `YYYY-MM-DD` (dashes optional) into a packed `YYYYMMDD` i32.
fn date_to_i32(s: &str) -> Result<i32, DataError> {
    s.replace('-', "")
        .parse()
        .map_err(|_| DataError::Parse(format!("bad date '{s}'")))
}

/// A numeric column coerced to `f64`; nulls become `NaN`. The Arrow `cast` kernel
/// handles every integer/float width, so one path covers all numeric columns.
fn column_to_f64(arr: &dyn Array) -> Result<Vec<f64>, DataError> {
    let casted = cast(arr, &DataType::Float64).map_err(err)?;
    let a = casted
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| err("cast to f64 failed"))?;
    Ok((0..a.len())
        .map(|i| if a.is_null(i) { f64::NAN } else { a.value(i) })
        .collect())
}

/// A day column coerced to packed `YYYYMMDD`; `None` for a null day (row skipped).
/// Date/string columns are rendered to ISO text (`Date32` → `YYYY-MM-DD`) then
/// parsed; integer columns are already packed `YYYYMMDD`.
fn column_to_days(arr: &dyn Array) -> Result<Vec<Option<i32>>, DataError> {
    let is_textual = matches!(
        arr.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Date32 | DataType::Date64
    );
    if is_textual {
        let s = cast(arr, &DataType::Utf8).map_err(err)?;
        let a = s
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| err("cast day column to text failed"))?;
        (0..a.len())
            .map(|i| {
                if a.is_null(i) {
                    Ok(None)
                } else {
                    date_to_i32(a.value(i)).map(Some)
                }
            })
            .collect()
    } else {
        let c = cast(arr, &DataType::Float64).map_err(err)?;
        let a = c
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| err("cast day column to number failed"))?;
        Ok((0..a.len())
            .map(|i| (!a.is_null(i)).then(|| a.value(i) as i32))
            .collect())
    }
}

fn schema_index(schema: &SchemaRef, name: &str) -> Option<usize> {
    schema.index_of(name).ok()
}

/// Read `day` + one named value column from a Parquet buffer into `(day, value)`
/// rows (rows with a null day are dropped). The value column must exist.
pub(crate) fn read_series(bytes: &[u8], column: &str) -> Result<Vec<(i32, f64)>, DataError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec())).map_err(err)?;
    let schema = builder.schema().clone();
    let day_idx = schema_index(&schema, "day").ok_or_else(|| err("no 'day' column"))?;
    let val_idx =
        schema_index(&schema, column).ok_or_else(|| err(format!("no '{column}' column")))?;
    let reader = builder.build().map_err(err)?;

    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.map_err(err)?;
        let days = column_to_days(batch.column(day_idx))?;
        let vals = column_to_f64(batch.column(val_idx))?;
        for (d, v) in days.into_iter().zip(vals) {
            if let Some(day) = d {
                out.push((day, v));
            }
        }
    }
    Ok(out)
}

/// Read `day` + a value column per requested symbol from a wide combined-panel
/// Parquet buffer. Returns `(dates, rows)` where each row holds one `f64` per
/// symbol in `symbols` order; a symbol with no matching column is all-`NaN`.
/// Rows with a null day are dropped.
pub(crate) fn read_wide(
    bytes: &[u8],
    symbols: &[String],
) -> Result<(Vec<i32>, Vec<Vec<f64>>), DataError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(Bytes::from(bytes.to_vec())).map_err(err)?;
    let schema = builder.schema().clone();
    let day_idx = schema_index(&schema, "day").ok_or_else(|| err("no 'day' column"))?;
    let col_of: Vec<Option<usize>> = symbols.iter().map(|s| schema_index(&schema, s)).collect();
    let reader = builder.build().map_err(err)?;

    let mut dates = Vec::new();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for batch in reader {
        let batch = batch.map_err(err)?;
        let days = column_to_days(batch.column(day_idx))?;
        // Materialize each requested symbol column once (NaN column if absent).
        let cols: Vec<Vec<f64>> = col_of
            .iter()
            .map(|c| match c {
                Some(i) => column_to_f64(batch.column(*i)),
                None => Ok(vec![f64::NAN; batch.num_rows()]),
            })
            .collect::<Result<_, _>>()?;
        for (r, d) in days.into_iter().enumerate() {
            if let Some(day) = d {
                dates.push(day);
                rows.push(cols.iter().map(|col| col[r]).collect());
            }
        }
    }
    Ok((dates, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{ArrayRef, Float64Array, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn write_parquet(batch: &RecordBatch) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut w = ArrowWriter::try_new(&mut buf, batch.schema(), None).unwrap();
        w.write(batch).unwrap();
        w.close().unwrap();
        buf
    }

    #[test]
    fn reads_int_day_and_f64_series() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Int32, false),
            Field::new("adj_close", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![20240102, 20240103])) as ArrayRef,
                Arc::new(Float64Array::from(vec![Some(10.5), None])) as ArrayRef,
            ],
        )
        .unwrap();
        let bytes = write_parquet(&batch);
        let rows = read_series(&bytes, "adj_close").unwrap();
        assert_eq!(rows[0], (20240102, 10.5));
        assert_eq!(rows[1].0, 20240103);
        assert!(rows[1].1.is_nan()); // null -> NaN
        assert!(read_series(&bytes, "nope").is_err());
    }

    #[test]
    fn reads_string_day_column() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Utf8, false),
            Field::new("pe", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["2024-01-02", "2024-01-03"])) as ArrayRef,
                Arc::new(Float64Array::from(vec![8.0, 9.0])) as ArrayRef,
            ],
        )
        .unwrap();
        let rows = read_series(&write_parquet(&batch), "pe").unwrap();
        assert_eq!(rows, vec![(20240102, 8.0), (20240103, 9.0)]);
    }

    #[test]
    fn reads_date32_day_column() {
        // 2024-01-02 is 19724 days since the epoch.
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Date32, false),
            Field::new("adj_close", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(arrow_array::Date32Array::from(vec![19724])) as ArrayRef,
                Arc::new(Float64Array::from(vec![12.0])) as ArrayRef,
            ],
        )
        .unwrap();
        let rows = read_series(&write_parquet(&batch), "adj_close").unwrap();
        assert_eq!(rows, vec![(20240102, 12.0)]);
    }

    #[test]
    fn reads_wide_panel_with_missing_symbol_column() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Int32, false),
            Field::new("AAA", DataType::Float64, true),
            Field::new("BBB", DataType::Float64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![20240102, 20240103])) as ArrayRef,
                Arc::new(Float64Array::from(vec![Some(1.0), Some(2.0)])) as ArrayRef,
                Arc::new(Float64Array::from(vec![None, Some(3.0)])) as ArrayRef,
            ],
        )
        .unwrap();
        let bytes = write_parquet(&batch);
        let syms = vec!["BBB".to_string(), "AAA".to_string(), "ZZZ".to_string()];
        let (dates, rows) = read_wide(&bytes, &syms).unwrap();
        assert_eq!(dates, vec![20240102, 20240103]);
        assert!(rows[0][0].is_nan()); // BBB null on 0102
        assert_eq!(rows[0][1], 1.0); // AAA 0102
        assert!(rows[0][2].is_nan()); // ZZZ absent column
        assert_eq!(rows[1][0], 3.0); // BBB 0103
    }

    #[test]
    fn int64_day_and_null_day_row_is_dropped() {
        use arrow_array::Int64Array;
        let schema = Arc::new(Schema::new(vec![
            Field::new("day", DataType::Int64, true),
            Field::new("adj_close", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![Some(20240102), None, Some(20240104)])) as ArrayRef,
                Arc::new(Float64Array::from(vec![10.0, 11.0, 12.0])) as ArrayRef,
            ],
        )
        .unwrap();
        // the null-day row is dropped; the surrounding rows survive
        let rows = read_series(&write_parquet(&batch), "adj_close").unwrap();
        assert_eq!(rows, vec![(20240102, 10.0), (20240104, 12.0)]);
    }

    #[test]
    fn missing_day_column_and_bad_bytes_error() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "adj_close",
            DataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(Float64Array::from(vec![10.0])) as ArrayRef],
        )
        .unwrap();
        let bytes = write_parquet(&batch);
        // no `day` column is an error on both the series and wide paths
        assert!(read_series(&bytes, "adj_close").is_err());
        assert!(read_wide(&bytes, &["adj_close".to_string()]).is_err());
        // non-Parquet bytes fail to open
        assert!(read_series(b"PAR1 not a real file", "adj_close").is_err());
    }
}
