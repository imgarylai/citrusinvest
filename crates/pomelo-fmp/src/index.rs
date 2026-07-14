//! Point-in-time index membership reconstruction (#125).
//!
//! FMP exposes an index's **current** constituents (`sp-500`) and a **change
//! log** of add/remove events (`historical-sp-500`). Neither is a per-date
//! snapshot, so PIT membership ("who was in the S&P 500 on date T") is
//! *reconstructed*: start from today's set and replay the change log backwards,
//! undoing every event dated after T.
//!
//! From that we produce two artifacts for a survivorship-honest index backtest:
//!
//! - **`ever_members(from, to)`** — every symbol that was a member at any point
//!   in the window; the sync universe (so each one's prices, incl. names that
//!   later left / delisted, are fetched).
//! - **`membership_panel(calendar, columns)`** — a `dates × symbols` 0/1 panel
//!   (`in_sp500`) written to `panels/`. Multiply a signal by it
//!   (`signal * in_sp500`) so a strategy holds a name only while it was a
//!   member and flattens it (to an explicit `0.0`) the day it leaves. (Avoid
//!   `mask(signal, in_sp500)` as the *outer* op: `mask` NaN-outs non-members
//!   and the NAV loop forward-fills a NaN position, so a masked name is held
//!   after it leaves the index — see `docs/data-layout.md` §8. `lemon run
//!   --index sp500` applies the correct spelling automatically.)
//!
//! ## Honest weakness
//!
//! Reconstruction is only as good as the change log. Older FMP rows drop the
//! `removedTicker` / `reason`, so backward replay **drifts the further back you
//! go** — membership is reliable in recent years and degrades pre-2000s. Cover
//! is **named indices only** (S&P 500 / Nasdaq / Dow); a full-market PIT
//! universe has no clean FMP endpoint and is out of scope (#53).

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
use super::FMP_BASE;

/// An index we can reconstruct PIT membership for. Each maps to FMP's
/// current-snapshot + historical-change-log endpoints and a membership series
/// name (`in_<index>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Index {
    Sp500,
    Nasdaq,
    DowJones,
}

/// Membership series names auto-loaded by the CLI (`load_ctx`) from `panels/`,
/// so `signal * in_sp500` works on the `run` / `sweep` path. Kept in sync
/// with [`Index::series_name`] (asserted in tests).
pub const MEMBERSHIP_SERIES: &[&str] = &["in_sp500", "in_nasdaq", "in_dowjones"];

impl Index {
    /// Parse the CLI spelling (`sp500` / `nasdaq` / `dowjones`, case-insensitive;
    /// a few common aliases accepted).
    pub fn parse(s: &str) -> Option<Index> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sp500" | "sp-500" | "spx" | "spy" => Some(Index::Sp500),
            "nasdaq" | "ndx" | "nasdaq100" => Some(Index::Nasdaq),
            "dowjones" | "dow" | "djia" | "dji" => Some(Index::DowJones),
            _ => None,
        }
    }

    fn current_endpoint(self) -> &'static str {
        match self {
            Index::Sp500 => "sp-500",
            Index::Nasdaq => "nasdaq",
            Index::DowJones => "dow-jones",
        }
    }

    fn historical_endpoint(self) -> &'static str {
        match self {
            Index::Sp500 => "historical-sp-500",
            Index::Nasdaq => "historical-nasdaq",
            Index::DowJones => "historical-dow-jones",
        }
    }

    /// The membership panel / series name (`in_sp500`, …).
    pub fn series_name(self) -> &'static str {
        match self {
            Index::Sp500 => "in_sp500",
            Index::Nasdaq => "in_nasdaq",
            Index::DowJones => "in_dowjones",
        }
    }
}

/// One membership change: on `date`, `added` joined the index and/or `removed`
/// left it (either side may be absent for a one-sided event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub date: i32,
    pub added: Option<String>,
    pub removed: Option<String>,
}

fn nonempty(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn parse_current(rows: &[Value]) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|r| nonempty(r.as_object()?, "symbol"))
        .collect()
}

