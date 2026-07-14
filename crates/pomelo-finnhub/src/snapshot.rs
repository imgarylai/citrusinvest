//! Best-effort snapshot-factor panels from Finnhub (#230).
//!
//! Current-as-of panels (not deep history), each a `dates × symbols` panel:
//! - `analyst_upside_pct` from `/stock/price-target` `targetMean` vs last close
//! - `consensus_rating` from the latest `/stock/recommendation` count buckets
//!   (1…5, lower = more bullish)
//! - `fcf_yield` from `/stock/metric` `freeCashFlowTTM / marketCapitalization`
//! - `pe_industry_pctile` cross-section within this run's industry cohorts
//!   (P/E from `/stock/metric`, industry from `/stock/profile2`)
//!
//! Best-effort / DIY (spike #208): price targets and some metrics are plan-gated,
//! so a series is simply **absent** when its endpoint returns nothing — no
//! fabricated values, and no piotroski/altman DIY. Panels are stamped on the
//! last synced price day forward.

use std::collections::HashMap;

use pomelo_data::{assemble, write_combined_panel, ObjectSink, PANELS_DIR};
use serde_json::Value;

use super::factors::consensus_from_rating_counts;
use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::FINNHUB_BASE;
use pomelo_data::factors::{analyst_upside_pct, pe_industry_pctile};

/// Per-symbol direct series (order matters for columns).
pub(crate) const DIRECT_SERIES: &[&str] = &["analyst_upside_pct", "consensus_rating", "fcf_yield"];

const PE_INDUSTRY_PCTILE: &str = "pe_industry_pctile";

