//! Annual fundamentals from Finnhub `/stock/financials-reported` → dense
//! `fundamentals/{SYM}.csv.gz`.
//!
//! Finnhub's as-reported financials carry a **`filedDate`** (the SEC filing
//! date), so — unlike Alpha Vantage (#216), which only has fiscal period-end —
//! each snapshot becomes visible on the day it was actually filed. That gives an
//! honest point-in-time `report_event` (spike #208: "better PIT story than AV").
//! When a period lacks `filedDate` we fall back to the fiscal `endDate` and mark
//! the snapshot `fell_back` so the log can say so.
//!
//! The `report` payload uses US-GAAP concept tags (`us-gaap_Revenues`, …) in
//! parallel `ic` / `bs` / `cf` arrays. We match line items by the concept tail
//! (the part after the namespace prefix), trying a small candidate list per
//! field. Price multiples (`pe`/`ps`/`pb`) and `market_cap` are left NaN — no
//! historical shares/price join here (DIY / snapshot phase).

use std::collections::BTreeMap;
use std::collections::HashMap;

use pomelo_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use pomelo_data::ObjectSink;
use serde_json::Value;

use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::FINNHUB_BASE;

/// One fiscal-period snapshot aligned to [`FUNDAMENTAL_FIELDS`].
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    /// Day the data becomes visible: filing date, or period-end on fallback.
    pub visible: i32,
    pub values: Vec<f64>,
    /// True when no `filedDate` was available and we used the fiscal period-end.
    pub fell_back: bool,
}

pub(crate) fn financials_url(fh_symbol: &str, api_key: &str) -> String {
    format!(
        "{FINNHUB_BASE}/stock/financials-reported?symbol={fh_symbol}&freq=annual&token={api_key}"
    )
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

/// The concept tail: the part after the namespace prefix (`us-gaap_Assets` →
/// `assets`), lowercased for case-insensitive matching.
fn concept_tail(concept: &str) -> String {
    let tail = concept.rsplit(['_', ':']).next().unwrap_or(concept);
    tail.to_ascii_lowercase()
}

/// Collapse a `report` statement array (`ic`/`bs`/`cf`) into `tail → value`,
/// first occurrence winning.
fn report_map(arr: &[Value]) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(concept) = obj.get("concept").and_then(Value::as_str) else {
            continue;
        };
        let Some(val) = obj.get("value").and_then(as_f64) else {
            continue;
        };
        m.entry(concept_tail(concept)).or_insert(val);
    }
    m
}

/// First matching concept-tail value from a statement map.
fn pick(map: &HashMap<String, f64>, candidates: &[&str]) -> Option<f64> {
    candidates.iter().find_map(|c| map.get(*c).copied())
}

fn safe_div(num: Option<f64>, den: Option<f64>) -> f64 {
    match (num, den) {
        (Some(n), Some(d)) if d != 0.0 && n.is_finite() && d.is_finite() => n / d,
        _ => f64::NAN,
    }
}

fn yoy(curr: Option<f64>, prev: Option<f64>) -> f64 {
    match (curr, prev) {
        (Some(c), Some(p)) if p != 0.0 && c.is_finite() && p.is_finite() => c / p - 1.0,
        _ => f64::NAN,
    }
}

// Concept-tail candidates (lowercased), most-specific first.
const REVENUE: &[&str] = &[
    "revenuefromcontractwithcustomerexcludingassessedtax",
    "revenuefromcontractwithcustomerincludingassessedtax",
    "revenues",
    "salesrevenuenet",
    "salesrevenuegoodsnet",
];
const GROSS_PROFIT: &[&str] = &["grossprofit"];
const NET_INCOME: &[&str] = &[
    "netincomeloss",
    "profitloss",
    "netincomelossavailabletocommonstockholdersbasic",
];
const OPERATING_INCOME: &[&str] = &["operatingincomeloss"];
const ASSETS: &[&str] = &["assets"];
const LIABILITIES: &[&str] = &["liabilities"];
const EQUITY: &[&str] = &[
    "stockholdersequity",
    "stockholdersequityincludingportionattributabletononcontrollinginterest",
];
const DEBT: &[&str] = &[
    "longtermdebtnoncurrent",
    "longtermdebt",
    "longtermdebtandcapitalleaseobligations",
    "longtermdebtcurrent",
    "debtcurrent",
];
const RECEIVABLES: &[&str] = &[
    "accountsreceivablenetcurrent",
    "receivablesnetcurrent",
    "accountsreceivablenet",
];