/// Parse the change log. Each row carries an effective `date`, the added ticker
/// in `symbol`, and the removed ticker in `removedTicker` (either may be blank).
/// Rows with no parseable date are dropped. Returned **sorted ascending** by date.
pub(crate) fn parse_changes(rows: &[Value]) -> Vec<Change> {
    let mut out: Vec<Change> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            let date = obj
                .get("date")
                .and_then(Value::as_str)
                .and_then(iso_to_i32)?;
            let added = nonempty(obj, "symbol");
            let removed = nonempty(obj, "removedTicker");
            if added.is_none() && removed.is_none() {
                return None;
            }
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
    /// Change log, sorted ascending by date.
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
        let cur = fetcher.get_rows(&Self::url(index.current_endpoint(), api_key))?;
        let hist = fetcher.get_rows(&Self::url(index.historical_endpoint(), api_key))?;
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

    fn url(endpoint: &str, key: &str) -> String {
        format!("{FMP_BASE}/stable/{endpoint}?apikey={key}")
    }

    pub fn series_name(&self) -> &'static str {
        self.index.series_name()
    }

    /// Membership as-of `date`: today's set with every change dated after `date`
    /// undone (an add is removed, a removal is restored). Undo walks newest→oldest
    /// so repeated add/remove of the same ticker resolves correctly.
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
    /// universe. Members at `from`, plus every ticker added or removed within the
    /// window (a removal means the name was a member up to that date). Sorted.
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
    /// column symbol was a member, else `0.0`. `calendar` must be ascending
    /// (typically the synced price trading days); `columns` are the panel's
    /// symbols (typically [`ever_members`]).
    pub fn membership_panel(&self, calendar: &[i32], columns: &[String]) -> Result<Panel, String> {
        if calendar.is_empty() {
            return Err("empty calendar for membership panel".to_string());
        }
        let mut set = self.members_asof(calendar[0]);
        // First change strictly after the calendar start (earlier ones are folded
        // into `members_asof` above); applied forward as the calendar advances.
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

/// The trading calendar of a synced tree: the ascending, unique dates of the
/// `close` panel over `[from, to]`. Used to place an index membership panel on
/// exactly the days prices exist for.
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

/// Reconstruct and write an index membership panel (`panels/in_sp500.csv.gz`,
/// etc.) over the synced tree's trading calendar. Columns are the index's
/// `ever_members(from, to)` — the same universe you should have synced. Returns
/// `(days, symbols)` in the written panel.
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

    /// Fixture with a ticker that leaves then rejoins, so undo order matters:
    /// AAA out at 2015-06-01 (DDD in), AAA back at 2020-06-01 (DDD out).
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
    fn members_asof_replays_the_log_backwards() {
        let m = fixture();
        // Today: AAA rejoined, DDD gone.
        assert_eq!(m.members_asof(20990101), set(&["AAA", "BBB", "CCC"]));
        // Between the two events: AAA out, DDD in (undo order must be newest-first).
        assert_eq!(m.members_asof(20180101), set(&["BBB", "CCC", "DDD"]));
        // Before either event: original set.
        assert_eq!(m.members_asof(20100101), set(&["AAA", "BBB", "CCC"]));
        // On an event day the change is already effective (2020-06-01 = AAA back).
        assert_eq!(m.members_asof(20200601), set(&["AAA", "BBB", "CCC"]));
        assert_eq!(m.members_asof(20200531), set(&["BBB", "CCC", "DDD"]));
    }

    #[test]
    fn ever_members_is_the_windowed_union() {
        let m = fixture();
        // Whole span: every name that ever appeared, incl. the one that left.
        assert_eq!(
            m.ever_members(20100101, 20250101),
            vec!["AAA", "BBB", "CCC", "DDD"]
        );
        // A window with no events collapses to the members at its start.
        assert_eq!(
            m.ever_members(20210101, 20220101),
            vec!["AAA", "BBB", "CCC"]
        );
    }

    #[test]
    fn membership_panel_is_per_day_zero_one() {
        let m = fixture();
        let cols: Vec<String> = ["AAA", "BBB", "CCC", "DDD"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cal = [20180101, 20200601, 20210101];
        let p = m.membership_panel(&cal, &cols).unwrap();
        // 2018: AAA out, DDD in.
        assert_eq!(p.data.row(0).to_vec(), vec![0.0, 1.0, 1.0, 1.0]);
        // 2020-06-01 (event day): AAA back, DDD out.
        assert_eq!(p.data.row(1).to_vec(), vec![1.0, 1.0, 1.0, 0.0]);
        // 2021: unchanged since the event.
        assert_eq!(p.data.row(2).to_vec(), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(p.dates, cal.to_vec());
    }

    #[test]
    fn parse_changes_drops_undated_and_sorts_ascending() {
        let rows = vec![
            serde_json::json!({"date":"2020-06-01","symbol":"AAA","removedTicker":"DDD"}),
            serde_json::json!({"date":"2015-06-01","symbol":"DDD","removedTicker":"AAA"}),
            serde_json::json!({"symbol":"NODATE","removedTicker":"X"}), // no date → dropped
            serde_json::json!({"date":"2019-01-01","symbol":"","removedTicker":""}), // both empty → dropped
        ];
        let changes = parse_changes(&rows);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].date, 20150601); // ascending
        assert_eq!(changes[1].date, 20200601);
        assert_eq!(changes[0].added.as_deref(), Some("DDD"));
        assert_eq!(changes[0].removed.as_deref(), Some("AAA"));
    }

    #[test]
    fn index_parse_and_series_names_are_consistent() {
        assert_eq!(Index::parse("SP500"), Some(Index::Sp500));
        assert_eq!(Index::parse("nasdaq"), Some(Index::Nasdaq));
        assert_eq!(Index::parse("dow"), Some(Index::DowJones));
        assert_eq!(Index::parse("russell2000"), None);
        // Every index's series name is in the CLI auto-load list.
        for idx in [Index::Sp500, Index::Nasdaq, Index::DowJones] {
            assert!(MEMBERSHIP_SERIES.contains(&idx.series_name()));
        }
    }
}
