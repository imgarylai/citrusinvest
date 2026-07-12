//! Read-only data-quality audit of a data-layout tree (#133).
//!
//! `yuzu-cli data-audit` walks a synced `prices/` / `fundamentals/` / `panels/`
//! / `tracked/` tree and answers *"is this clean enough to trust a backtest?"* —
//! turning "high-quality data" from a claim into a measurement. It also doubles
//! as the verification tool for #131 (filing-date lag) and #132 (snapshot-factor
//! coverage). No network, no engine run: it reuses the `pomelo-data` loaders and
//! reports per-check `OK` / `WARN` / `FAIL` with counts. Any `FAIL` makes the CLI
//! exit non-zero so it can gate CI or a nightly job.

use std::collections::BTreeSet;
use std::path::Path;

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::{
    load_combined_panel, load_panel, Field, LocalSource, ObjectSource, FACTOR_PANEL_FIELDS,
    FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS, PANELS_DIR, PRICES_DIR,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::list_symbols;

/// Overnight |return| above this flags a candidate un-adjusted split / bad tick.
const JUMP_THRESHOLD: f64 = 0.5;
/// Fraction of filing (`report_event`) days on a calendar month-end above which
/// the PIT-lag check warns of possible period-end (lookahead) stamping.
const LOOKAHEAD_FRACTION: f64 = 0.5;

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
    fn new(name: &'static str, status: Status, summary: impl Into<String>, details: Value) -> Self {
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

/// Run every check over the data-layout tree at `root`, windowed to `[from, to]`.
/// Fail-soft: a missing directory or file downgrades a check, never panics.
pub fn run_data_audit(root: &Path, from: i32, to: i32) -> Result<DataAuditReport, String> {
    let src = LocalSource::new(root);
    let symbols = list_symbols(root).map_err(|e| format!("listing symbols under {root:?}: {e}"))?;

    // One adj-close Panel (union calendar × symbols, NaN where absent) backs the
    // coverage / gaps / delist / jump checks.
    let closes = if symbols.is_empty() {
        None
    } else {
        Some(
            load_panel(&src, &symbols, Field::AdjClose, from, to, PRICES_DIR)
                .map_err(|e| format!("loading price panel: {e}"))?,
        )
    };

    // Fundamentals are parsed once (per-field coverage + filing-event days).
    let fund = scan_fundamentals(root, &src, from, to);

    let checks = vec![
        check_coverage(&src, &symbols, closes.as_ref()),
        check_calendar_gaps(&symbols, closes.as_ref()),
        check_adjustment(&symbols, closes.as_ref()),
        check_survivorship(&symbols, closes.as_ref()),
        check_nan_density(&src, &symbols, &fund, from, to),
        check_pit_lag(&fund),
        check_index_membership(root, &src, &symbols, from, to),
    ];

    let overall = checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok);
    Ok(DataAuditReport {
        data_dir: root.display().to_string(),
        from,
        to,
        symbol_count: symbols.len(),
        overall,
        checks,
    })
}

// ── per-symbol close series helpers ─────────────────────────────────────────

/// The non-NaN `(day, close)` points of symbol `col` in the panel, in date order.
fn symbol_points(panel: &yuzu_core::panel::Panel, col: usize) -> Vec<(i32, f64)> {
    panel
        .dates
        .iter()
        .enumerate()
        .filter_map(|(r, &day)| {
            let v = panel.data[[r, col]];
            v.is_finite().then_some((day, v))
        })
        .collect()
}

// ── checks ──────────────────────────────────────────────────────────────────

/// Coverage: symbols with price files vs the `tracked/universe.csv.gz` map —
/// names in the universe but missing prices (and vice versa).
fn check_coverage(
    src: &LocalSource,
    symbols: &[String],
    closes: Option<&yuzu_core::panel::Panel>,
) -> Check {
    if symbols.is_empty() {
        return Check::new(
            "coverage",
            Status::Fail,
            "no price files found under prices/",
            json!({ "symbols_with_prices": 0 }),
        );
    }
    let price_set: BTreeSet<&str> = symbols.iter().map(String::as_str).collect();
    let universe = load_industry_map(src);
    let (in_universe_only, price_only, universe_len) = match &universe {
        Some(map) => {
            let uni: BTreeSet<&str> = map.keys().map(String::as_str).collect();
            let in_universe_only: Vec<&str> =
                uni.difference(&price_set).copied().collect::<Vec<_>>();
            let price_only: Vec<&str> = price_set.difference(&uni).copied().collect::<Vec<_>>();
            (in_universe_only, price_only, uni.len())
        }
        None => (Vec::new(), Vec::new(), 0),
    };

    // Per-symbol first/last observed day, for the report.
    let (mut first_day, mut last_day) = (i32::MAX, i32::MIN);
    if let Some(panel) = closes {
        for col in 0..symbols.len() {
            let pts = symbol_points(panel, col);
            if let (Some(&(f, _)), Some(&(l, _))) = (pts.first(), pts.last()) {
                first_day = first_day.min(f);
                last_day = last_day.max(l);
            }
        }
    }

    let status = if universe.is_some() && !in_universe_only.is_empty() {
        Status::Warn
    } else {
        Status::Ok
    };
    let summary = match &universe {
        Some(_) => format!(
            "{} symbols priced; {} in universe missing prices, {} priced not in universe",
            symbols.len(),
            in_universe_only.len(),
            price_only.len()
        ),
        None => format!(
            "{} symbols priced; no tracked/universe.csv.gz to cross-check",
            symbols.len()
        ),
    };
    Check::new(
        "coverage",
        status,
        summary,
        json!({
            "symbols_with_prices": symbols.len(),
            "universe_size": universe_len,
            "in_universe_missing_prices": sample(&in_universe_only),
            "priced_not_in_universe": sample(&price_only),
            "date_range": range_or_null(first_day, last_day),
        }),
    )
}

/// Calendar gaps: trading days a symbol lacks *between* its own first and last
/// observation (holes), as opposed to a legitimately-ended (delisted) tail.
fn check_calendar_gaps(symbols: &[String], closes: Option<&yuzu_core::panel::Panel>) -> Check {
    let Some(panel) = closes else {
        return Check::new(
            "calendar_gaps",
            Status::Ok,
            "no prices to check",
            Value::Null,
        );
    };
    // Map each day to its index in the union calendar to count interior holes.
    let mut holes_total = 0usize;
    let mut symbols_with_holes = 0usize;
    let mut worst: Vec<Value> = Vec::new();
    for (col, sym) in symbols.iter().enumerate() {
        let pts = symbol_points(panel, col);
        if pts.len() < 2 {
            continue;
        }
        // Interior span in calendar-row terms: rows between first & last obs that
        // this symbol has no value on.
        let first_row = panel.dates.iter().position(|&d| d == pts[0].0).unwrap_or(0);
        let last_row = panel
            .dates
            .iter()
            .rposition(|&d| d == pts[pts.len() - 1].0)
            .unwrap_or(0);
        let span = last_row - first_row + 1;
        let holes = span - pts.len();
        if holes > 0 {
            holes_total += holes;
            symbols_with_holes += 1;
            if worst.len() < 10 {
                worst.push(json!({ "symbol": sym, "holes": holes }));
            }
        }
    }
    let status = if holes_total > 0 {
        Status::Warn
    } else {
        Status::Ok
    };
    Check::new(
        "calendar_gaps",
        status,
        format!("{holes_total} interior gaps across {symbols_with_holes} symbols"),
        json!({
            "symbols_with_holes": symbols_with_holes,
            "total_holes": holes_total,
            "worst": worst,
        }),
    )
}

/// Adjustment sanity: an overnight |return| above [`JUMP_THRESHOLD`] on adjacent
/// observations flags a candidate un-adjusted split or bad tick.
fn check_adjustment(symbols: &[String], closes: Option<&yuzu_core::panel::Panel>) -> Check {
    let Some(panel) = closes else {
        return Check::new("adjustment", Status::Ok, "no prices to check", Value::Null);
    };
    let mut flagged: Vec<Value> = Vec::new();
    let mut count = 0usize;
    for (col, sym) in symbols.iter().enumerate() {
        let pts = symbol_points(panel, col);
        for w in pts.windows(2) {
            let (d0, c0) = w[0];
            let (d1, c1) = w[1];
            if c0 <= 0.0 {
                continue;
            }
            let ret = c1 / c0 - 1.0;
            if ret.abs() > JUMP_THRESHOLD {
                count += 1;
                if flagged.len() < 10 {
                    flagged.push(json!({
                        "symbol": sym,
                        "from_day": d0,
                        "to_day": d1,
                        "return_pct": (ret * 1000.0).round() / 10.0,
                    }));
                }
            }
        }
    }
    let status = if count > 0 { Status::Warn } else { Status::Ok };
    Check::new(
        "adjustment",
        status,
        format!(
            "{count} overnight moves > {:.0}% (candidate un-adjusted splits / bad ticks)",
            JUMP_THRESHOLD * 100.0
        ),
        json!({ "flagged": count, "examples": flagged }),
    )
}

/// Survivorship: whether any symbol's price file ends before the universe's last
/// trading day (a delisting proxy). A tree where *nothing* ends early is likely
/// survivors-only and biases every backtest.
fn check_survivorship(symbols: &[String], closes: Option<&yuzu_core::panel::Panel>) -> Check {
    let Some(panel) = closes else {
        return Check::new(
            "survivorship",
            Status::Ok,
            "no prices to check",
            Value::Null,
        );
    };
    let Some(&global_last) = panel.dates.last() else {
        return Check::new("survivorship", Status::Ok, "empty calendar", Value::Null);
    };
    let mut ended_early = 0usize;
    for col in 0..symbols.len() {
        let pts = symbol_points(panel, col);
        if let Some(&(last, _)) = pts.last() {
            if last < global_last {
                ended_early += 1;
            }
        }
    }
    // Only a multi-symbol universe can meaningfully look "survivors-only".
    let status = if ended_early == 0 && symbols.len() > 1 {
        Status::Warn
    } else {
        Status::Ok
    };
    let summary = if ended_early == 0 {
        format!("no symbols end before {global_last} — universe may be survivors-only")
    } else {
        format!("{ended_early} symbols end before the last trading day (delisted tails)")
    };
    Check::new(
        "survivorship",
        status,
        summary,
        json!({ "ended_early": ended_early, "last_trading_day": global_last }),
    )
}

/// NaN density: fundamental fields the plan never populated, and snapshot-factor
/// panels that are missing or entirely NaN (the #132 all-NaN smell).
fn check_nan_density(
    src: &LocalSource,
    symbols: &[String],
    fund: &FundScan,
    from: i32,
    to: i32,
) -> Check {
    // Fundamental fields never seen with a finite value across every symbol.
    let empty_fields: Vec<&str> = FUNDAMENTAL_FIELDS
        .iter()
        .copied()
        .filter(|f| !fund.fields_seen.contains(*f))
        .collect();

    // Snapshot-factor panels: absent, or present-but-all-NaN.
    let mut missing_panels: Vec<&str> = Vec::new();
    let mut empty_panels: Vec<&str> = Vec::new();
    if !symbols.is_empty() {
        for name in FACTOR_PANEL_FIELDS {
            match load_combined_panel(src, name, symbols, from, to, PANELS_DIR) {
                Ok(Some(panel)) => {
                    if panel.data.iter().all(|v| v.is_nan()) {
                        empty_panels.push(name);
                    }
                }
                Ok(None) => missing_panels.push(name),
                Err(_) => missing_panels.push(name),
            }
        }
    }

    let has_fund_data = fund.file_count > 0;
    let anything_wrong = (has_fund_data && !empty_fields.is_empty()) || !empty_panels.is_empty();
    let status = if anything_wrong {
        Status::Warn
    } else {
        Status::Ok
    };
    let summary = if !has_fund_data && missing_panels.len() == FACTOR_PANEL_FIELDS.len() {
        "no fundamentals or factor panels present".to_string()
    } else {
        format!(
            "{} fundamental fields never populated; {} factor panels all-NaN, {} missing",
            if has_fund_data { empty_fields.len() } else { 0 },
            empty_panels.len(),
            missing_panels.len()
        )
    };
    Check::new(
        "nan_density",
        status,
        summary,
        json!({
            "fundamentals_files": fund.file_count,
            "empty_fundamental_fields": if has_fund_data { empty_fields } else { Vec::new() },
            "all_nan_factor_panels": empty_panels,
            "missing_factor_panels": missing_panels,
        }),
    )
}

/// PIT lag (lookahead heuristic): `report_event` days should be *filing* days,
/// which lag the fiscal period-end by ~30–90 days (#131). A high fraction landing
/// exactly on a calendar month-end (every fiscal period-end is a month-end) is
/// the smell that snapshots were stamped on period-end instead.
fn check_pit_lag(fund: &FundScan) -> Check {
    let total = fund.report_event_days.len();
    if total == 0 {
        return Check::new(
            "pit_lag",
            Status::Ok,
            "no filing events to check",
            json!({ "report_events": 0 }),
        );
    }
    let on_month_end = fund
        .report_event_days
        .iter()
        .filter(|&&d| is_month_end(d))
        .count();
    let fraction = on_month_end as f64 / total as f64;
    let status = if fraction > LOOKAHEAD_FRACTION {
        Status::Warn
    } else {
        Status::Ok
    };
    let summary = format!(
        "{on_month_end}/{total} filing days on a month-end ({:.0}%){}",
        fraction * 100.0,
        if status == Status::Warn {
            " — possible period-end (lookahead) stamping"
        } else {
            ""
        }
    );
    Check::new(
        "pit_lag",
        status,
        summary,
        json!({
            "report_events": total,
            "on_month_end": on_month_end,
            "month_end_fraction": (fraction * 1000.0).round() / 1000.0,
        }),
    )
}

/// Index membership: for any `panels/in_*.csv.gz`, the count of members over time
/// (sanity vs a known index size). Informational unless a panel is all-empty.
fn check_index_membership(
    root: &Path,
    src: &LocalSource,
    symbols: &[String],
    from: i32,
    to: i32,
) -> Check {
    let names = list_membership_panels(root);
    if names.is_empty() {
        return Check::new(
            "index_membership",
            Status::Ok,
            "no panels/in_*.csv.gz present",
            Value::Null,
        );
    }
    let mut reports: Vec<Value> = Vec::new();
    let mut status = Status::Ok;
    for name in &names {
        // Load over the union of the tree's symbols so every member column shows.
        let panel = match load_combined_panel(src, name, symbols, from, to, PANELS_DIR) {
            Ok(Some(p)) => p,
            _ => {
                status = status.max(Status::Warn);
                reports.push(json!({ "panel": name, "error": "unreadable" }));
                continue;
            }
        };
        let (mut min_c, mut max_c, mut last_c) = (usize::MAX, 0usize, 0usize);
        for r in 0..panel.dates.len() {
            let members = (0..panel.symbols.len())
                .filter(|&c| panel.data[[r, c]] == 1.0)
                .count();
            min_c = min_c.min(members);
            max_c = max_c.max(members);
            last_c = members;
        }
        if panel.dates.is_empty() || max_c == 0 {
            status = status.max(Status::Warn);
        }
        reports.push(json!({
            "panel": name,
            "days": panel.dates.len(),
            "min_members": if panel.dates.is_empty() { 0 } else { min_c },
            "max_members": max_c,
            "last_members": last_c,
        }));
    }
    Check::new(
        "index_membership",
        status,
        format!("{} membership panel(s) checked", names.len()),
        json!({ "panels": reports }),
    )
}

// ── fundamentals scan ────────────────────────────────────────────────────────

/// Result of the single-pass fundamentals scan.
struct FundScan {
    /// Fundamental field names seen with ≥1 finite value across all symbols.
    fields_seen: BTreeSet<String>,
    /// Every `report_event == 1` day (windowed to `[from, to]`).
    report_event_days: Vec<i32>,
    /// Number of fundamentals files read.
    file_count: usize,
}

/// Parse every `fundamentals/{SYM}.csv.gz` once: which fields are ever populated
/// and every filing (`report_event`) day. Fail-soft per file.
fn scan_fundamentals(root: &Path, src: &LocalSource, from: i32, to: i32) -> FundScan {
    let mut scan = FundScan {
        fields_seen: BTreeSet::new(),
        report_event_days: Vec::new(),
        file_count: 0,
    };
    for sym in list_dir_stems(&root.join(FUNDAMENTALS_DIR)) {
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

// ── small I/O + parsing helpers ──────────────────────────────────────────────

/// Load and decode `tracked/universe.csv.gz` into a `symbol → sector` map.
fn load_industry_map(src: &LocalSource) -> Option<std::collections::HashMap<String, String>> {
    let bytes = try_get(src, "tracked/universe")?;
    let map = parse_industry_csv(&decode_text(&bytes));
    (!map.is_empty()).then_some(map)
}

/// `src.get` for `key` trying `.csv.gz` then `.csv` (the formats the sync writes).
fn try_get(src: &LocalSource, key_stem: &str) -> Option<Vec<u8>> {
    for ext in [".csv.gz", ".csv"] {
        if let Ok(Some(bytes)) = src.get(&format!("{key_stem}{ext}")) {
            return Some(bytes);
        }
    }
    None
}

/// File stems (symbol names) under `dir`, stripping `.csv.gz` / `.csv`.
fn list_dir_stems(dir: &Path) -> Vec<String> {
    const EXTS: &[&str] = &[".csv.gz", ".csv"];
    let mut out = BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in rd.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stem) = EXTS.iter().find_map(|e| name.strip_suffix(e)) {
                    out.insert(stem.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Series names of `panels/in_*.{csv.gz,csv}` (index membership panels).
fn list_membership_panels(root: &Path) -> Vec<String> {
    list_dir_stems(&root.join(PANELS_DIR))
        .into_iter()
        .filter(|s| s.starts_with("in_"))
        .collect()
}

/// Decode bytes that may be gzip (`.csv.gz`) or plain UTF-8 text.
fn decode_text(bytes: &[u8]) -> String {
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
fn parse_day(s: &str) -> Option<i32> {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    (digits.len() == 8).then(|| digits.parse().ok()).flatten()
}

/// Parse a cell to a finite f64, or `None` for empty / non-finite.
fn parse_finite(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Whether `day` (YYYYMMDD) is the last calendar day of its month — every fiscal
/// period-end is a month-end, so a filing date landing here is the lookahead smell.
fn is_month_end(day: i32) -> bool {
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

/// At most 20 sample names, as a JSON array (keeps the report bounded).
fn sample(names: &[&str]) -> Vec<String> {
    names.iter().take(20).map(|s| s.to_string()).collect()
}

fn range_or_null(first: i32, last: i32) -> Value {
    if first == i32::MAX {
        Value::Null
    } else {
        json!({ "first_day": first, "last_day": last })
    }
}

/// Render the report as a compact human-readable table (the default output).
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

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Ok => "OK",
        Status::Warn => "WARN",
        Status::Fail => "FAIL",
    }
}
