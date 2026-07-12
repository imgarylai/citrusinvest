//! Report types and rendering: the [`Status`] / [`Check`] / [`DataAuditReport`]
//! shapes serialized for `--json`, the [`render_table`] human view, and the
//! small formatting/date helpers the checks share.

use serde::Serialize;
use serde_json::{json, Value};

/// A single check's verdict. Ordered `Ok < Warn < Fail` so the report's overall
/// status is the max across checks.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

/// One named check: a status, a one-line human summary, and structured details.
#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub summary: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl Check {
    pub(crate) fn new(
        name: &'static str,
        status: Status,
        summary: impl Into<String>,
        details: Value,
    ) -> Self {
        Check {
            name,
            status,
            summary: summary.into(),
            details,
        }
    }
}

/// The full audit report — serialized directly for `--json`.
#[derive(Serialize)]
pub struct DataAuditReport {
    pub data_dir: String,
    pub from: i32,
    pub to: i32,
    pub symbol_count: usize,
    pub overall: Status,
    pub checks: Vec<Check>,
}

/// Render the report as a compact human-readable table (the CLI's default output).
pub fn render_table(report: &DataAuditReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "data-audit: {}  [{}..{}]  {} symbols\n",
        report.data_dir, report.from, report.to, report.symbol_count
    ));
    out.push_str(&format!("overall: {}\n\n", status_str(report.overall)));
    for c in &report.checks {
        out.push_str(&format!(
            "[{:>4}] {:<16} {}\n",
            status_str(c.status),
            c.name,
            c.summary
        ));
    }
    out
}

pub(crate) fn status_str(s: Status) -> &'static str {
    match s {
        Status::Ok => "OK",
        Status::Warn => "WARN",
        Status::Fail => "FAIL",
    }
}

/// At most 20 sample names, as a JSON array (keeps the report bounded).
pub(crate) fn sample(names: &[&str]) -> Vec<String> {
    names.iter().take(20).map(|s| s.to_string()).collect()
}

pub(crate) fn range_or_null(first: i32, last: i32) -> Value {
    if first == i32::MAX {
        Value::Null
    } else {
        json!({ "first_day": first, "last_day": last })
    }
}

/// Whether `day` (YYYYMMDD) is the last calendar day of its month — every fiscal
/// period-end is a month-end, so a filing date landing here is the lookahead smell.
pub(crate) fn is_month_end(day: i32) -> bool {
    let (y, m, d) = (day / 10000, (day / 100) % 100, day % 100);
    d == days_in_month(y, m)
}

fn days_in_month(y: i32, m: i32) -> i32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_end_and_days_in_month() {
        assert!(is_month_end(20240229)); // leap February
        assert!(is_month_end(20230228)); // non-leap February
        assert!(!is_month_end(20240228)); // 28th is not month-end in a leap year
        assert!(is_month_end(20240131));
        assert!(is_month_end(20240430));
        assert!(!is_month_end(20240415));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29); // divisible by 400
        assert_eq!(days_in_month(1900, 2), 28); // divisible by 100, not 400
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 7), 31);
        assert_eq!(days_in_month(2024, 13), 0);
    }

    #[test]
    fn sample_and_range_helpers() {
        assert_eq!(sample(&["a", "b"]), vec!["a".to_string(), "b".to_string()]);
        assert!(range_or_null(i32::MAX, i32::MIN).is_null());
        assert!(!range_or_null(20240101, 20240102).is_null());
        assert_eq!(status_str(Status::Ok), "OK");
        assert_eq!(status_str(Status::Warn), "WARN");
        assert_eq!(status_str(Status::Fail), "FAIL");
    }
}
