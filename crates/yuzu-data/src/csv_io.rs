//! Per-symbol gzip CSV I/O. Files hold full adjusted OHLCV
//! (`day,adj_open,adj_high,adj_low,adj_close,volume`, oldest-first); `parse_series`
//! extracts one chosen [`Field`] column into `(day, value)` rows.

use crate::error::DataError;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

/// One row of adjusted OHLCV for a trading day.
#[derive(Debug, Clone, PartialEq)]
pub struct OhlcvRow {
    pub day: i32,
    pub adj_open: f64,
    pub adj_high: f64,
    pub adj_low: f64,
    pub adj_close: f64,
    pub volume: f64,
}

/// Which OHLCV column to pull into a Panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    AdjOpen,
    AdjHigh,
    AdjLow,
    AdjClose,
    Volume,
}

impl Field {
    /// Column index in the CSV (day is column 0).
    fn col(self) -> usize {
        match self {
            Field::AdjOpen => 1,
            Field::AdjHigh => 2,
            Field::AdjLow => 3,
            Field::AdjClose => 4,
            Field::Volume => 5,
        }
    }

    /// Column name (matches the CSV header and the Parquet column name).
    pub fn name(self) -> &'static str {
        match self {
            Field::AdjOpen => "adj_open",
            Field::AdjHigh => "adj_high",
            Field::AdjLow => "adj_low",
            Field::AdjClose => "adj_close",
            Field::Volume => "volume",
        }
    }
}

fn date_to_i32(s: &str) -> Result<i32, DataError> {
    s.replace('-', "")
        .parse()
        .map_err(|_| DataError::Parse(format!("bad date '{s}'")))
}

fn i32_to_date(d: i32) -> String {
    format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100)
}

/// Parse the OHLCV data for `field` into `(YYYYMMDD, value)` rows. The buffer's
/// format is detected from its content: gzip CSV, plain CSV, or — with the
/// `parquet` feature — Apache Parquet. CSV is header-optional, oldest-first.
pub fn parse_series(bytes: &[u8], field: Field) -> Result<Vec<(i32, f64)>, DataError> {
    #[cfg(feature = "parquet")]
    if crate::format::Format::detect(bytes) == crate::format::Format::Parquet {
        return crate::parquet_io::read_series(bytes, field.name());
    }
    let text = crate::format::read_csv_text(bytes)?;
    parse_series_csv(&text, field.col())
}

/// Extract column `col` (0 = `day`) from decoded OHLCV CSV text.
fn parse_series_csv(text: &str, col: usize) -> Result<Vec<(i32, f64)>, DataError> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("day") {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        if cells.len() <= col {
            return Err(DataError::Parse(format!(
                "row has {} cells, need > {col}",
                cells.len()
            )));
        }
        let day = date_to_i32(cells[0].trim())?;
        let val: f64 = cells[col]
            .trim()
            .parse()
            .map_err(|_| DataError::Parse(format!("bad value '{}'", cells[col])))?;
        out.push((day, val));
    }
    Ok(out)
}

/// Serialize OHLCV rows (oldest-first) to gzip CSV with the standard header.
pub fn write_series(rows: &[OhlcvRow]) -> Result<Vec<u8>, DataError> {
    let mut buf = String::from("day,adj_open,adj_high,adj_low,adj_close,volume\n");
    for r in rows {
        buf.push_str(&format!(
            "{},{},{},{},{},{}\n",
            i32_to_date(r.day),
            r.adj_open,
            r.adj_high,
            r.adj_low,
            r.adj_close,
            r.volume
        ));
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(buf.as_bytes())
        .map_err(|e| DataError::Io(e.to_string()))?;
    enc.finish().map_err(|e| DataError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: i32, c: f64) -> OhlcvRow {
        OhlcvRow {
            day,
            adj_open: c - 0.5,
            adj_high: c + 1.0,
            adj_low: c - 1.0,
            adj_close: c,
            volume: 1000.0,
        }
    }

    #[test]
    fn round_trips_and_selects_fields() {
        let rows = vec![row(20240102, 10.5), row(20240103, 11.25)];
        let gz = write_series(&rows).unwrap();
        assert_eq!(
            parse_series(&gz, Field::AdjClose).unwrap(),
            vec![(20240102, 10.5), (20240103, 11.25)]
        );
        assert_eq!(
            parse_series(&gz, Field::AdjHigh).unwrap(),
            vec![(20240102, 11.5), (20240103, 12.25)]
        );
        assert_eq!(
            parse_series(&gz, Field::AdjLow).unwrap(),
            vec![(20240102, 9.5), (20240103, 10.25)]
        );
        assert_eq!(
            parse_series(&gz, Field::AdjOpen).unwrap(),
            vec![(20240102, 10.0), (20240103, 10.75)]
        );
        assert_eq!(
            parse_series(&gz, Field::Volume).unwrap(),
            vec![(20240102, 1000.0), (20240103, 1000.0)]
        );
    }

    fn gz(text: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn parse_errors_on_non_gzip_bad_value_bad_date_and_short_row() {
        let hdr = "day,adj_open,adj_high,adj_low,adj_close,volume\n";
        assert!(parse_series(b"not gzip", Field::AdjClose).is_err());
        assert!(parse_series(
            &gz(&format!("{hdr}2024-01-02,1,2,3,bad,5\n")),
            Field::AdjClose
        )
        .is_err());
        assert!(parse_series(&gz(&format!("{hdr}bad-date,1,2,3,4,5\n")), Field::AdjClose).is_err());
        // row missing the volume column, but Volume requested
        assert!(parse_series(&gz(&format!("{hdr}2024-01-02,1,2,3,4\n")), Field::Volume).is_err());
    }
}
