//! Per-symbol gzip CSV I/O for fundamentals — the native mirror of the Worker's
//! `factor-panels.ts` export. Files hold dense, forward-filled fundamental fields
//! (`day,pe,ps,pb,…`, oldest-first); `parse_fundamentals` extracts one field
//! column into `(day, value)` rows. Same format/conventions as `csv_io.rs`.
//!
//! `FUNDAMENTAL_FIELDS` is the column contract: it MUST match the TS exporter's
//! `FACTOR_FIELDS` order AND the `Data { name }` snake_case names in specs.

use crate::date::{date_to_i32, i32_to_date};
use crate::error::DataError;
use crate::source::ObjectSource;
use flate2::write::GzEncoder;
use flate2::Compression;
use ndarray::Array2;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use yuzu_core::panel::Panel;

/// Fundamental field column names, in CSV order (column 0 is `day`).
pub const FUNDAMENTAL_FIELDS: &[&str] = &[
    "pe",
    "ps",
    "pb",
    "roe",
    "net_margin",
    "debt_to_equity",
    "market_cap",
    "gross_margin",
    "receivables_turnover",
    "debt_to_assets",
    "revenue",
    "revenue_growth",
    "eps_growth",
    "operating_income_growth",
    "net_income_growth",
    "gross_profit_growth",
];

/// The extra trailing column marking real report-filing days (`1.0` on a day a
/// new report was disclosed, else `0.0`). Not a [`FUNDAMENTAL_FIELDS`] factor —
/// it's the event signal that the dense forward-filled factor columns can't
/// express. Missing → `NaN`, which the engine's `is_true` (x == 1.0) reads as
/// "no event", same as `0.0`.
pub const REPORT_EVENT_FIELD: &str = "report_event";

/// Snapshot-based factor fields whose combined panels (`panels/{name}.csv.gz`)
/// are written directly by the Worker (not by `rebuild_combined_panels`, which
/// only processes per-symbol fundamentals CSVs). Keeping them separate from
/// `FUNDAMENTAL_FIELDS` prevents `rebuild_combined_panels` from overwriting them
/// with all-NaN panels every nightly run.
///
/// ORDER PARITY: names must appear in the same order as `STABLE_FACTOR_NAMES`
/// in `apps/web/src/lib/lemon/fields.ts`. A Vitest assertion in
/// `factor-panels.test.ts` verifies the TS side; the Rust side is tested in
/// `fundamentals.rs` (see the parity test below).
pub const FACTOR_PANEL_FIELDS: &[&str] = &[
    "piotroski_score",
    "altman_z",
    "fcf_yield",
    "pe_industry_pctile",
    "analyst_upside_pct",
    "consensus_rating",
];

/// Column index (0 = `day`) of a series in the per-symbol fundamentals CSV, or
/// `None` if the name is neither a factor nor [`REPORT_EVENT_FIELD`].
fn field_col(field: &str) -> Option<usize> {
    if field == REPORT_EVENT_FIELD {
        Some(FUNDAMENTAL_FIELDS.len() + 1) // last column, after day + the 16 factors
    } else {
        FUNDAMENTAL_FIELDS
            .iter()
            .position(|f| *f == field)
            .map(|i| i + 1)
    }
}

/// Whether `name` is a series the fundamentals files carry (a factor, the
/// report-event signal, or a Worker-written snapshot factor) — used by callers
/// to route a spec series to the right combined-panel loader.
pub fn is_fundamental_series(name: &str) -> bool {
    field_col(name).is_some() || FACTOR_PANEL_FIELDS.contains(&name)
}

/// One dense row of fundamentals for a trading day. `values` is aligned to
/// [`FUNDAMENTAL_FIELDS`]; unset fields are `NaN`. `report_event` is the trailing
/// [`REPORT_EVENT_FIELD`] column.
#[derive(Debug, Clone, PartialEq)]
pub struct FundamentalRow {
    pub day: i32,
    pub values: Vec<f64>,
    pub report_event: f64,
}

/// Parse fundamentals for `field` (one of [`FUNDAMENTAL_FIELDS`] or
/// [`REPORT_EVENT_FIELD`]) into `(YYYYMMDD, value)` rows. The buffer's format is
/// detected from its content: gzip CSV, plain CSV, or — with the `parquet`
/// feature — Apache Parquet. Empty / `NaN` cells parse to `NaN`.
pub fn parse_fundamentals(bytes: &[u8], field: &str) -> Result<Vec<(i32, f64)>, DataError> {
    // Resolve the column first so an unknown field fails before any I/O.
    let col = field_col(field)
        .ok_or_else(|| DataError::Parse(format!("unknown fundamental field '{field}'")))?;
    #[cfg(feature = "parquet")]
    if crate::format::Format::detect(bytes) == crate::format::Format::Parquet {
        return crate::parquet_io::read_series(bytes, field);
    }
    let text = crate::format::read_csv_text(bytes)?;
    parse_fundamentals_csv(&text, col)
}

