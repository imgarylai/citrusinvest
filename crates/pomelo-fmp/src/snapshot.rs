//! Snapshot-factor panels (#132): compute the `FACTOR_PANEL_FIELDS` combined
//! panels from FMP and write them as `panels/{name}.csv.gz`.
//!
//! Phase 1 covers the five **per-symbol** factors — `piotroski_score`,
//! `altman_z`, `fcf_yield`, `analyst_upside_pct`, `consensus_rating`. The
//! cross-sectional `pe_industry_pctile` (needs an industry cohort) is Phase 2.
//!
//! ## Visibility (as_of) semantics
//!
//! FMP's `financial-scores`, `*-ttm`, `price-target-consensus`, and
//! `grades-summary` endpoints return a **current** value with no history, so a
//! one-shot sync produces a *current snapshot*, forward-filled from an as_of day:
//!
//! - **`piotroski_score`, `altman_z`, `fcf_yield`** — anchored to the latest
//!   annual report's **filing date** (from `income-statement`, reusing #131's
//!   [`FILING_DATE_KEYS`]); visible from when that report went public onward.
//!   (`fcf_yield` is a TTM figure approximated to the latest filing — documented.)
//! - **`analyst_upside_pct`, `consensus_rating`** — a live market/analyst view
//!   with no report date, so anchored to the **last synced trading day**
//!   (current-only; appears on the final bar).
//!
//! These panels are for **current-universe screening**, not deep historical
//! backtests — the web app accumulates daily history via cron; a one-shot CLI
//! run cannot. On `--resume`, panels reflect only the symbols processed this run.

use serde_json::Value;

use pomelo_data::{assemble, write_combined_panel, ObjectSink, PANELS_DIR};

use super::factors::{analyst_upside_pct, consensus_to_rating};
use super::fundamentals::{annual_url, FILING_DATE_KEYS};
use super::http::Fetcher;
use super::util::{iso_to_i32, num};
use super::HttpClient;
use super::FMP_BASE;

/// Series names this module writes, in `FACTOR_PANEL_FIELDS` order (minus the
/// cross-sectional `pe_industry_pctile`, which lands in Phase 2).
pub(crate) const SNAPSHOT_SERIES: &[&str] = &[
    "piotroski_score",
    "altman_z",
    "fcf_yield",
    "analyst_upside_pct",
    "consensus_rating",
];

/// A single-symbol endpoint URL (`?symbol=…`, no period) on the stable API.
fn snapshot_url(endpoint: &str, sym: &str, key: &str) -> String {
    format!("{FMP_BASE}/stable/{endpoint}?symbol={sym}&apikey={key}")
}

/// Fetch one endpoint and return its first row object (fail-soft: any transport
/// / status / shape error → `None`, so a missing endpoint just yields NaN).
fn first_row<H: HttpClient>(
    fetcher: &Fetcher<H>,
    url: &str,
) -> Option<serde_json::Map<String, Value>> {
    fetcher
        .get_rows(url)
        .ok()?
        .into_iter()
        .next()
        .and_then(|v| v.as_object().cloned())
}

/// Forward-fill a single snapshot `value` from `as_of` across `price_days`:
/// `(day, value)` for every day on/after `as_of`, empty when there is no value.
fn tail(price_days: &[i32], as_of: i32, value: Option<f64>) -> Vec<(i32, f64)> {
    match value {
        Some(v) => price_days
            .iter()
            .copied()
            .filter(|&d| d >= as_of)
            .map(|d| (d, v))
            .collect(),
        None => Vec::new(),
    }
}

