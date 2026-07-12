//! Tree discovery and the single-pass fundamentals scan: which symbols /
//! fundamentals files / membership panels exist ([`ObjectLister::list`]), the
//! small `get`/decode/parse helpers that back every read, and [`scan_fundamentals`].

use std::collections::{BTreeSet, HashMap};

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::{
    ObjectLister, ObjectSource, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS, PANELS_DIR, PRICES_DIR,
};

/// Result of the single-pass fundamentals scan.
pub(crate) struct FundScan {
    /// Fundamental field names seen with ≥1 finite value across all symbols.
    pub(crate) fields_seen: BTreeSet<String>,
    /// Every `report_event == 1` day (windowed to `[from, to]`).
    pub(crate) report_event_days: Vec<i32>,
    /// Number of fundamentals files read.
    pub(crate) file_count: usize,
}

/// Parse every `fundamentals/{SYM}.csv.gz` once: which fields are ever populated
/// and every filing (`report_event`) day. Fail-soft per file.
pub(crate) fn scan_fundamentals<S: ObjectSource + ObjectLister>(
    src: &S,
    from: i32,
    to: i32,
) -> FundScan {
    let mut scan = FundScan {
        fields_seen: BTreeSet::new(),
        report_event_days: Vec::new(),
        file_count: 0,
    };
    for sym in list_stems(src, FUNDAMENTALS_DIR, &[".csv.gz", ".csv"]) {
        let bytes = match try_get(src, &format!("{FUNDAMENTALS_DIR}/{sym}")) {
            Some(b) => b,
            None => continue,
        };
        scan.file_count += 1;
        let text = decode_text(&bytes);
        let mut lines = text.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let cols: Vec<&str> = header.split(',').map(str::trim).collect();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let cells: Vec<&str> = line.split(',').collect();
            let day = cells.first().and_then(|c| parse_day(c));
            for (i, col) in cols.iter().enumerate() {
                let Some(cell) = cells.get(i) else { continue };
                let Some(v) = parse_finite(cell) else {
                    continue;
                };
                if *col == pomelo_data::REPORT_EVENT_FIELD {
                    if v >= 0.5 {
                        if let Some(d) = day {
                            if d >= from && d <= to {
                                scan.report_event_days.push(d);
                            }
                        }
                    }
                } else if FUNDAMENTAL_FIELDS.contains(col) {
                    scan.fields_seen.insert((*col).to_string());
                }
            }
        }
    }
    scan
}

/// Symbols with a per-symbol price file under `prices/` (`.csv.gz` /
/// `.parquet` / `.csv`), sorted and de-duplicated.
pub(crate) fn list_price_symbols<S: ObjectLister>(src: &S) -> Vec<String> {
    list_stems(src, PRICES_DIR, &[".csv.gz", ".parquet", ".csv"])
}

/// Load and decode `tracked/universe.csv.gz` into a `symbol → sector` map.
pub(crate) fn load_industry_map<S: ObjectSource>(src: &S) -> Option<HashMap<String, String>> {
    let bytes = try_get(src, "tracked/universe")?;
    let map = parse_industry_csv(&decode_text(&bytes));
    (!map.is_empty()).then_some(map)
}

/// `src.get` for `key` trying `.csv.gz` then `.csv` (the formats the sync writes).
pub(crate) fn try_get<S: ObjectSource>(src: &S, key_stem: &str) -> Option<Vec<u8>> {
    for ext in [".csv.gz", ".csv"] {
        if let Ok(Some(bytes)) = src.get(&format!("{key_stem}{ext}")) {
            return Some(bytes);
        }
    }
    None
}

/// File stems under `prefix` (via [`ObjectLister::list`]) with any of `exts`
/// stripped, sorted + de-duplicated. `exts` must be ordered longest-first so
/// `.csv.gz` isn't mis-stripped to `.csv`. Fail-soft: an unreadable/absent
/// prefix (local dir missing, or a listing error) yields no stems.
pub(crate) fn list_stems<S: ObjectLister>(src: &S, prefix: &str, exts: &[&str]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for key in src.list(prefix).unwrap_or_default() {
        let name = key.rsplit('/').next().unwrap_or(&key);
        if let Some(stem) = exts.iter().find_map(|e| name.strip_suffix(e)) {
            out.insert(stem.to_string());
        }
    }
    out.into_iter().collect()
}

/// Series names of `panels/in_*.{csv.gz,csv}` (index membership panels).
pub(crate) fn list_membership_panels<S: ObjectLister>(src: &S) -> Vec<String> {
    list_stems(src, PANELS_DIR, &[".csv.gz", ".csv"])
        .into_iter()
        .filter(|s| s.starts_with("in_"))
        .collect()
}

/// Decode bytes that may be gzip (`.csv.gz`) or plain UTF-8 text.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    use std::io::Read;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = String::new();
        if flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .is_ok()
        {
            return out;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Parse a `day` cell in either `YYYY-MM-DD` or `YYYYMMDD` form to a packed i32.
pub(crate) fn parse_day(s: &str) -> Option<i32> {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    (digits.len() == 8).then(|| digits.parse().ok()).flatten()
}

/// Parse a cell to a finite f64, or `None` for empty / non-finite.
pub(crate) fn parse_finite(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_decode_helpers() {
        assert_eq!(parse_day("2024-01-02"), Some(20240102));
        assert_eq!(parse_day("20240102"), Some(20240102));
        assert_eq!(parse_day("garbage"), None);
        assert_eq!(parse_finite("1.5"), Some(1.5));
        assert_eq!(parse_finite(""), None);
        assert_eq!(parse_finite("  "), None);
        assert!(parse_finite("nan").is_none()); // NaN is filtered
        assert!(parse_finite("inf").is_none()); // infinity is filtered
        assert_eq!(decode_text(b"plain,text"), "plain,text");
    }
}
