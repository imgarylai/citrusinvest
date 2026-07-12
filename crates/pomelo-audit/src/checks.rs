//! The individual data-quality checks. Each `check_*` returns a [`Check`];
//! `run_data_audit` (in `lib.rs`) sequences them and folds their statuses.

use std::collections::BTreeSet;

use pomelo_data::{
    load_combined_panel, ObjectLister, ObjectSource, FACTOR_PANEL_FIELDS, FUNDAMENTAL_FIELDS,
    PANELS_DIR,
};
use serde_json::{json, Value};

use crate::report::{is_month_end, range_or_null, sample, Check, Status};
use crate::scan::{list_membership_panels, load_industry_map, FundScan};

/// Overnight |return| above this flags a candidate un-adjusted split / bad tick.
const JUMP_THRESHOLD: f64 = 0.5;
/// Fraction of filing (`report_event`) days on a calendar month-end above which
/// the PIT-lag check warns of possible period-end (lookahead) stamping.
const LOOKAHEAD_FRACTION: f64 = 0.5;

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
pub(crate) fn check_coverage<S: ObjectSource>(
    src: &S,
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
pub(crate) fn check_calendar_gaps(
    symbols: &[String],
    closes: Option<&yuzu_core::panel::Panel>,
) -> Check {
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
pub(crate) fn check_adjustment(
    symbols: &[String],
    closes: Option<&yuzu_core::panel::Panel>,
) -> Check {
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
pub(crate) fn check_survivorship(
    symbols: &[String],
    closes: Option<&yuzu_core::panel::Panel>,
) -> Check {
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
pub(crate) fn check_nan_density<S: ObjectSource>(
    src: &S,
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
pub(crate) fn check_pit_lag(fund: &FundScan) -> Check {
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
pub(crate) fn check_index_membership<S: ObjectSource + ObjectLister>(
    src: &S,
    symbols: &[String],
    from: i32,
    to: i32,
) -> Check {
    let names = list_membership_panels(src);
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
