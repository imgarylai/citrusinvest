//! Best-effort snapshot-factor panels from EODHD fundamentals (#198).
//!
//! Writes current-as-of panels (not deep history):
//! - `analyst_upside_pct`, `consensus_rating` from `AnalystRatings`
//! - `fcf_yield` from latest annual freeCashFlow / market cap when present
//! - `pe_industry_pctile` cross-section within this run's industry cohorts
//!
//! Does **not** implement piotroski/altman DIY (leave those panels absent).

use std::collections::HashMap;

use pomelo_data::fundamentals::FACTOR_PANEL_FIELDS;
use pomelo_data::{assemble, write_combined_panel, ObjectSink, PANELS_DIR};
use serde_json::Value;

use super::factors::{analyst_upside_pct, eodhd_rating_to_consensus, pe_industry_pctile};
use super::http::Fetcher;
use super::util::{iso_to_i32, num};
use super::HttpClient;
use super::EODHD_BASE;

/// Per-symbol direct series (order matters for columns).
pub(crate) const DIRECT_SERIES: &[&str] = &["analyst_upside_pct", "consensus_rating", "fcf_yield"];

const PE_INDUSTRY_PCTILE: &str = "pe_industry_pctile";

fn snapshot_url(eodhd_code: &str, api_token: &str) -> String {
    format!(
        "{EODHD_BASE}/v1.1/fundamentals/{eodhd_code}?api_token={api_token}&fmt=json\
         &filter=AnalystRatings,Highlights::PERatio,Highlights::MarketCapitalization,\
         General::Industry,Financials::Cash_Flow::yearly"
    )
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
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Latest free cash flow from yearly CF map (period keys or nested).
fn latest_fcf(value: &Value) -> Option<f64> {
    let root = value.as_object()?;
    let yearly = root
        .get("Financials::Cash_Flow::yearly")
        .or_else(|| {
            root.get("Financials")
                .and_then(|f| f.get("Cash_Flow"))
                .and_then(|c| c.get("yearly"))
        })?
        .as_object()?;
    let mut best: Option<(i32, f64)> = None;
    for (k, v) in yearly {
        let obj = v.as_object()?;
        let day = obj
            .get("date")
            .and_then(Value::as_str)
            .and_then(iso_to_i32)
            .or_else(|| iso_to_i32(k))?;
        let fcf = num(obj, &["freeCashFlow", "free_cash_flow"])?;
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
    eodhd_code: &str,
    api_token: &str,
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

    let value = fetcher.get_json(&snapshot_url(eodhd_code, api_token)).ok();
    let root = value.as_ref().and_then(Value::as_object);

    let mut target = None;
    let mut rating = None;
    if let Some(ar) = root
        .and_then(|m| m.get("AnalystRatings"))
        .and_then(Value::as_object)
    {
        target = num(ar, &["TargetPrice", "targetPrice"]);
        rating = num(ar, &["Rating", "rating"]).and_then(eodhd_rating_to_consensus);
    }
    // Flat multi-filter form
    if target.is_none() {
        if let Some(m) = root {
            target = m.get("AnalystRatings::TargetPrice").and_then(as_f64);
            rating = m
                .get("AnalystRatings::Rating")
                .and_then(as_f64)
                .and_then(eodhd_rating_to_consensus)
                .or(rating);
        }
    }

    let upside = target.and_then(|t| analyst_upside_pct(t, last_close));

    let pe = root.and_then(|m| {
        m.get("Highlights::PERatio").and_then(as_f64).or_else(|| {
            m.get("Highlights")
                .and_then(Value::as_object)
                .and_then(|h| num(h, &["PERatio", "TrailingPE"]))
        })
    });

    let mcap = root.and_then(|m| {
        m.get("Highlights::MarketCapitalization")
            .and_then(as_f64)
            .or_else(|| {
                m.get("Highlights")
                    .and_then(Value::as_object)
                    .and_then(|h| num(h, &["MarketCapitalization"]))
            })
    });

    let industry = root.and_then(|m| {
        m.get("General::Industry")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                m.get("General")
                    .and_then(Value::as_object)
                    .and_then(|g| g.get("Industry"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
    });

    let fcf_yield = match (value.as_ref().and_then(latest_fcf), mcap) {
        (Some(fcf), Some(mc)) if mc > 0.0 && fcf.is_finite() => Some(fcf / mc),
        _ => None,
    };

    // Analyst / fcf are current-as-of last trading day.
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
        // Document that FACTOR_PANEL_FIELDS not written stay absent (NaN).
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
    use crate::http::HttpError;
    use std::cell::RefCell;
    use std::time::Duration;

    use super::super::config::SyncConfig;

    struct MockHttp {
        body: Vec<u8>,
    }
    impl HttpClient for MockHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.body.clone())
        }
    }

    #[test]
    fn compute_and_write_snapshot_panels() {
        let payload = br#"{
          "AnalystRatings": {"TargetPrice": 120.0, "Rating": 4.0},
          "Highlights::PERatio": 25.0,
          "Highlights::MarketCapitalization": 1000.0,
          "General::Industry": "Software",
          "Financials::Cash_Flow::yearly": {
            "2024-09-30": {"date":"2024-09-30","freeCashFlow":100.0}
          }
        }"#;
        let http = MockHttp {
            body: payload.to_vec(),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let days = [20240102, 20240103];
        let snap = compute_symbol(&fetcher, "AAPL.US", "tok", &days, 100.0);
        // upside 20%
        assert_eq!(snap.columns[0].last().map(|(_, v)| *v), Some(20.0));
        // rating 4 → consensus 2
        assert_eq!(snap.columns[1].last().map(|(_, v)| *v), Some(2.0));
        // fcf/mcap = 0.1
        assert!((snap.columns[2].last().unwrap().1 - 0.1).abs() < 1e-9);
        assert_eq!(snap.pe, Some(25.0));
        assert_eq!(snap.industry.as_deref(), Some("Software"));

        let mut acc = SnapshotAccum::new();
        // Need 5 symbols for pe cohort min
        for (i, pe) in [25.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
            let mut s = snap.clone();
            s.pe = Some(*pe);
            s.industry = Some("Software".into());
            acc.push(format!("S{i}"), s, &days);
        }
        let dir = std::env::temp_dir().join("pomelo_eodhd_snap");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let n = acc
            .write_panels(&pomelo_data::LocalSource::new(&dir))
            .unwrap();
        assert!(n >= 3);
        let _ = RefCell::new(0);
    }

    #[test]
    fn empty_price_days_and_missing_payload() {
        let http = MockHttp {
            body: b"{}".to_vec(),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let empty = compute_symbol(&fetcher, "X.US", "tok", &[], 100.0);
        assert!(empty.columns[0].is_empty());
        let sparse = compute_symbol(&fetcher, "X.US", "tok", &[20240102], 0.0);
        // close 0 → no upside
        assert!(sparse.columns[0].is_empty());
    }

    #[test]
    fn nested_highlights_and_skip_empty_panels() {
        let payload = br#"{
          "AnalystRatings": {"TargetPrice": "110", "Rating": "3.0"},
          "Highlights": {"PERatio": 15.0, "MarketCapitalization": 500.0},
          "General": {"Industry": "Banks"},
          "Financials": {"Cash_Flow": {"yearly": {
            "2023-12-31": {"date":"2023-12-31","freeCashFlow":"50"}
          }}}
        }"#;
        let http = MockHttp {
            body: payload.to_vec(),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let days = [20240102];
        let snap = compute_symbol(&fetcher, "B.US", "tok", &days, 100.0);
        assert_eq!(snap.columns[0].last().map(|(_, v)| *v), Some(10.0));
        assert_eq!(snap.pe, Some(15.0));
        assert!((snap.columns[2].last().unwrap().1 - 0.1).abs() < 1e-9);

        let acc = SnapshotAccum::new();
        let dir = std::env::temp_dir().join("pomelo_eodhd_snap_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // no symbols → skip all
        assert_eq!(
            acc.write_panels(&pomelo_data::LocalSource::new(&dir))
                .unwrap(),
            0
        );
    }

    #[test]
    fn pe_pctile_thin_cohort_skipped() {
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
}

// SymbolSnapshot needs Clone for test
impl Clone for SymbolSnapshot {
    fn clone(&self) -> Self {
        SymbolSnapshot {
            columns: [
                self.columns[0].clone(),
                self.columns[1].clone(),
                self.columns[2].clone(),
            ],
            pe: self.pe,
            industry: self.industry.clone(),
            as_of: self.as_of,
        }
    }
}
