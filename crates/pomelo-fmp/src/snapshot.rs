//! Snapshot-factor panels (#132): compute the `FACTOR_PANEL_FIELDS` combined
//! panels from FMP and write them as `panels/{name}.csv.gz`.
//!
//! Five factors are **per-symbol** — `piotroski_score`, `altman_z`, `fcf_yield`,
//! `analyst_upside_pct`, `consensus_rating` — computed independently as each
//! symbol is synced. The sixth, `pe_industry_pctile`, is **cross-sectional**:
//! it ranks a symbol's P/E within its industry cohort, so it can only be
//! resolved after the whole universe is fetched (see [`SnapshotAccum`]).
//!
//! ## Visibility (as_of) semantics
//!
//! FMP's `financial-scores`, `*-ttm`, `price-target-consensus`, and
//! `grades-summary` endpoints return a **current** value with no history, so a
//! one-shot sync produces a *current snapshot*, forward-filled from an as_of day:
//!
//! - **`piotroski_score`, `altman_z`, `fcf_yield`, `pe_industry_pctile`** —
//!   anchored to the latest annual report's **filing date** (from
//!   `income-statement`, reusing #131's [`FILING_DATE_KEYS`]); visible from when
//!   that report went public onward. (`fcf_yield` / the P/E are TTM figures
//!   approximated to the latest filing — documented.)
//! - **`analyst_upside_pct`, `consensus_rating`** — a live market/analyst view
//!   with no report date, so anchored to the **last synced trading day**
//!   (current-only; appears on the final bar).
//!
//! ## Cross-sectional cohort scope
//!
//! `pe_industry_pctile` groups symbols by the profile's `industry` field and
//! ranks each P/E within its cohort (mirroring the web builder). The web app
//! draws the cohort from its *entire* stored universe; a one-shot CLI run only
//! has **this run's symbols**, so the cohort is the intersection of the run
//! universe with each industry. Thin cohorts (< [`MIN_COHORT`](super::factors::MIN_COHORT)
//! finite, positive P/Es) are suppressed. Sync a broad universe for meaningful
//! percentiles.
//!
//! These panels are for **current-universe screening**, not deep historical
//! backtests — the web app accumulates daily history via cron; a one-shot CLI
//! run cannot. On `--resume`, panels reflect only the symbols processed this run.

use std::collections::HashMap;

use serde_json::Value;

use pomelo_data::{assemble, write_combined_panel, ObjectSink, PANELS_DIR};

use super::factors::{analyst_upside_pct, consensus_to_rating, pe_industry_pctile};
use super::fundamentals::{annual_url, FILING_DATE_KEYS};
use super::http::Fetcher;
use super::util::{iso_to_i32, num};
use super::HttpClient;
use super::FMP_BASE;

/// The five **per-symbol** snapshot series, in `FACTOR_PANEL_FIELDS` order. The
/// cross-sectional `pe_industry_pctile` is written separately by
/// [`SnapshotAccum::write_panels`] after the universe pass.
pub(crate) const DIRECT_SERIES: &[&str] = &[
    "piotroski_score",
    "altman_z",
    "fcf_yield",
    "analyst_upside_pct",
    "consensus_rating",
];

/// The cross-sectional series name (its own combined panel).
const PE_INDUSTRY_PCTILE: &str = "pe_industry_pctile";

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

/// A single symbol's snapshot-factor contribution: the five per-symbol columns
/// (forward-filled onto `price_days`, in [`DIRECT_SERIES`] order) plus the raw
/// inputs the cross-sectional `pe_industry_pctile` needs — the TTM P/E and the
/// report-filing as_of it is anchored to (its industry and price grid come from
/// the sync loop). A missing input leaves the corresponding column/field empty.
pub(crate) struct SymbolSnapshot {
    pub(crate) columns: [Vec<(i32, f64)>; DIRECT_SERIES.len()],
    /// TTM P/E for `pe_industry_pctile` (`None` when neither TTM endpoint has it).
    pub(crate) pe: Option<f64>,
    /// Filing-date anchor shared by the report-derived factors (incl. the P/E).
    pub(crate) report_asof: i32,
}

