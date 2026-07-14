//! Best-effort snapshot-factor panels from Alpha Vantage OVERVIEW (+ CF) (#218).
//!
//! Writes current-as-of panels (not deep history):
//! - `analyst_upside_pct` from `AnalystTargetPrice` vs last close
//! - `consensus_rating` from AnalystRating* counts (1…5, lower = more bullish)
//! - `fcf_yield` from latest annual free cash flow / market cap when present
//! - `pe_industry_pctile` cross-section within this run's industry cohorts
//!
//! Does **not** implement piotroski/altman DIY.

use std::collections::HashMap;

use pomelo_data::fundamentals::FACTOR_PANEL_FIELDS;
use pomelo_data::{assemble, write_combined_panel, ObjectSink, PANELS_DIR};
use serde_json::Value;

use super::factors::consensus_from_rating_counts;
use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::ALPHA_VANTAGE_BASE;
use pomelo_data::factors::{analyst_upside_pct, pe_industry_pctile};

/// Per-symbol direct series (order matters for columns).
pub(crate) const DIRECT_SERIES: &[&str] = &["analyst_upside_pct", "consensus_rating", "fcf_yield"];

const PE_INDUSTRY_PCTILE: &str = "pe_industry_pctile";

fn overview_url(av_symbol: &str, api_key: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=OVERVIEW&symbol={av_symbol}&apikey={api_key}")
}

fn cash_flow_url(av_symbol: &str, api_key: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=CASH_FLOW&symbol={av_symbol}&apikey={api_key}")
}

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

fn count_field(root: &serde_json::Map<String, Value>, key: &str) -> f64 {
    root.get(key).and_then(as_f64).unwrap_or(0.0)
}

/// Latest free cash flow from CASH_FLOW annualReports.
fn latest_fcf(value: &Value) -> Option<f64> {
    let root = value.as_object()?;
    for err_key in ["Error Message", "Information", "Note"] {
        if root.get(err_key).and_then(Value::as_str).is_some() {
            return None;
        }
    }
    let arr = root.get("annualReports")?.as_array()?;
    let mut best: Option<(i32, f64)> = None;
    for v in arr {
        let obj = v.as_object()?;
        let day = obj
            .get("fiscalDateEnding")
            .and_then(Value::as_str)
            .and_then(iso_to_i32)?;
        let fcf = obj
            .get("freeCashFlow")
            .and_then(as_f64)
            .or_else(|| obj.get("operatingCashflow").and_then(as_f64))
            .or_else(|| obj.get("operatingCashFlow").and_then(as_f64))?;
        if best.map(|(d, _)| day >= d).unwrap_or(true) {
            best = Some((day, fcf));
        }
    }
    best.map(|(_, v)| v)
}

pub(crate) struct SymbolSnapshot {
    pub(crate) columns: [Vec<(i32, f64)>; DIRECT_SERIES.len()],
    pub(crate) pe: Option<f64>,
    pub(crate) industry: Option<String>,
    pub(crate) as_of: i32,
}

