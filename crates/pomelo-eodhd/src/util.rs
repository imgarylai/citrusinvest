//! Small pure helpers shared across EODHD modules.

use serde_json::Value;

pub(crate) fn i32_to_iso(d: i32) -> String {
    format!("{:04}-{:02}-{:02}", d / 10000, d / 100 % 100, d % 100)
}

/// Parse `YYYY-MM-DD` (optional trailing time) to packed `YYYYMMDD`.
pub(crate) fn iso_to_i32(s: &str) -> Option<i32> {
    s.split_whitespace()
        .next()?
        .trim()
        .replace('-', "")
        .parse()
        .ok()
}

/// First present numeric value among `keys`.
pub(crate) fn num(row: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        row.get(*k).and_then(|v| match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn iso_date_roundtrips() {
        assert_eq!(iso_to_i32("2024-01-02"), Some(20240102));
        assert_eq!(iso_to_i32("2024-01-02 00:00:00"), Some(20240102));
        assert_eq!(i32_to_iso(20240102), "2024-01-02");
        assert_eq!(iso_to_i32("garbage"), None);
    }

    #[test]
    fn num_reads_number_or_string() {
        let obj = json!({"a": 1.5, "b": "2.5", "c": true})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(num(&obj, &["a"]), Some(1.5));
        assert_eq!(num(&obj, &["missing", "b"]), Some(2.5));
        assert_eq!(num(&obj, &["c"]), None);
        assert_eq!(num(&obj, &["nope"]), None);
    }
}
