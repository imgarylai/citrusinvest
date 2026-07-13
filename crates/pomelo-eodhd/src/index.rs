//! S&P 500 point-in-time membership from EODHD fundamentals snapshots.
//!
//! EODHD provides **current** `Components` and optional **HistoricalComponents**
//! (per-date full membership snapshots) on `GSPC.INDX`. That is more direct
//! than FMP's changelog reconstruction: membership on day T is the latest
//! snapshot with date ≤ T.
//!
//! Writes `panels/in_sp500.csv.gz` for `mask(signal, in_sp500)`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use pomelo_data::{
    list_symbols, load_panel, write_combined_panel, Field, LocalSource, ObjectSink, PANELS_DIR,
    PRICES_DIR,
};
use serde_json::Value;
use yuzu_core::panel::Panel;

use super::config::SyncConfig;
use super::http::Fetcher;
use super::symbol::layout_symbol;
use super::util::{i32_to_iso, iso_to_i32};
use super::HttpClient;
use super::EODHD_BASE;

/// Indices we can materialize. v1: SPX only (EODHD fundamentals package).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Sp500,
}

/// Membership series names auto-loaded by the CLI (`load_ctx`) from `panels/`.
/// Kept aligned with `pomelo_fmp::MEMBERSHIP_SERIES` for SPX.
pub const MEMBERSHIP_SERIES: &[&str] = &["in_sp500"];

impl Index {
    pub fn parse(s: &str) -> Option<Index> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sp500" | "sp-500" | "spx" | "spy" | "gspc" => Some(Index::Sp500),
            _ => None,
        }
    }

    pub fn series_name(self) -> &'static str {
        match self {
            Index::Sp500 => "in_sp500",
        }
    }

    fn eodhd_code(self) -> &'static str {
        match self {
            Index::Sp500 => "GSPC.INDX",
        }
    }
}

/// One dated membership snapshot (codes are layout tickers, e.g. `AAPL`).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub date: i32,
    pub members: BTreeSet<String>,
}

/// Loaded current + historical membership for an index.
pub struct IndexMembership {
    index: Index,
    /// Snapshots sorted ascending by date (includes a synthetic "current" row
    /// when historical data is missing the latest set).
    snapshots: Vec<Snapshot>,
}

impl IndexMembership {
    pub fn series_name(&self) -> &'static str {
        self.index.series_name()
    }

    /// Fetch components (+ optional historical) for `index`.
    pub fn fetch<H: HttpClient>(
        http: &H,
        api_token: &str,
        index: Index,
        cfg: &SyncConfig,
    ) -> Result<IndexMembership, String> {
        let fetcher = Fetcher::new(http, cfg);
        let url = format!(
            "{EODHD_BASE}/fundamentals/{}?api_token={api_token}&fmt=json&historical=1&from={}&to={}",
            index.eodhd_code(),
            i32_to_iso(cfg.from),
            i32_to_iso(cfg.to)
        );
        let value = fetcher.get_json(&url)?;
        let current = parse_components(value.get("Components"));
        let mut by_date = parse_historical_components(value.get("HistoricalComponents"));

        if !current.is_empty() {
            // Anchor current set at `cfg.to` so post-last-history days stay filled.
            by_date.insert(cfg.to, current.clone());
        }

        let mut snapshots: Vec<Snapshot> = by_date
            .into_iter()
            .map(|(date, members)| Snapshot { date, members })
            .collect();
        snapshots.sort_by_key(|s| s.date);

        if snapshots.is_empty() {
            return Err(format!(
                "index '{}' returned no constituents (check Fundamentals plan / GSPC.INDX access)",
                index.series_name()
            ));
        }

        Ok(IndexMembership { index, snapshots })
    }

    /// Membership as-of `date`: last snapshot with `snapshot.date <= date`.
    pub(crate) fn members_asof(&self, date: i32) -> BTreeSet<String> {
        let i = self.snapshots.partition_point(|s| s.date <= date);
        if i == 0 {
            BTreeSet::new()
        } else {
            self.snapshots[i - 1].members.clone()
        }
    }

    /// Every symbol that appears in any snapshot with `date <= to` (includes
    /// the pre-window snapshot that still applies at `from`). Over-inclusion is
    /// intentional for a safe sync universe.
    pub fn ever_members(&self, _from: i32, to: i32) -> Vec<String> {
        let mut set = BTreeSet::new();
        for s in &self.snapshots {
            if s.date <= to {
                set.extend(s.members.iter().cloned());
            }
        }
        set.into_iter().collect()
    }

    /// `calendar × columns` 0/1 membership panel.
    pub fn membership_panel(&self, calendar: &[i32], columns: &[String]) -> Result<Panel, String> {
        if calendar.is_empty() {
            return Err("empty calendar for membership panel".to_string());
        }
        // Start from as-of first calendar day, then walk snapshots forward.
        let mut i = self.snapshots.partition_point(|s| s.date <= calendar[0]);
        let mut set = self.members_asof(calendar[0]);
        let mut rows: Vec<Vec<f64>> = Vec::with_capacity(calendar.len());
        for &day in calendar {
            while i < self.snapshots.len() && self.snapshots[i].date <= day {
                set = self.snapshots[i].members.clone();
                i += 1;
            }
            rows.push(
                columns
                    .iter()
                    .map(|s| if set.contains(s) { 1.0 } else { 0.0 })
                    .collect(),
            );
        }
        Panel::from_rows(calendar.to_vec(), columns.to_vec(), rows).map_err(|e| e.to_string())
    }
}