fn recommendation_url(sym: &str, key: &str) -> String {
    format!("{FINNHUB_BASE}/stock/recommendation?symbol={sym}&token={key}")
}
fn price_target_url(sym: &str, key: &str) -> String {
    format!("{FINNHUB_BASE}/stock/price-target?symbol={sym}&token={key}")
}
fn metric_url(sym: &str, key: &str) -> String {
    format!("{FINNHUB_BASE}/stock/metric?symbol={sym}&metric=all&token={key}")
}
fn profile_url(sym: &str, key: &str) -> String {
    format!("{FINNHUB_BASE}/stock/profile2?symbol={sym}&token={key}")
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() || s.eq_ignore_ascii_case("None") || s == "-" {
                None
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

fn str_clean(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "None" && *s != "-")
        .map(str::to_string)
}

/// Stamp `value` on every price day at/after `as_of` (empty when `None`).
fn tail(price_days: &[i32], as_of: i32, value: Option<f64>) -> Vec<(i32, f64)> {
    match value {
        Some(v) if v.is_finite() => price_days
            .iter()
            .copied()
            .filter(|&d| d >= as_of)
            .map(|d| (d, v))
            .collect(),
        _ => Vec::new(),
    }
}

/// Consensus rating from the latest `/stock/recommendation` row (max `period`).
pub(crate) fn latest_consensus(value: &Value) -> Option<f64> {
    let arr = value.as_array()?;
    let latest = arr.iter().filter_map(Value::as_object).max_by_key(|o| {
        o.get("period")
            .and_then(Value::as_str)
            .and_then(iso_to_i32)
            .unwrap_or(0)
    })?;
    let c = |k: &str| latest.get(k).and_then(as_f64).unwrap_or(0.0);
    consensus_from_rating_counts(
        c("strongBuy"),
        c("buy"),
        c("hold"),
        c("sell"),
        c("strongSell"),
    )
}

/// Read the `metric` object from a `/stock/metric` payload.
fn metric_obj(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object()?.get("metric")?.as_object()
}

pub(crate) struct SymbolSnapshot {
    pub(crate) columns: [Vec<(i32, f64)>; DIRECT_SERIES.len()],
    pub(crate) pe: Option<f64>,
    pub(crate) industry: Option<String>,
    pub(crate) as_of: i32,
}

impl SymbolSnapshot {
    fn empty(as_of: i32) -> Self {
        SymbolSnapshot {
            columns: Default::default(),
            pe: None,
            industry: None,
            as_of,
        }
    }
}

pub(crate) fn compute_symbol<H: HttpClient>(
    fetcher: &Fetcher<H>,
    fh_symbol: &str,
    api_key: &str,
    price_days: &[i32],
    last_close: f64,
) -> SymbolSnapshot {
    let Some(&last_day) = price_days.last() else {
        return SymbolSnapshot::empty(0);
    };

    // consensus_rating (recommendation trends)
    let rating = fetcher
        .get_json(&recommendation_url(fh_symbol, api_key))
        .ok()
        .as_ref()
        .and_then(latest_consensus);

    // analyst_upside_pct (price target — often plan-gated)
    let upside = fetcher
        .get_json(&price_target_url(fh_symbol, api_key))
        .ok()
        .as_ref()
        .and_then(|v| {
            v.as_object()
                .and_then(|o| o.get("targetMean").and_then(as_f64))
        })
        .and_then(|t| analyst_upside_pct(t, last_close));

    // metric: pe + fcf_yield
    let metric = fetcher.get_json(&metric_url(fh_symbol, api_key)).ok();
    let m = metric.as_ref().and_then(metric_obj);
    let pe = m.and_then(|m| {
        m.get("peTTM")
            .and_then(as_f64)
            .or_else(|| m.get("peBasicExclExtraTTM").and_then(as_f64))
            .or_else(|| m.get("peExclExtraTTM").and_then(as_f64))
    });
    let mcap = m.and_then(|m| m.get("marketCapitalization").and_then(as_f64));
    let fcf = m.and_then(|m| m.get("freeCashFlowTTM").and_then(as_f64));
    let fcf_yield = match (fcf, mcap) {
        (Some(f), Some(mc)) if mc > 0.0 && f.is_finite() => Some(f / mc),
        _ => None,
    };

    // industry for the pe cohort (profile2)
    let industry = fetcher
        .get_json(&profile_url(fh_symbol, api_key))
        .ok()
        .as_ref()
        .and_then(|v| {
            v.as_object()
                .and_then(|o| o.get("finnhubIndustry").and_then(str_clean))
        });

    SymbolSnapshot {
        columns: [
            tail(price_days, last_day, upside),
            tail(price_days, last_day, rating),
            tail(price_days, last_day, fcf_yield),
        ],
        pe,
        industry,
        as_of: last_day,
    }
}

struct PeInput {
    industry: Option<String>,
    pe: Option<f64>,
    as_of: i32,
    price_days: Vec<i32>,
}

pub(crate) struct SnapshotAccum {
    symbols: Vec<String>,
    columns: Vec<Vec<Vec<(i32, f64)>>>,
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

    pub(crate) fn push(&mut self, sym: String, snap: SymbolSnapshot, price_days: &[i32]) {
        self.symbols.push(sym);
        for (i, col) in snap.columns.into_iter().enumerate() {
            self.columns[i].push(col);
        }
        self.pe_inputs.push(PeInput {
            industry: snap.industry,
            pe: snap.pe,
            as_of: snap.as_of,
            price_days: price_days.to_vec(),
        });
    }

    pub(crate) fn write_panels(&self, store: &impl ObjectSink) -> Result<usize, String> {
        let mut written = 0;
        for (i, name) in DIRECT_SERIES.iter().enumerate() {
            written += self.write_one(store, name, &self.columns[i])?;
        }
        let pe_cols = pe_industry_pctile_columns(&self.pe_inputs);
        written += self.write_one(store, PE_INDUSTRY_PCTILE, &pe_cols)?;
        Ok(written)
    }

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
    use crate::config::SyncConfig;
    use crate::http::{HttpClient, HttpError};
    use std::time::Duration;

    struct RouteHttp {
        routes: Vec<(&'static str, Vec<u8>)>,
    }
    impl HttpClient for RouteHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            for (pat, body) in &self.routes {
                if url.contains(pat) {
                    return Ok(body.clone());
                }
            }
            Err(HttpError::Status(404))
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    const REC: &str = r#"[
        {"period":"2024-02-01","strongBuy":1,"buy":1,"hold":1,"sell":1,"strongSell":1},
        {"period":"2024-03-01","strongBuy":2,"buy":3,"hold":0,"sell":0,"strongSell":0}
    ]"#;
    const PT: &str = r#"{"symbol":"AAPL","targetMean":120.0,"targetHigh":140,"targetLow":100}"#;
    const METRIC: &str = r#"{"metricType":"all","symbol":"AAPL","metric":{
        "peTTM":25.0,"marketCapitalization":1000.0,"freeCashFlowTTM":100.0}}"#;
    const PROFILE: &str = r#"{"ticker":"AAPL","finnhubIndustry":"Software"}"#;

    fn full_routes() -> RouteHttp {
        RouteHttp {
            routes: vec![
                ("stock/recommendation", REC.as_bytes().to_vec()),
                ("stock/price-target", PT.as_bytes().to_vec()),
                ("stock/metric", METRIC.as_bytes().to_vec()),
                ("stock/profile2", PROFILE.as_bytes().to_vec()),
            ],
        }
    }

    #[test]
    fn latest_consensus_picks_newest_period() {
        let v: Value = serde_json::from_str(REC).unwrap();
        // 2024-03: strongBuy 2 + buy 3 → (1*2+2*3)/5 = 1.6
        assert!((latest_consensus(&v).unwrap() - 1.6).abs() < 1e-9);
        assert!(latest_consensus(&serde_json::json!([])).is_none());
    }

    #[test]
    fn compute_symbol_all_series() {
        let http = full_routes();
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let days = [20240102, 20240103];
        let snap = compute_symbol(&fetcher, "AAPL", "tok", &days, 100.0);
        // upside (120-100)/100*100 = 20
        assert_eq!(snap.columns[0].last().map(|(_, v)| *v), Some(20.0));
        // consensus 1.6
        assert!((snap.columns[1].last().unwrap().1 - 1.6).abs() < 1e-9);
        // fcf_yield 100/1000 = 0.1
        assert!((snap.columns[2].last().unwrap().1 - 0.1).abs() < 1e-9);
        assert_eq!(snap.pe, Some(25.0));
        assert_eq!(snap.industry.as_deref(), Some("Software"));
    }

    #[test]
    fn compute_symbol_empty_and_missing_endpoints() {
        // No price days → empty as_of 0.
        let http = full_routes();
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let empty = compute_symbol(&fetcher, "X", "tok", &[], 100.0);
        assert!(empty.columns.iter().all(|c| c.is_empty()));

        // All endpoints 404 → all series absent, but as_of set.
        let none = RouteHttp { routes: vec![] };
        let fetcher = Fetcher::new(&none, &cfg);
        let snap = compute_symbol(&fetcher, "X", "tok", &[20240102], 100.0);
        assert!(snap.columns.iter().all(|c| c.is_empty()));
        assert!(snap.pe.is_none());
        assert_eq!(snap.as_of, 20240102);
    }

    #[test]
    fn compute_symbol_zero_close_no_upside() {
        let http = full_routes();
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let snap = compute_symbol(&fetcher, "X", "tok", &[20240102], 0.0);
        assert!(snap.columns[0].is_empty()); // upside needs close > 0
        assert_eq!(snap.pe, Some(25.0));
    }

    #[test]
    fn write_and_pe_cohort_panels() {
        let http = full_routes();
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let days = [20240102, 20240103];
        let base = compute_symbol(&fetcher, "AAPL", "tok", &days, 100.0);

        let mut acc = SnapshotAccum::new();
        for (i, pe) in [25.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
            let s = SymbolSnapshot {
                columns: [
                    base.columns[0].clone(),
                    base.columns[1].clone(),
                    base.columns[2].clone(),
                ],
                pe: Some(*pe),
                industry: Some("Software".into()),
                as_of: base.as_of,
            };
            acc.push(format!("S{i}"), s, &days);
        }
        let dir = std::env::temp_dir().join("pomelo_fh_snap");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let n = acc
            .write_panels(&pomelo_data::LocalSource::new(&dir))
            .unwrap();
        assert!(n >= 3);
        assert!(dir.join("panels/pe_industry_pctile.csv.gz").exists());
    }

    #[test]
    fn write_panels_empty_accum_is_noop() {
        let acc = SnapshotAccum::new();
        let dir = std::env::temp_dir().join("pomelo_fh_snap_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            acc.write_panels(&pomelo_data::LocalSource::new(&dir))
                .unwrap(),
            0
        );
    }

    #[test]
    fn pe_cohort_too_thin_is_empty() {
        let mut acc = SnapshotAccum::new();
        let days = [20240102];
        for i in 0..3 {
            acc.push(
                format!("T{i}"),
                SymbolSnapshot {
                    columns: Default::default(),
                    pe: Some(10.0 + i as f64),
                    industry: Some("Thin".into()),
                    as_of: 20240102,
                },
                &days,
            );
        }
        assert!(pe_industry_pctile_columns(&acc.pe_inputs)
            .iter()
            .all(|c| c.is_empty()));
    }

    #[test]
    fn pe_pctile_without_industry_empty() {
        let cols = pe_industry_pctile_columns(&[PeInput {
            industry: None,
            pe: Some(10.0),
            as_of: 20240102,
            price_days: vec![20240102],
        }]);
        assert!(cols[0].is_empty());
    }
}