/// Compute one symbol's snapshot-factor contribution. `last_close` anchors
/// `analyst_upside_pct`. Every FMP fetch is fail-soft → a missing input leaves
/// that factor's column empty (NaN in the panel).
pub(crate) fn compute_symbol<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sym: &str,
    api_key: &str,
    price_days: &[i32],
    last_close: f64,
) -> SymbolSnapshot {
    let Some(&last_day) = price_days.last() else {
        return SymbolSnapshot {
            columns: Default::default(),
            pe: None,
            report_asof: 0,
        };
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

    // fcf_yield — TTM free-cash-flow yield (key-metrics-ttm). Keep the whole row
    // so the P/E can fall back to it when /ratios-ttm omits the field.
    let key_metrics = first_row(fetcher, &snapshot_url("key-metrics-ttm", sym, api_key));
    let fcf = key_metrics
        .as_ref()
        .and_then(|o| num(o, &["freeCashFlowYieldTTM"]));

    // pe (for pe_industry_pctile) — /ratios-ttm first, falling back to the
    // key-metrics row, mirroring the web's getFundamentals merge (ratios wins).
    let pe = first_row(fetcher, &snapshot_url("ratios-ttm", sym, api_key))
        .as_ref()
        .and_then(|o| num(o, PE_KEYS))
        .or_else(|| key_metrics.as_ref().and_then(|o| num(o, PE_KEYS)));

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

    SymbolSnapshot {
        columns: [
            tail(price_days, report_asof, piotroski),
            tail(price_days, report_asof, altman),
            tail(price_days, report_asof, fcf),
            tail(price_days, last_day, upside),
            tail(price_days, last_day, rating),
        ],
        pe,
        report_asof,
    }
}

/// TTM P/E aliases, in the web's read order (`priceToEarningsRatioTTM` first).
const PE_KEYS: &[&str] = &["priceToEarningsRatioTTM", "peRatioTTM"];

/// Per-symbol inputs for the cross-sectional `pe_industry_pctile`, held until
/// the whole universe is known: the P/E, its cohort key (industry), the
/// filing-date as_of, and the symbol's price grid to forward-fill onto.
struct PeInput {
    industry: Option<String>,
    pe: Option<f64>,
    as_of: i32,
    price_days: Vec<i32>,
}

/// Accumulates snapshot-factor contributions across a sync run, then writes one
/// combined `panels/{name}.csv.gz` per factor. The five per-symbol factors are
/// stored as ready columns; `pe_industry_pctile` is resolved from [`PeInput`]s
/// in a final cross-sectional pass (industry cohorts span the whole run).
pub(crate) struct SnapshotAccum {
    symbols: Vec<String>,
    /// `columns[factor][symbol]` — the forward-filled `(day, value)` rows, in
    /// [`DIRECT_SERIES`] order.
    columns: Vec<Vec<Vec<(i32, f64)>>>,
    /// `pe_inputs[symbol]` — parallel to `symbols`, for `pe_industry_pctile`.
    pe_inputs: Vec<PeInput>,
}

impl SnapshotAccum {
    pub(crate) fn new() -> Self {
        SnapshotAccum {
            symbols: Vec::new(),
            columns: vec![Vec::new(); DIRECT_SERIES.len()],
            pe_inputs: Vec::new(),
        }
    }

    /// Record one symbol's contribution: its five direct columns plus the P/E
    /// inputs (`industry` from the sync loop's profile fetch, `price_days` its
    /// trading grid) that the cross-sectional pass needs.
    pub(crate) fn push(
        &mut self,
        sym: String,
        snap: SymbolSnapshot,
        industry: Option<String>,
        price_days: &[i32],
    ) {
        self.symbols.push(sym);
        for (factor, col) in snap.columns.into_iter().enumerate() {
            self.columns[factor].push(col);
        }
        self.pe_inputs.push(PeInput {
            industry,
            pe: snap.pe,
            as_of: snap.report_asof,
            price_days: price_days.to_vec(),
        });
    }

    /// Assemble and write each factor's combined panel to `store` — the five
    /// direct factors plus the cross-sectional `pe_industry_pctile`. A factor
    /// with no data across every symbol is skipped (the loader treats an absent
    /// panel as all-NaN). Returns the number of panels written.
    pub(crate) fn write_panels(&self, store: &impl ObjectSink) -> Result<usize, String> {
        let mut written = 0;
        for (factor, name) in DIRECT_SERIES.iter().enumerate() {
            written += self.write_one(store, name, &self.columns[factor])?;
        }
        let pe_cols = pe_industry_pctile_columns(&self.pe_inputs);
        written += self.write_one(store, PE_INDUSTRY_PCTILE, &pe_cols)?;
        Ok(written)
    }

    /// Assemble one factor's `per_symbol` columns into a combined panel and put
    /// it under `panels/{name}.csv.gz`. Returns 1 if written, 0 if skipped
    /// (every symbol's column empty).
    fn write_one(
        &self,
        store: &impl ObjectSink,
        name: &str,
        per_symbol: &[Vec<(i32, f64)>],
    ) -> Result<usize, String> {
        if per_symbol.iter().all(|c| c.is_empty()) {
            eprintln!("{name}: no data across the universe, skipping panel");
            return Ok(0);
        }
        let panel = assemble(&self.symbols, per_symbol).map_err(|e| e.to_string())?;
        let bytes = write_combined_panel(&panel).map_err(|e| e.to_string())?;
        store
            .put(&format!("{PANELS_DIR}/{name}.csv.gz"), &bytes)
            .map_err(|e| e.to_string())?;
        eprintln!(
            "wrote {PANELS_DIR}/{name}.csv.gz ({} symbols)",
            self.symbols.len()
        );
        Ok(1)
    }
}

/// Cross-sectional pass for `pe_industry_pctile`: build each industry's P/E
/// cohort from the run's symbols (finite, strictly-positive P/Es only, mirroring
/// the web's `getPeCohortValues`), then rank every symbol's P/E within its
/// cohort and forward-fill the percentile from its as_of. A symbol with no
/// industry, an absent/non-positive P/E, or a thin cohort (< `MIN_COHORT`) gets
/// an empty column (NaN in the panel). Returns one column per input, in order.
fn pe_industry_pctile_columns(inputs: &[PeInput]) -> Vec<Vec<(i32, f64)>> {
    let mut cohorts: HashMap<&str, Vec<f64>> = HashMap::new();
    for pin in inputs {
        if let (Some(ind), Some(pe)) = (pin.industry.as_deref(), pin.pe) {
            if pe.is_finite() && pe > 0.0 {
                cohorts.entry(ind).or_default().push(pe);
            }
        }
    }
    inputs
        .iter()
        .map(|pin| match (pin.industry.as_deref(), pin.pe) {
            (Some(ind), Some(pe)) => {
                let cohort = cohorts.get(ind).map(Vec::as_slice).unwrap_or(&[]);
                match pe_industry_pctile(pe, cohort) {
                    Some(v) => tail(&pin.price_days, pin.as_of, Some(v)),
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(industry: Option<&str>, pe: Option<f64>, as_of: i32) -> PeInput {
        PeInput {
            industry: industry.map(str::to_string),
            pe,
            as_of,
            price_days: vec![20240101, 20240102],
        }
    }

    #[test]
    fn pe_industry_pctile_ranks_within_cohort_and_suppresses() {
        // Five "Software" P/Es form a 5-member cohort (== MIN_COHORT). A "Thin"
        // industry has one member; one symbol has no industry; one has pe <= 0
        // (excluded from its cohort *and* suppressed as a subject).
        let inputs = vec![
            pin(Some("Software"), Some(10.0), 20240102),
            pin(Some("Software"), Some(20.0), 20240102),
            pin(Some("Software"), Some(30.0), 20240102),
            pin(Some("Software"), Some(40.0), 20240102),
            pin(Some("Software"), Some(50.0), 20240102),
            pin(Some("Thin"), Some(15.0), 20240102),
            pin(None, Some(25.0), 20240102),
            pin(Some("Software"), Some(-5.0), 20240102),
        ];
        let cols = pe_industry_pctile_columns(&inputs);

        // pe=30 sits at midrank 2.5/5 = 0.5 → 50.0, forward-filled from as_of.
        assert_eq!(cols[2], vec![(20240102, 50.0)]);
        // Cohort extremes: 10 → 0.5/5*100 = 10, 50 → 4.5/5*100 = 90.
        assert_eq!(cols[0], vec![(20240102, 10.0)]);
        assert_eq!(cols[4], vec![(20240102, 90.0)]);
        // Thin cohort (1 member < MIN_COHORT), no industry, and non-positive
        // P/E all suppress to an empty column.
        assert!(cols[5].is_empty());
        assert!(cols[6].is_empty());
        assert!(cols[7].is_empty());
    }

    #[test]
    fn pe_industry_pctile_negative_peers_do_not_join_the_cohort() {
        // A negative P/E in the same industry must NOT count toward MIN_COHORT
        // (matching getPeCohortValues' `value > 0` filter): four positive peers
        // + one negative = a 4-member finite cohort → still suppressed.
        let inputs = vec![
            pin(Some("Auto"), Some(10.0), 20240102),
            pin(Some("Auto"), Some(20.0), 20240102),
            pin(Some("Auto"), Some(30.0), 20240102),
            pin(Some("Auto"), Some(40.0), 20240102),
            pin(Some("Auto"), Some(-8.0), 20240102),
        ];
        let cols = pe_industry_pctile_columns(&inputs);
        assert!(cols.iter().all(|c| c.is_empty()));
    }
}
