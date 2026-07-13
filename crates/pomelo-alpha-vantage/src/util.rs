//! Small pure helpers shared across Alpha Vantage modules.

use serde_json::Value;

/// Parse `YYYY-MM-DD` (optional trailing time) to packed `YYYYMMDD`.
pub(crate) fn iso_to_i32(s: &str) -> Option<i32> {
    s.split_whitespace()
        .next()?
        .trim()
        .replace('-', "")
        .parse()
        .ok()
}

/// Alpha Vantage time-series cells use `"1. open"`-style keys; match by suffix.
pub(crate) fn num_av_field(row: &serde_json::Map<String, Value>, field: &str) -> Option<f64> {
    let field_lc = field.to_ascii_lowercase();
    for (k, v) in row {
        let k_lc = k.to_ascii_lowercase();
        // exact, or ends with " open" / ". open" / "open"
        let hit = k_lc == field_lc
            || k_lc.ends_with(&format!(" {field_lc}"))
            || k_lc.ends_with(&format!(".{field_lc}"))
            || k_lc.ends_with(&field_lc);
        if hit {
            return match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn iso_date_parses() {
        assert_eq!(iso_to_i32("2024-01-02"), Some(20240102));
        assert_eq!(iso_to_i32("2024-01-02 00:00:00"), Some(20240102));
        assert_eq!(iso_to_i32("garbage"), None);
    }

    #[test]
    fn num_av_field_reads_numbered_keys() {
        let obj = json!({
            "1. open": "102.5",
            "5. adjusted close": "50.0",
            "6. volume": 1000
        })
        .as_object()
        .unwrap()
        .clone();
        assert_eq!(num_av_field(&obj, "open"), Some(102.5));
        assert_eq!(num_av_field(&obj, "adjusted close"), Some(50.0));
        assert_eq!(num_av_field(&obj, "volume"), Some(1000.0));
        assert_eq!(num_av_field(&obj, "missing"), None);
    }
}
