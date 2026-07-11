//! On-the-wire format detection for data files. Objects can be stored as gzip
//! CSV (`.csv.gz`, the default write format), plain CSV (`.csv`), or Apache
//! Parquet (`.parquet`, requires the `parquet` feature). Detection is by content
//! magic bytes, not the object key, so a file parses correctly regardless of its
//! extension and existing gzip CSV data keeps working unchanged.

use crate::error::DataError;
use flate2::read::GzDecoder;
use std::io::Read;

/// The storage encoding of a data object, inferred from its leading bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// gzip-compressed CSV — the default per-symbol/panel write format (`.csv.gz`).
    CsvGz,
    /// Uncompressed CSV text (`.csv`).
    Csv,
    /// Apache Parquet (`.parquet`).
    Parquet,
}

impl Format {
    /// Infer the format from a buffer's magic bytes. gzip starts `1f 8b`,
    /// Parquet files start (and end) with the ASCII marker `PAR1`; anything
    /// else is treated as plain CSV text.
    pub fn detect(bytes: &[u8]) -> Format {
        if bytes.starts_with(&[0x1f, 0x8b]) {
            Format::CsvGz
        } else if bytes.starts_with(b"PAR1") {
            Format::Parquet
        } else {
            Format::Csv
        }
    }
}

/// Candidate object-key extensions to probe for a per-symbol/panel object, in
/// priority order. `.csv.gz` is first so existing deployments hit on the first
/// GET; `.parquet` is only probed when the feature that can read it is enabled
/// (otherwise the extra GET would fetch bytes we could not parse).
#[cfg(feature = "parquet")]
pub(crate) const CANDIDATE_EXTS: &[&str] = &[".csv.gz", ".parquet", ".csv"];
#[cfg(not(feature = "parquet"))]
pub(crate) const CANDIDATE_EXTS: &[&str] = &[".csv.gz", ".csv"];

/// Decode a CSV-family buffer to text: gunzip when gzip, otherwise interpret the
/// bytes as UTF-8 as-is (plain CSV). Parquet buffers are handled elsewhere and
/// must not be passed here.
pub(crate) fn read_csv_text(bytes: &[u8]) -> Result<String, DataError> {
    match Format::detect(bytes) {
        Format::CsvGz => {
            let mut text = String::new();
            GzDecoder::new(bytes)
                .read_to_string(&mut text)
                .map_err(|e| DataError::Io(e.to_string()))?;
            Ok(text)
        }
        Format::Csv => {
            String::from_utf8(bytes.to_vec()).map_err(|e| DataError::Parse(e.to_string()))
        }
        Format::Parquet => Err(DataError::Parse(
            "expected CSV bytes, got Parquet".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn gz(text: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn detects_each_format_from_magic_bytes() {
        assert_eq!(Format::detect(&gz("day,close\n")), Format::CsvGz);
        assert_eq!(Format::detect(b"day,close\n2024-01-02,10\n"), Format::Csv);
        assert_eq!(Format::detect(b"PAR1\x00\x00PAR1"), Format::Parquet);
        assert_eq!(Format::detect(b""), Format::Csv);
    }

    #[test]
    fn read_csv_text_handles_gzip_and_plain() {
        let plain = "day,close\n2024-01-02,10\n";
        assert_eq!(read_csv_text(plain.as_bytes()).unwrap(), plain);
        assert_eq!(read_csv_text(&gz(plain)).unwrap(), plain);
        assert!(read_csv_text(b"PAR1junk").is_err());
    }
}
