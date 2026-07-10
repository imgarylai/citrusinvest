//! Small pure helpers shared across FMP modules.

use serde_json::Value;

pub(crate) fn i32_to_iso(d: i32) -> String {
    format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100)
}

/// Parse an FMP date (`YYYY-MM-DD`, optionally with a trailing time) to packed
/// `YYYYMMDD`.
pub(crate) fn iso_to_i32(s: &str) -> Option<i32> {
    s.split_whitespace()
        .next()?
        .trim()
        .replace('-', "")
        .parse()
        .ok()
}

/// First present, numeric value among `keys` (case-sensitive JSON field names).
pub(crate) fn num(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|k| row.get(*k).and_then(Value::as_f64))
}