pub(crate) fn compute_symbol<H: HttpClient>(
    fetcher: &Fetcher<H>,
    av_symbol: &str,
    api_key: &str,
    price_days: &[i32],
    last_close: f64,
) -> SymbolSnapshot {
    let Some(&last_day) = price_days.last() else {
        return SymbolSnapshot {
            columns: Default::default(),
            pe: None,
            industry: None,
            as_of: 0,
        };
    };

    let overview = fetcher.get_json(&overview_url(av_symbol, api_key)).ok();
    let root = overview.as_ref().and_then(Value::as_object);

    // Drop error envelopes
    if let Some(m) = root {
        for err_key in ["Error Message", "Information", "Note"] {
            if m.get(err_key).and_then(Value::as_str).is_some() {
                return SymbolSnapshot {
                    columns: Default::default(),
                    pe: None,
                    industry: None,
                    as_of: last_day,
                };
            }
        }
    }

    let target = root.and_then(|m| m.get("AnalystTargetPrice").and_then(as_f64));
    let upside = target.and_then(|t| analyst_upside_pct(t, last_close));

    let rating = root.and_then(|m| {
        consensus_from_rating_counts(
            count_field(m, "AnalystRatingStrongBuy"),
            count_field(m, "AnalystRatingBuy"),
            count_field(m, "AnalystRatingHold"),
            count_field(m, "AnalystRatingSell"),
            count_field(m, "AnalystRatingStrongSell"),
        )
    });

    let pe = root.and_then(|m| {
        m.get("PERatio")
            .and_then(as_f64)
            .or_else(|| m.get("TrailingPE").and_then(as_f64))
    });

    let mcap = root.and_then(|m| m.get("MarketCapitalization").and_then(as_f64));

    let industry = root
        .and_then(|m| m.get("Industry").and_then(str_clean))
        .or_else(|| root.and_then(|m| m.get("Sector").and_then(str_clean)));

    let fcf = fetcher
        .get_json(&cash_flow_url(av_symbol, api_key))
        .ok()
        .as_ref()
        .and_then(latest_fcf);
    let fcf_yield = match (fcf, mcap) {
        (Some(f), Some(mc)) if mc > 0.0 && f.is_finite() => Some(f / mc),
        _ => None,
    };

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
        let _ = FACTOR_PANEL_FIELDS;
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
    use std::cell::RefCell;
    use std::time::Duration;

    struct RouteHttp {
        routes: Vec<(String, Vec<u8>)>,
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

    #[test]
    fn compute_and_write_snapshot_panels() {
        let ov = br#"{
          "Symbol": "AAPL",
          "AnalystTargetPrice": "120.0",
          "AnalystRatingStrongBuy": "2",
          "AnalystRatingBuy": "3",
          "AnalystRatingHold": "0",
          "AnalystRatingSell": "0",
          "AnalystRatingStrongSell": "0",
          "PERatio": "25.0",
          "MarketCapitalization": "1000",
          "Industry": "Software"
        }"#;
        let cf = br#"{
          "symbol": "AAPL",
          "annualReports": [
            {"fiscalDateEnding": "2024-09-30", "freeCashFlow": "100.0"}
          ]
        }"#;
        let http = RouteHttp {
            routes: vec![
                ("OVERVIEW".into(), ov.to_vec()),
                ("CASH_FLOW".into(), cf.to_vec()),
            ],
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let days = [20240102, 20240103];
        let snap = compute_symbol(&fetcher, "AAPL", "tok", &days, 100.0);
        assert_eq!(snap.columns[0].last().map(|(_, v)| *v), Some(20.0));
        // strong buy 2 + buy 3 → (1*2+2*3)/5 = 1.6
        assert!((snap.columns[1].last().unwrap().1 - 1.6).abs() < 1e-9);
        assert!((snap.columns[2].last().unwrap().1 - 0.1).abs() < 1e-9);
        assert_eq!(snap.pe, Some(25.0));

        let mut acc = SnapshotAccum::new();
        for (i, pe) in [25.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
            let s = SymbolSnapshot {
                columns: [
                    snap.columns[0].clone(),
                    snap.columns[1].clone(),
                    snap.columns[2].clone(),
                ],
                pe: Some(*pe),
                industry: Some("Software".into()),
                as_of: snap.as_of,
            };
            acc.push(format!("S{i}"), s, &days);
        }
        let dir = std::env::temp_dir().join("pomelo_av_snap");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let n = acc
            .write_panels(&pomelo_data::LocalSource::new(&dir))
            .unwrap();
        assert!(n >= 3);
        let _ = RefCell::new(0);
    }

    #[test]
    fn empty_days_and_error_overview() {
        let http = RouteHttp {
            routes: vec![("OVERVIEW".into(), br#"{"Note":"x"}"#.to_vec())],
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let empty = compute_symbol(&fetcher, "X", "tok", &[], 100.0);
        assert!(empty.columns[0].is_empty());
        let err = compute_symbol(&fetcher, "X", "tok", &[20240102], 100.0);
        assert!(err.columns.iter().all(|c| c.is_empty()));
    }

    #[test]
    fn write_panels_skips_when_empty_accum() {
        let acc = SnapshotAccum::new();
        let dir = std::env::temp_dir().join("pomelo_av_snap_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            acc.write_panels(&pomelo_data::LocalSource::new(&dir))
                .unwrap(),
            0
        );
    }

    #[test]
    fn pe_thin_cohort_empty() {
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
        let cols = pe_industry_pctile_columns(&acc.pe_inputs);
        assert!(cols.iter().all(|c| c.is_empty()));
    }

    #[test]
    fn latest_fcf_prefers_free_cash_flow() {
        let v = serde_json::json!({
            "annualReports": [
                {"fiscalDateEnding": "2023-12-31", "freeCashFlow": "10"},
                {"fiscalDateEnding": "2024-12-31", "freeCashFlow": "20"}
            ]
        });
        assert_eq!(latest_fcf(&v), Some(20.0));
    }

    #[test]
    fn latest_fcf_none_on_note() {
        let v = serde_json::json!({"Note": "x"});
        assert!(latest_fcf(&v).is_none());
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

    #[test]
    fn compute_close_zero_no_upside() {
        let ov = br#"{"Symbol":"X","AnalystTargetPrice":"120","PERatio":"10","Industry":"A"}"#;
        let http = RouteHttp {
            routes: vec![
                ("OVERVIEW".into(), ov.to_vec()),
                ("CASH_FLOW".into(), br#"{"annualReports":[]}"#.to_vec()),
            ],
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let snap = compute_symbol(&fetcher, "X", "tok", &[20240102], 0.0);
        assert!(snap.columns[0].is_empty());
        assert_eq!(snap.pe, Some(10.0));
    }
}
