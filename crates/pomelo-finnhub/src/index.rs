//! Point-in-time S&P 500 membership from Finnhub index constituents (#229).
//!
//! Finnhub exposes an index's **current** constituents (`index/constituents`)
//! and a **change log** of add/remove events (`index/historical-constituents`).
//! Neither is a per-date snapshot, so PIT membership ("who was in the S&P 500 on
//! date T") is *reconstructed* the same way as `pomelo-fmp`: start from today's
//! set and replay the change log backwards, undoing every event dated after T.
//!
//! This is the main reason Finnhub can back an index-honest backtest that Alpha
//! Vantage cannot (AV #217 had no real constituents, so it refused to fake
//! `in_sp500`; spike #208 rates Finnhub index PIT **Y**).
//!
//! Two artifacts feed a survivorship-honest index backtest:
//! - **`ever_members(from, to)`** — every symbol that was a member at any point
//!   in the window; the sync universe (so leavers/delistings get priced too).
//! - **`membership_panel(calendar, columns)`** — a `dates × symbols` 0/1 panel
//!   (`in_sp500`) written to `panels/` for `signal * in_sp500`.
//!
//! ## Honest weakness
//!
//! Reconstruction is only as good as the change log; Finnhub's depth thins the
//! further back you go (spike #208), so membership is reliable in recent years
//! and degrades in older history. v1 covers **S&P 500 only** (`^GSPC`).

use std::collections::BTreeSet;
use std::path::Path;

use pomelo_data::{
    list_symbols, load_panel, write_combined_panel, Field, LocalSource, ObjectSink, PANELS_DIR,
    PRICES_DIR,
};
use serde_json::Value;
use yuzu_core::panel::Panel;

use super::config::SyncConfig;
use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::FINNHUB_BASE;

/// An index we can reconstruct PIT membership for. v1: S&P 500 only — the one
/// Finnhub's historical constituents were validated on (spike #208).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Sp500,
}

/// Membership series names auto-loaded by the CLI (`load_ctx`) from `panels/`,
/// so `signal * in_sp500` works on the `run` / `sweep` path.
pub const MEMBERSHIP_SERIES: &[&str] = &["in_sp500"];

impl Index {
    /// Parse the CLI spelling (`sp500`, case-insensitive; common aliases).
    pub fn parse(s: &str) -> Option<Index> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sp500" | "sp-500" | "spx" | "spy" | "gspc" => Some(Index::Sp500),
            _ => None,
        }
    }

    /// Finnhub index symbol (URL-encoded `^` → `%5E`).
    fn finnhub_symbol(self) -> &'static str {
        match self {
            Index::Sp500 => "%5EGSPC",
        }
    }

    /// The membership panel / series name (`in_sp500`).
    pub fn series_name(self) -> &'static str {
        match self {
            Index::Sp500 => "in_sp500",
        }
    }
}

/// One membership change: on `date`, `added` joined the index and/or `removed`
/// left it (Finnhub events are one-sided — a single `add` or `remove`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub date: i32,
    pub added: Option<String>,
    pub removed: Option<String>,
}

/// Trim + uppercase a symbol into a layout ticker, dropping empties.
fn norm(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_ascii_uppercase())
    }
}

/// Parse the current-constituents payload (`{"constituents":["AAPL",…]}`, or a
/// `data[].symbol` fallback).
pub(crate) fn parse_current(value: &Value) -> BTreeSet<String> {
    let Some(obj) = value.as_object() else {
        return BTreeSet::new();
    };
    if let Some(arr) = obj.get("constituents").and_then(Value::as_array) {
        return arr.iter().filter_map(norm).collect();
    }
    if let Some(arr) = obj.get("data").and_then(Value::as_array) {
        return arr
            .iter()
            .filter_map(|r| norm(r.as_object()?.get("symbol")?))
            .collect();
    }
    BTreeSet::new()
}