/// Compute the five per-symbol snapshot-factor columns (forward-filled onto
/// `price_days`), in [`SNAPSHOT_SERIES`] order. `last_close` anchors
/// `analyst_upside_pct`. Every FMP fetch is fail-soft → a missing input leaves
/// that factor's column empty (NaN in the panel).
pub(crate) fn compute_symbol<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sym: &str,
    api_key: &str,
    price_days: &[i32],
    last_close: f64,
) -> [Vec<(i32, f64)>; SNAPSHOT_SERIES.len()] {
    let empty: [Vec<(i32, f64)>; 5] = Default::default();
    let Some(&last_day) = price_days.last() else {
        return empty;
    };

    // piotroski_score + altman_z — FMP's authoritative /financial-scores.
    let scores = first_row(fetcher, &snapshot_url("financial-scores", sym, api_key));
    let piotroski = scores.as_ref().and_then(|o| num(o, &["piotroskiScore"]));
    let altman = scores.as_ref().and_then(|o| num(o, &["altmanZScore"]));

    // as_of for the report-derived factors: the latest annual filing date
    // (newest row first), falling back to the last trading day.
    let report_asof = first_row(fetcher, &annual_url("income-statement", sym, api_key))
        .and_then(|o| {
            FILING_DATE_KEYS
                .iter()
                .find_map(|k| o.get(*k)?.as_str().and_then(iso_to_i32))
        })
        .unwrap_or(last_day);

    // fcf_yield — TTM free-cash-flow yield (key-metrics-ttm).
    let fcf = first_row(fetcher, &snapshot_url("key-metrics-ttm", sym, api_key))
        .and_then(|o| num(&o, &["freeCashFlowYieldTTM"]));

    // analyst_upside_pct — consensus target vs the last close.
    let upside = first_row(
        fetcher,
        &snapshot_url("price-target-consensus", sym, api_key),
    )
    .and_then(|o| num(&o, &["targetConsensus"]))
    .and_then(|target| analyst_upside_pct(target, last_close));

    // consensus_rating — grades-summary consensus label → 1..5.
    let rating = first_row(fetcher, &snapshot_url("grades-summary", sym, api_key)).and_then(|o| {
        o.get("consensus")
            .and_then(Value::as_str)
            .and_then(consensus_to_rating)
    });

    [
        tail(price_days, report_asof, piotroski),
        tail(price_days, report_asof, altman),
        tail(price_days, report_asof, fcf),
        tail(price_days, last_day, upside),
        tail(price_days, last_day, rating),
    ]
}

/// Accumulates per-symbol snapshot-factor columns across a sync run, then writes
/// one combined `panels/{name}.csv.gz` per factor.
pub(crate) struct SnapshotAccum {
    symbols: Vec<String>,
    /// `columns[factor][symbol]` — the forward-filled `(day, value)` rows.
    columns: Vec<Vec<Vec<(i32, f64)>>>,
}

impl SnapshotAccum {
    pub(crate) fn new() -> Self {
        SnapshotAccum {
            symbols: Vec::new(),
            columns: vec![Vec::new(); SNAPSHOT_SERIES.len()],
        }
    }

    /// Record one symbol's five factor columns (in [`SNAPSHOT_SERIES`] order).
    pub(crate) fn push(&mut self, sym: String, cols: [Vec<(i32, f64)>; SNAPSHOT_SERIES.len()]) {
        self.symbols.push(sym);
        for (factor, col) in cols.into_iter().enumerate() {
            self.columns[factor].push(col);
        }
    }

    /// Assemble and write each factor's combined panel to `store`. A factor with
    /// no data across every symbol is skipped (the loader treats an absent panel
    /// as all-NaN). Returns the number of panels written.
    pub(crate) fn write_panels(&self, store: &impl ObjectSink) -> Result<usize, String> {
        let mut written = 0;
        for (factor, name) in SNAPSHOT_SERIES.iter().enumerate() {
            let per_symbol = &self.columns[factor];
            if per_symbol.iter().all(|c| c.is_empty()) {
                eprintln!("{name}: no data across the universe, skipping panel");
                continue;
            }
            let panel = assemble(&self.symbols, per_symbol).map_err(|e| e.to_string())?;
            let bytes = write_combined_panel(&panel).map_err(|e| e.to_string())?;
            store
                .put(&format!("{PANELS_DIR}/{name}.csv.gz"), &bytes)
                .map_err(|e| e.to_string())?;
            written += 1;
            eprintln!(
                "wrote {PANELS_DIR}/{name}.csv.gz ({} symbols)",
                self.symbols.len()
            );
        }
        Ok(written)
    }
}