/// A single as-reported annual filing, reduced to what we need.
#[derive(Debug)]
pub(crate) struct Filing {
    end: i32,
    filed: Option<i32>,
    ic: HashMap<String, f64>,
    bs: HashMap<String, f64>,
}

/// Extract annual filings from a `financials-reported` payload, keyed & sorted
/// by fiscal period-end (later filings for the same period win).
pub(crate) fn extract_filings(value: &Value) -> Result<Vec<Filing>, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "financials-reported payload is not a JSON object".to_string())?;
    if let Some(err) = root.get("error").and_then(Value::as_str) {
        return Err(format!("Finnhub error: {err}"));
    }
    let Some(data) = root.get("data").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let stmt = |report: &serde_json::Map<String, Value>, key: &str| -> HashMap<String, f64> {
        report
            .get(key)
            .and_then(Value::as_array)
            .map(|a| report_map(a))
            .unwrap_or_default()
    };

    // Keep the latest filing per fiscal period-end.
    let mut by_period: BTreeMap<i32, Filing> = BTreeMap::new();
    for entry in data {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(end) = obj
            .get("endDate")
            .and_then(Value::as_str)
            .and_then(iso_to_i32)
        else {
            continue;
        };
        let filed = obj
            .get("filedDate")
            .and_then(Value::as_str)
            .and_then(iso_to_i32);
        let report = obj.get("report").and_then(Value::as_object);
        let (ic, bs) = match report {
            Some(r) => (stmt(r, "ic"), stmt(r, "bs")),
            None => (HashMap::new(), HashMap::new()),
        };
        by_period.insert(end, Filing { end, filed, ic, bs });
    }
    Ok(by_period.into_values().collect())
}

/// Build sorted snapshots from annual filings (filing-date visibility).
pub(crate) fn build_snapshots(filings: &[Filing]) -> Vec<Snapshot> {
    let mut snaps = Vec::new();
    let mut prev_ic: Option<&HashMap<String, f64>> = None;

    for f in filings {
        let rev = pick(&f.ic, REVENUE);
        let gp = pick(&f.ic, GROSS_PROFIT);
        let ni = pick(&f.ic, NET_INCOME);
        let oi = pick(&f.ic, OPERATING_INCOME);

        let equity = pick(&f.bs, EQUITY);
        let assets = pick(&f.bs, ASSETS);
        let liab = pick(&f.bs, LIABILITIES);
        let debt = pick(&f.bs, DEBT);
        let recv = pick(&f.bs, RECEIVABLES);

        let prev_rev = prev_ic.and_then(|p| pick(p, REVENUE));
        let prev_ni = prev_ic.and_then(|p| pick(p, NET_INCOME));
        let prev_oi = prev_ic.and_then(|p| pick(p, OPERATING_INCOME));
        let prev_gp = prev_ic.and_then(|p| pick(p, GROSS_PROFIT));

        let values = vec![
            f64::NAN,                        // pe
            f64::NAN,                        // ps
            f64::NAN,                        // pb
            safe_div(ni, equity),            // roe
            safe_div(ni, rev),               // net_margin
            safe_div(liab.or(debt), equity), // debt_to_equity
            f64::NAN,                        // market_cap
            safe_div(gp, rev),               // gross_margin
            safe_div(rev, recv),             // receivables_turnover
            safe_div(debt.or(liab), assets), // debt_to_assets
            rev.unwrap_or(f64::NAN),         // revenue
            yoy(rev, prev_rev),              // revenue_growth
            yoy(ni, prev_ni),                // eps_growth (NI YoY proxy)
            yoy(oi, prev_oi),                // operating_income_growth
            yoy(ni, prev_ni),                // net_income_growth
            yoy(gp, prev_gp),                // gross_profit_growth
        ];
        debug_assert_eq!(values.len(), FUNDAMENTAL_FIELDS.len());

        snaps.push(Snapshot {
            visible: f.filed.unwrap_or(f.end),
            values,
            fell_back: f.filed.is_none(),
        });
        prev_ic = Some(&f.ic);
    }

    snaps.sort_by_key(|s| s.visible);
    snaps
}

