//! Small pure helpers for pomelo-finnhub.
//!
//! Finnhub `/stock/candle` speaks UNIX seconds, while the rest of the crate and
//! the data-layout use packed `YYYYMMDD`. These conversions bridge the two with
//! integer-only proleptic-Gregorian math (Howard Hinnant's `days_from_civil` /
//! `civil_from_days`) so we need no `chrono`/`time` dependency.

/// Convert a packed `YYYYMMDD` date to UNIX seconds at 00:00:00 UTC.
pub(crate) fn i32_to_unix(packed: i32) -> i64 {
    let y = (packed / 10000) as i64;
    let m = (packed / 100 % 100) as i64;
    let d = (packed % 100) as i64;
    days_from_civil(y, m, d) * 86_400
}

/// Convert UNIX seconds to the packed `YYYYMMDD` of the UTC calendar day.
pub(crate) fn unix_to_i32(secs: i64) -> i32 {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    (y * 10000 + m * 100 + d) as i32
}

/// Days since 1970-01-01 for a proleptic Gregorian civil date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`]: days-since-epoch → `(year, month, day)`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trips() {
        assert_eq!(i32_to_unix(19700101), 0);
        assert_eq!(unix_to_i32(0), 19700101);
    }

    #[test]
    fn known_dates() {
        // 2024-01-02 00:00:00 UTC = 1704153600
        assert_eq!(i32_to_unix(20240102), 1_704_153_600);
        assert_eq!(unix_to_i32(1_704_153_600), 20240102);
        // Any time within the UTC day maps back to the same calendar day.
        assert_eq!(unix_to_i32(1_704_153_600 + 86_399), 20240102);
    }

    #[test]
    fn round_trip_across_years() {
        for packed in [20000101, 20080229, 20191231, 20200101, 20241231, 20991231] {
            assert_eq!(unix_to_i32(i32_to_unix(packed)), packed, "{packed}");
        }
    }
}