/// Extract column `col` (0 = `day`) from decoded fundamentals CSV text.
fn parse_fundamentals_csv(text: &str, col: usize) -> Result<Vec<(i32, f64)>, DataError> {
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
        let cell = cells[col].trim();
        // Empty cell → NaN (no fundamental that day); otherwise parse (incl. "NaN").
        let val: f64 = if cell.is_empty() {
            f64::NAN
        } else {
            cell.parse()
                .map_err(|_| DataError::Parse(format!("bad value '{cell}'")))?
        };
        out.push((day, val));
    }
    Ok(out)
}

/// Serialize fundamentals rows (oldest-first) to gzip CSV with the standard header.
pub fn write_fundamentals(rows: &[FundamentalRow]) -> Result<Vec<u8>, DataError> {
    let mut buf = String::from("day");
    for f in FUNDAMENTAL_FIELDS {
        buf.push(',');
        buf.push_str(f);
    }
    buf.push(',');
    buf.push_str(REPORT_EVENT_FIELD);
    buf.push('\n');
    for r in rows {
        buf.push_str(&i32_to_date(r.day));
        for v in &r.values {
            buf.push(',');
            buf.push_str(&v.to_string());
        }
        buf.push(',');
        buf.push_str(&r.report_event.to_string());
        buf.push('\n');
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(buf.as_bytes())
        .map_err(|e| DataError::Io(e.to_string()))?;
    enc.finish().map_err(|e| DataError::Io(e.to_string()))
}

/// Default object-key directory for per-symbol fundamentals files. Override at the
/// entry point (e.g. `YUZU_FUNDAMENTALS_DIR`) for a custom layout.
pub const FUNDAMENTALS_DIR: &str = "fundamentals";

/// Read `{dir}/{symbol}.csv.gz` for each symbol, extract `field`, filter to
/// `[from, to]` (inclusive), and assemble a Panel: rows = sorted union of kept days,
/// columns = `symbols` in order. Missing/corrupt files and absent cells are NaN.
/// `dir` defaults to [`FUNDAMENTALS_DIR`] at call sites. The native mirror of the
/// Worker building a factor panel — but read straight from object storage, no D1.
pub fn load_fundamental_panel<S: ObjectSource + Sync>(
    source: &S,
    symbols: &[String],
    field: &str,
    from: i32,
    to: i32,
    dir: &str,
) -> Result<Panel, DataError> {
    // Validate the field once — an unknown field is a bug, not a missing file.
    if !is_fundamental_series(field) {
        return Err(DataError::Parse(format!(
            "unknown fundamental field '{field}'"
        )));
    }
    // Fetch + parse every symbol concurrently (network-bound); missing/corrupt
    // files leave a NaN column rather than sinking the batch.
    let per_symbol = crate::parallel::fetch_series(source, symbols, dir, from, to, |b| {
        parse_fundamentals(b, field)
    })?;

    let mut date_set: BTreeSet<i32> = BTreeSet::new();
    for map in &per_symbol {
        date_set.extend(map.keys().copied());
    }
    let dates: Vec<i32> = date_set.into_iter().collect();
    let row_of: HashMap<i32, usize> = dates.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut data = Array2::from_elem((dates.len(), symbols.len()), f64::NAN);
    for (c, map) in per_symbol.iter().enumerate() {
        for (d, v) in map {
            data[[row_of[d], c]] = *v;
        }
    }
    Panel::new(dates, symbols.to_vec(), data).map_err(|e| DataError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frow(day: i32, set: &[(&str, f64)]) -> FundamentalRow {
        let mut values = vec![f64::NAN; FUNDAMENTAL_FIELDS.len()];
        let mut report_event = 0.0;
        for (k, v) in set {
            if *k == REPORT_EVENT_FIELD {
                report_event = *v;
                continue;
            }
            let i = FUNDAMENTAL_FIELDS.iter().position(|f| f == k).unwrap();
            values[i] = *v;
        }
        FundamentalRow {
            day,
            values,
            report_event,
        }
    }

    /// Verify that FACTOR_PANEL_FIELDS contains exactly the expected stable
    /// snapshot factor names in the expected order (mirrors the TS parity test
    /// in factor-panels.test.ts).
    #[test]
    fn factor_panel_fields_order_and_count() {
        let expected = [
            "piotroski_score",
            "altman_z",
            "fcf_yield",
            "pe_industry_pctile",
            "analyst_upside_pct",
            "consensus_rating",
        ];
        assert_eq!(
            FACTOR_PANEL_FIELDS, &expected,
            "FACTOR_PANEL_FIELDS must match TS STABLE_FACTOR_NAMES in order"
        );
    }

    #[test]
    fn is_fundamental_series_recognises_factor_panel_fields() {
        for name in FACTOR_PANEL_FIELDS {
            assert!(
                is_fundamental_series(name),
                "is_fundamental_series should return true for factor panel field '{name}'"
            );
        }
        // Price fields are NOT fundamental series.
        assert!(!is_fundamental_series("close"));
        assert!(!is_fundamental_series("unknown_field"));
        // Original fundamental fields still work.
        assert!(is_fundamental_series("pe"));
        assert!(is_fundamental_series("market_cap"));
        // report_event still works.
        assert!(is_fundamental_series("report_event"));
    }

    #[test]
    fn report_event_is_a_trailing_column() {
        let rows = vec![
            frow(20240102, &[("pe", 10.0), ("report_event", 0.0)]),
            frow(20240103, &[("pe", 11.0), ("report_event", 1.0)]),
        ];
        let bytes = write_fundamentals(&rows).unwrap();
        // factors still resolve, and report_event round-trips from the last column
        assert_eq!(
            parse_fundamentals(&bytes, "pe").unwrap()[1],
            (20240103, 11.0)
        );
        assert_eq!(
            parse_fundamentals(&bytes, "report_event").unwrap(),
            vec![(20240102, 0.0), (20240103, 1.0)]
        );
        assert!(is_fundamental_series("report_event"));
        assert!(is_fundamental_series("pe"));
        assert!(!is_fundamental_series("close"));
    }

    #[test]
    fn roundtrip_and_field_extract() {
        let rows = vec![
            frow(20240102, &[("pe", 10.0), ("pb", 1.5), ("market_cap", 1e9)]),
            frow(
                20240103,
                &[("pe", 11.0), ("pb", 1.6), ("market_cap", 1.1e9)],
            ),
        ];
        let bytes = write_fundamentals(&rows).unwrap();
        assert_eq!(
            parse_fundamentals(&bytes, "pe").unwrap(),
            vec![(20240102, 10.0), (20240103, 11.0)]
        );
        assert_eq!(
            parse_fundamentals(&bytes, "market_cap").unwrap()[1],
            (20240103, 1.1e9)
        );
        assert_eq!(
            parse_fundamentals(&bytes, "pb").unwrap()[0],
            (20240102, 1.5)
        );
    }

    #[test]
    fn unknown_field_errors_and_bad_gzip_errors() {
        let bytes = write_fundamentals(&[frow(20240102, &[("pe", 10.0)])]).unwrap();
        assert!(parse_fundamentals(&bytes, "not_a_field").is_err());
        assert!(parse_fundamentals(b"not gzip", "pe").is_err());
    }

    #[test]
    fn load_fundamental_panel_union_dates_and_nan() {
        use crate::source::LocalSource;
        use std::fs;
        let dir = std::env::temp_dir().join("yuzu_data_fund_panel");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("fundamentals")).unwrap();
        fs::write(
            dir.join("fundamentals/AAPL.csv.gz"),
            write_fundamentals(&[
                frow(20240102, &[("pe", 10.0)]),
                frow(20240103, &[("pe", 11.0)]),
            ])
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("fundamentals/MSFT.csv.gz"),
            write_fundamentals(&[frow(20240103, &[("pe", 20.0)])]).unwrap(),
        )
        .unwrap();
        let src = LocalSource::new(&dir);
        let syms = vec!["AAPL".to_string(), "MSFT".to_string(), "ZZZ".to_string()];

        let pe = load_fundamental_panel(&src, &syms, "pe", 20240102, 20240103, FUNDAMENTALS_DIR)
            .unwrap();
        assert_eq!(pe.dates, vec![20240102, 20240103]);
        assert_eq!(pe.data[[0, 0]], 10.0); // AAPL on 0102
        assert!(pe.data[[0, 1]].is_nan()); // MSFT absent 0102
        assert_eq!(pe.data[[1, 1]], 20.0); // MSFT on 0103
        assert!(pe.data[[1, 2]].is_nan()); // ZZZ no file

        // unknown field is an error, not an all-NaN panel
        assert!(
            load_fundamental_panel(&src, &syms, "nope", 20240102, 20240103, FUNDAMENTALS_DIR)
                .is_err()
        );
    }
}