/// Parse the change log (`{"historicalConstituents":[{action,symbol,date},…]}`).
/// Rows with no parseable date or unknown action are dropped. Sorted ascending.
pub(crate) fn parse_changes(value: &Value) -> Vec<Change> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let Some(arr) = obj.get("historicalConstituents").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<Change> = arr
        .iter()
        .filter_map(|r| {
            let o = r.as_object()?;
            let date = o.get("date").and_then(Value::as_str).and_then(iso_to_i32)?;
            let sym = norm(o.get("symbol")?)?;
            let action = o
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let (added, removed) = match action.as_str() {
                "add" | "added" => (Some(sym), None),
                "remove" | "removed" => (None, Some(sym)),
                _ => return None,
            };
            Some(Change {
                date,
                added,
                removed,
            })
        })
        .collect();
    out.sort_by_key(|c| c.date);
    out
}

/// The reconstructor: the current constituent set plus the change log (ascending).
pub struct IndexMembership {
    index: Index,
    current: BTreeSet<String>,
    changes: Vec<Change>,
}

impl IndexMembership {
    /// Fetch the current snapshot and the change log for `index`.
    pub fn fetch<H: HttpClient>(
        http: &H,
        api_key: &str,
        index: Index,
        cfg: &SyncConfig,
    ) -> Result<IndexMembership, String> {
        let fetcher = Fetcher::new(http, cfg);
        let sym = index.finnhub_symbol();
        let cur = fetcher.get_json(&format!(
            "{FINNHUB_BASE}/index/constituents?symbol={sym}&token={api_key}"
        ))?;
        let hist = fetcher.get_json(&format!(
            "{FINNHUB_BASE}/index/historical-constituents?symbol={sym}&token={api_key}"
        ))?;
        for v in [&cur, &hist] {
            if let Some(err) = v
                .as_object()
                .and_then(|o| o.get("error"))
                .and_then(Value::as_str)
            {
                return Err(format!("Finnhub error: {err}"));
            }
        }
        let current = parse_current(&cur);
        if current.is_empty() {
            return Err(format!(
                "index '{}' returned no current constituents",
                index.series_name()
            ));
        }
        Ok(IndexMembership {
            index,
            current,
            changes: parse_changes(&hist),
        })
    }

