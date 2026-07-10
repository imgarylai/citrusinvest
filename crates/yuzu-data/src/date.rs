//! Packed trading-day helpers: `YYYY-MM-DD` text ↔ `i32` `YYYYMMDD`.
//!
//! Shared by CSV, fundamentals, combined panels, and Parquet loaders so the
//! parse/format rules cannot drift between modules.

use crate::error::DataError;

/// Parse a date string into packed `YYYYMMDD`. Dashes are optional
/// (`2024-01-02` and `20240102` both work).
pub(crate) fn date_to_i32(s: &str) -> Result<i32, DataError> {
    s.replace('-', "")
        .parse()
        .map_err(|_| DataError::Parse(format!("bad date '{s}'")))
}

/// Format a packed `YYYYMMDD` as `YYYY-MM-DD`.
pub(crate) fn i32_to_date(d: i32) -> String {
    format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_dashless_parse() {
        assert_eq!(date_to_i32("2024-01-02").unwrap(), 20240102);
        assert_eq!(date_to_i32("20240102").unwrap(), 20240102);
        assert_eq!(i32_to_date(20240102), "2024-01-02");
        assert!(date_to_i32("not-a-date").is_err());
    }
}