/// Forward-fill snapshots onto `price_days`, flagging `report_event` on the day
/// each filing first becomes visible.
pub(crate) fn densify_fundamentals(
    snapshots: &[Snapshot],
    price_days: &[i32],
) -> Vec<FundamentalRow> {
    let nfields = FUNDAMENTAL_FIELDS.len();
    let mut rows = Vec::with_capacity(price_days.len());
    let mut si = 0usize;
    let mut current = vec![f64::NAN; nfields];
    for &day in price_days {
        let mut event = 0.0;
        while si < snapshots.len() && snapshots[si].visible <= day {
            current = snapshots[si].values.clone();
            event = 1.0;
            si += 1;
        }
        rows.push(FundamentalRow {
            day,
            values: current.clone(),
            report_event: event,
        });
    }
    rows
}

/// Fetch annual as-reported financials, densify onto `price_days`, write the
/// fundamentals file. Returns `false` when no annual filings are available.
pub(crate) fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &impl ObjectSink,
    layout_sym: &str,
    fh_symbol: &str,
    api_key: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{layout_sym}: fetching fundamentals ({fh_symbol})…");
    let val = fetcher.get_json(&financials_url(fh_symbol, api_key))?;
    let filings = extract_filings(&val)?;
    let snapshots = build_snapshots(&filings);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let fell_back = snapshots.iter().filter(|s| s.fell_back).count();
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{layout_sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{layout_sym}: wrote {} fundamental rows ({} annual filings; {fell_back} lacked \
         filedDate → fiscal period-end visibility)",
        rows.len(),
        snapshots.len(),
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncConfig;
    use crate::http::{HttpClient, HttpError};
    use pomelo_data::LocalSource;
    use serde_json::json;
    use std::time::Duration;

    fn line(concept: &str, value: f64) -> Value {
        json!({"concept": concept, "label": concept, "value": value, "unit": "usd"})
    }

    fn filing(end: &str, filed: Option<&str>, rev: f64, ni: f64, gp: f64, oi: f64) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("year".into(), json!(2023));
        obj.insert("form".into(), json!("10-K"));
        obj.insert("endDate".into(), json!(format!("{end} 00:00:00")));
        if let Some(f) = filed {
            obj.insert("filedDate".into(), json!(format!("{f} 00:00:00")));
        }
        obj.insert(
            "report".into(),
            json!({
                "ic": [
                    line("us-gaap_Revenues", rev),
                    line("us-gaap_NetIncomeLoss", ni),
                    line("us-gaap_GrossProfit", gp),
                    line("us-gaap_OperatingIncomeLoss", oi),
                ],
                "bs": [
                    line("us-gaap_Assets", 200.0),
                    line("us-gaap_Liabilities", 80.0),
                    line("us-gaap_StockholdersEquity", 120.0),
                    line("us-gaap_AccountsReceivableNetCurrent", 10.0),
                    line("us-gaap_LongTermDebtNoncurrent", 50.0),
                ],
            }),
        );
        Value::Object(obj)
    }

    fn sample(filed_a: Option<&str>) -> Value {
        json!({
            "cik": "1",
            "data": [
                filing("2023-12-31", filed_a, 100.0, 20.0, 40.0, 30.0),
                filing("2024-12-31", Some("2025-02-01"), 110.0, 22.0, 44.0, 33.0),
            ]
        })
    }

    #[test]
    fn concept_tail_strips_namespace() {
        assert_eq!(concept_tail("us-gaap_Revenues"), "revenues");
        assert_eq!(concept_tail("us-gaap:Assets"), "assets");
        assert_eq!(concept_tail("NetIncomeLoss"), "netincomeloss");
    }

    #[test]
    fn assets_tail_does_not_match_current_assets() {
        let m = report_map(&[
            line("us-gaap_AssetsCurrent", 5.0),
            line("us-gaap_Assets", 200.0),
        ]);
        assert_eq!(pick(&m, ASSETS), Some(200.0));
    }

    #[test]
    fn uses_filed_date_for_visibility() {
        let filings = extract_filings(&sample(Some("2024-02-15"))).unwrap();
        assert_eq!(filings.len(), 2);
        let snaps = build_snapshots(&filings);
        assert_eq!(snaps.len(), 2);
        // First filing visible on its filedDate, not its 2023-12-31 period-end.
        assert_eq!(snaps[0].visible, 20240215);
        assert!(!snaps[0].fell_back);
        // revenue = 100
        assert!((snaps[0].values[10] - 100.0).abs() < 1e-9);
        // net_margin 20/100
        assert!((snaps[0].values[4] - 0.2).abs() < 1e-9);
        // roe 20/120
        assert!((snaps[0].values[3] - 20.0 / 120.0).abs() < 1e-9);
        // revenue growth 10% on the second filing
        assert!((snaps[1].values[11] - 0.1).abs() < 1e-9);
        assert!(snaps[0].values[0].is_nan()); // pe
    }

    #[test]
    fn falls_back_to_period_end_without_filed_date() {
        let filings = extract_filings(&sample(None)).unwrap();
        let snaps = build_snapshots(&filings);
        assert_eq!(snaps[0].visible, 20231231);
        assert!(snaps[0].fell_back);
    }

    #[test]
    fn densify_marks_events_on_visibility() {
        let snaps = vec![
            Snapshot {
                visible: 20240102,
                values: vec![1.0; FUNDAMENTAL_FIELDS.len()],
                fell_back: false,
            },
            Snapshot {
                visible: 20240104,
                values: vec![2.0; FUNDAMENTAL_FIELDS.len()],
                fell_back: false,
            },
        ];
        let days = [20240101, 20240102, 20240103, 20240104];
        let rows = densify_fundamentals(&snaps, &days);
        assert!(rows[0].values[0].is_nan());
        assert_eq!(rows[0].report_event, 0.0);
        assert_eq!(rows[1].values[0], 1.0);
        assert_eq!(rows[1].report_event, 1.0);
        assert_eq!(rows[2].report_event, 0.0);
        assert_eq!(rows[3].values[0], 2.0);
        assert_eq!(rows[3].report_event, 1.0);
    }

    #[test]
    fn extract_error_and_empty() {
        let err = extract_filings(&json!({"error": "no access"})).unwrap_err();
        assert!(err.contains("no access"), "{err}");
        assert!(extract_filings(&json!({"data": []})).unwrap().is_empty());
        assert!(extract_filings(&json!({"cik": "1"})).unwrap().is_empty());
        assert!(extract_filings(&json!([1, 2])).is_err());
    }

    struct OneShot(Vec<u8>);
    impl HttpClient for OneShot {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.0.clone())
        }
    }
    struct FailHttp;
    impl HttpClient for FailHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Status(500))
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

    #[test]
    fn financials_url_shape() {
        let u = financials_url("AAPL", "TOK");
        assert!(u.contains("/stock/financials-reported?symbol=AAPL"));
        assert!(u.contains("freq=annual"));
        assert!(u.contains("token=TOK"));
    }

    #[test]
    fn sync_fundamentals_writes_file() {
        let http = OneShot(serde_json::to_vec(&sample(Some("2024-02-15"))).unwrap());
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_fh_fund_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        // Price days straddling both filing visibility dates.
        let days = [20240101, 20240216, 20250201];
        let wrote = sync_fundamentals(&fetcher, &store, "AAPL", "AAPL", "tok", &days).unwrap();
        assert!(wrote);
        assert!(dir.join("fundamentals/AAPL.csv.gz").exists());
    }

    #[test]
    fn sync_fundamentals_empty_returns_false() {
        let http = OneShot(br#"{"data":[]}"#.to_vec());
        let cfg = cfg();
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_fh_fund_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        let wrote = sync_fundamentals(&fetcher, &store, "X", "X", "tok", &[20240102]).unwrap();
        assert!(!wrote);
    }

    #[test]
    fn sync_fundamentals_http_error_propagates() {
        let cfg = cfg();
        let fetcher = Fetcher::new(&FailHttp, &cfg);
        let dir = std::env::temp_dir().join("pomelo_fh_fund_err");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        assert!(sync_fundamentals(&fetcher, &store, "X", "X", "tok", &[20240102]).is_err());
    }
}