    pub fn series_name(&self) -> &'static str {
        self.index.series_name()
    }

    /// Membership as-of `date`: today's set with every change dated after `date`
    /// undone (an add is removed, a removal is restored). Walks newest→oldest so
    /// repeated add/remove of the same ticker resolves correctly.
    pub(crate) fn members_asof(&self, date: i32) -> BTreeSet<String> {
        let mut set = self.current.clone();
        for c in self.changes.iter().rev() {
            if c.date <= date {
                break;
            }
            if let Some(a) = &c.added {
                set.remove(a);
            }
            if let Some(r) = &c.removed {
                set.insert(r.clone());
            }
        }
        set
    }

    /// Every symbol that was a member at any point in `[from, to]` — the sync
    /// universe. Sorted.
    pub fn ever_members(&self, from: i32, to: i32) -> Vec<String> {
        let mut set = self.members_asof(from);
        for c in &self.changes {
            if c.date < from || c.date > to {
                continue;
            }
            if let Some(a) = &c.added {
                set.insert(a.clone());
            }
            if let Some(r) = &c.removed {
                set.insert(r.clone());
            }
        }
        set.into_iter().collect()
    }

    /// A `calendar × columns` 0/1 membership panel: cell is `1.0` on the days the
    /// column symbol was a member, else `0.0`. `calendar` must be ascending.
    pub fn membership_panel(&self, calendar: &[i32], columns: &[String]) -> Result<Panel, String> {
        if calendar.is_empty() {
            return Err("empty calendar for membership panel".to_string());
        }
        let mut set = self.members_asof(calendar[0]);
        let mut p = self.changes.partition_point(|c| c.date <= calendar[0]);
        let mut rows: Vec<Vec<f64>> = Vec::with_capacity(calendar.len());
        for &day in calendar {
            while p < self.changes.len() && self.changes[p].date <= day {
                let c = &self.changes[p];
                if let Some(a) = &c.added {
                    set.insert(a.clone());
                }
                if let Some(r) = &c.removed {
                    set.remove(r);
                }
                p += 1;
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

/// The trading calendar of a synced tree: ascending unique dates of the `close`
/// panel over `[from, to]`.
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

/// Reconstruct and write `panels/in_sp500.csv.gz` over the synced tree's trading
/// calendar. Columns are the index's `ever_members(from, to)`. Returns
/// `(days, symbols)`.
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

    /// AAA leaves 2015-06-01 (DDD in), AAA rejoins 2020-06-01 (DDD out).
    fn fixture() -> IndexMembership {
        IndexMembership {
            index: Index::Sp500,
            current: ["AAA", "BBB", "CCC"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            changes: vec![
                Change {
                    date: 20150601,
                    added: Some("DDD".into()),
                    removed: Some("AAA".into()),
                },
                Change {
                    date: 20200601,
                    added: Some("AAA".into()),
                    removed: Some("DDD".into()),
                },
            ],
        }
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_current_reads_constituents_and_data() {
        let v = json!({"symbol": "^GSPC", "constituents": ["aapl", " MSFT ", ""]});
        assert_eq!(parse_current(&v), set(&["AAPL", "MSFT"]));
        let v2 = json!({"data": [{"symbol": "ibm"}, {"symbol": ""}]});
        assert_eq!(parse_current(&v2), set(&["IBM"]));
        assert!(parse_current(&json!({})).is_empty());
    }

    #[test]
    fn parse_changes_maps_actions_and_sorts() {
        let v = json!({"historicalConstituents": [
            {"action": "add", "symbol": "aaa", "date": "2020-06-01"},
            {"action": "remove", "symbol": "ddd", "date": "2015-06-01"},
            {"action": "add", "symbol": "x", "date": "bad-date"},
            {"action": "weird", "symbol": "y", "date": "2021-01-01"}
        ]});
        let changes = parse_changes(&v);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].date, 20150601);
        assert_eq!(changes[0].removed.as_deref(), Some("DDD"));
        assert_eq!(changes[1].date, 20200601);
        assert_eq!(changes[1].added.as_deref(), Some("AAA"));
        assert!(parse_changes(&json!({})).is_empty());
    }

    #[test]
    fn members_asof_replays_backwards() {
        let m = fixture();
        assert_eq!(m.members_asof(20990101), set(&["AAA", "BBB", "CCC"]));
        assert_eq!(m.members_asof(20180101), set(&["BBB", "CCC", "DDD"]));
        assert_eq!(m.members_asof(20100101), set(&["AAA", "BBB", "CCC"]));
        assert_eq!(m.members_asof(20200601), set(&["AAA", "BBB", "CCC"]));
        assert_eq!(m.members_asof(20200531), set(&["BBB", "CCC", "DDD"]));
    }

    #[test]
    fn ever_members_windowed_union() {
        let m = fixture();
        assert_eq!(
            m.ever_members(20100101, 20250101),
            vec!["AAA", "BBB", "CCC", "DDD"]
        );
        assert_eq!(
            m.ever_members(20210101, 20220101),
            vec!["AAA", "BBB", "CCC"]
        );
    }

    #[test]
    fn membership_panel_per_day_zero_one() {
        let m = fixture();
        let cols: Vec<String> = ["AAA", "BBB", "CCC", "DDD"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cal = [20180101, 20200601, 20210101];
        let p = m.membership_panel(&cal, &cols).unwrap();
        assert_eq!(p.data.row(0).to_vec(), vec![0.0, 1.0, 1.0, 1.0]);
        assert_eq!(p.data.row(1).to_vec(), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(p.data.row(2).to_vec(), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(p.dates, cal.to_vec());
        assert!(m.membership_panel(&[], &cols).is_err());
    }

    #[test]
    fn index_parse_and_series_name() {
        assert_eq!(Index::parse("SPX"), Some(Index::Sp500));
        assert_eq!(Index::parse("gspc"), Some(Index::Sp500));
        assert_eq!(Index::parse("russell2000"), None);
        assert!(MEMBERSHIP_SERIES.contains(&Index::Sp500.series_name()));
    }

    #[test]
    fn fetch_wires_endpoints() {
        use crate::http::{HttpClient, HttpError};
        use std::time::Duration;

        struct RouteHttp;
        impl HttpClient for RouteHttp {
            fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
                if url.contains("historical-constituents") {
                    Ok(br#"{"historicalConstituents":[{"action":"remove","symbol":"DDD","date":"2015-06-01"}]}"#.to_vec())
                } else if url.contains("index/constituents") {
                    Ok(br#"{"constituents":["AAA","BBB","CCC"]}"#.to_vec())
                } else {
                    Ok(b"{}".to_vec())
                }
            }
        }
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let m = IndexMembership::fetch(&RouteHttp, "tok", Index::Sp500, &cfg).unwrap();
        assert_eq!(m.series_name(), "in_sp500");
        assert_eq!(m.ever_members(20100101, 20250101).len(), 4); // AAA,BBB,CCC + DDD
    }

    #[test]
    fn fetch_empty_current_errors() {
        use crate::http::{HttpClient, HttpError};
        use std::time::Duration;
        struct EmptyHttp;
        impl HttpClient for EmptyHttp {
            fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
                Ok(b"{}".to_vec())
            }
        }
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        assert!(IndexMembership::fetch(&EmptyHttp, "tok", Index::Sp500, &cfg).is_err());
    }

    #[test]
    fn fetch_surfaces_error_object_and_http_error() {
        use crate::http::{HttpClient, HttpError};
        use std::time::Duration;

        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };

        struct ErrObj;
        impl HttpClient for ErrObj {
            fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
                Ok(br#"{"error":"You don't have access to this resource."}"#.to_vec())
            }
        }
        match IndexMembership::fetch(&ErrObj, "tok", Index::Sp500, &cfg) {
            Err(e) => assert!(e.contains("access"), "{e}"),
            Ok(_) => panic!("expected error object to surface"),
        }

        struct HttpErr;
        impl HttpClient for HttpErr {
            fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
                Err(HttpError::Status(403))
            }
        }
        assert!(IndexMembership::fetch(&HttpErr, "tok", Index::Sp500, &cfg).is_err());
    }

    #[test]
    fn write_index_membership_writes_panel() {
        use pomelo_data::csv_io::write_series;
        use pomelo_data::csv_io::OhlcvRow;

        let dir = std::env::temp_dir().join("pomelo_fh_index_write");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prices")).unwrap();
        // Two synced symbols give the trading calendar.
        for sym in ["AAA", "BBB"] {
            let rows = vec![
                OhlcvRow {
                    day: 20180101,
                    adj_open: 1.0,
                    adj_high: 1.0,
                    adj_low: 1.0,
                    adj_close: 1.0,
                    volume: 1.0,
                },
                OhlcvRow {
                    day: 20210101,
                    adj_open: 1.0,
                    adj_high: 1.0,
                    adj_low: 1.0,
                    adj_close: 1.0,
                    volume: 1.0,
                },
            ];
            std::fs::write(
                dir.join(format!("prices/{sym}.csv.gz")),
                write_series(&rows).unwrap(),
            )
            .unwrap();
        }
        let m = fixture();
        let (days, cols) = write_index_membership(&dir, &m, 20180101, 20210101).unwrap();
        assert_eq!(days, 2);
        assert!(cols >= 3);
        assert!(dir.join("panels/in_sp500.csv.gz").exists());
    }
}