fn code_from_component(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    let code = obj
        .get("Code")
        .or_else(|| obj.get("code"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    layout_symbol(code).or_else(|| {
        if code.contains('.') {
            None
        } else {
            Some(code.to_ascii_uppercase())
        }
    })
}

/// Parse current `Components` object/array into layout tickers.
pub(crate) fn parse_components(components: Option<&Value>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(components) = components else {
        return out;
    };
    match components {
        Value::Object(map) => {
            for v in map.values() {
                if let Some(c) = code_from_component(v) {
                    out.insert(c);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                if let Some(c) = code_from_component(v) {
                    out.insert(c);
                }
            }
        }
        _ => {}
    }
    out
}

/// Parse `HistoricalComponents` date → member set.
pub(crate) fn parse_historical_components(hist: Option<&Value>) -> BTreeMap<i32, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    let Some(Value::Object(map)) = hist else {
        return out;
    };
    for (date_s, members_v) in map {
        let Some(day) = iso_to_i32(date_s) else {
            continue;
        };
        let set = parse_components(Some(members_v));
        if !set.is_empty() {
            out.insert(day, set);
        }
    }
    out
}

fn trading_calendar(root: &Path, from: i32, to: i32) -> Result<Vec<i32>, String> {
    let syms = list_symbols(root).map_err(|e| e.to_string())?;
    if syms.is_empty() {
        return Err("no synced prices to derive a trading calendar from".to_string());
    }
    let close = load_panel(
        &LocalSource::new(root),
        &syms,
        Field::AdjClose,
        from,
        to,
        PRICES_DIR,
    )
    .map_err(|e| e.to_string())?;
    Ok(close.dates)
}

/// Write `panels/in_sp500.csv.gz` over the tree's trading calendar.
pub fn write_index_membership(
    root: &Path,
    membership: &IndexMembership,
    from: i32,
    to: i32,
) -> Result<(usize, usize), String> {
    let calendar = trading_calendar(root, from, to)?;
    let columns = membership.ever_members(from, to);
    let panel = membership.membership_panel(&calendar, &columns)?;
    let bytes = write_combined_panel(&panel).map_err(|e| e.to_string())?;
    let key = format!("{PANELS_DIR}/{}.csv.gz", membership.series_name());
    LocalSource::new(root)
        .put(&key, &bytes)
        .map_err(|e| e.to_string())?;
    Ok((calendar.len(), columns.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_components_object() {
        let v = json!({
            "0": {"Code": "aapl", "Exchange": "US"},
            "1": {"Code": "MSFT", "Exchange": "US"}
        });
        let set = parse_components(Some(&v));
        assert!(set.contains("AAPL"));
        assert!(set.contains("MSFT"));
    }

    #[test]
    fn historical_and_panel() {
        let hist = json!({
            "2024-01-02": {
                "0": {"Code": "AAA"},
                "1": {"Code": "BBB"}
            },
            "2024-01-04": {
                "0": {"Code": "BBB"},
                "1": {"Code": "CCC"}
            }
        });
        let by = parse_historical_components(Some(&hist));
        assert_eq!(by.len(), 2);
        let m = IndexMembership {
            index: Index::Sp500,
            snapshots: by
                .into_iter()
                .map(|(date, members)| Snapshot { date, members })
                .collect(),
        };
        let cal = [20240102, 20240103, 20240104];
        let cols = vec!["AAA".into(), "BBB".into(), "CCC".into()];
        let panel = m.membership_panel(&cal, &cols).unwrap();
        // day0: AAA,BBB
        assert_eq!(panel.data[[0, 0]], 1.0);
        assert_eq!(panel.data[[0, 1]], 1.0);
        assert_eq!(panel.data[[0, 2]], 0.0);
        // day1: still AAA,BBB
        assert_eq!(panel.data[[1, 0]], 1.0);
        // day2: BBB,CCC
        assert_eq!(panel.data[[2, 0]], 0.0);
        assert_eq!(panel.data[[2, 1]], 1.0);
        assert_eq!(panel.data[[2, 2]], 1.0);

        let ever = m.ever_members(20240102, 20240104);
        assert!(ever.contains(&"AAA".to_string()));
        assert!(ever.contains(&"CCC".to_string()));
        assert!(m.members_asof(20240103).contains("AAA"));
        assert!(!m.members_asof(20240104).contains("AAA"));
    }

    #[test]
    fn index_parse() {
        assert_eq!(Index::parse("spx"), Some(Index::Sp500));
        assert_eq!(Index::parse("nope"), None);
        assert_eq!(Index::Sp500.series_name(), "in_sp500");
        assert!(MEMBERSHIP_SERIES.contains(&"in_sp500"));
    }
}
