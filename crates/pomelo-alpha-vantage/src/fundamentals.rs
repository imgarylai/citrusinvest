//! Annual fundamentals from Alpha Vantage statements → dense `fundamentals/{SYM}.csv.gz`.
//!
//! Builds layout [`FUNDAMENTAL_FIELDS`] from annual Income Statement + Balance
//! Sheet (ratios + YoY growth). **No filing_date on AV statements** (#207) →
//! visibility is fiscal period-end (optimistic). Price multiples and
//! `market_cap` left NaN historically.

use std::collections::BTreeMap;

use pomelo_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use pomelo_data::ObjectSink;
use serde_json::Value;

use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::ALPHA_VANTAGE_BASE;

/// One fiscal-period snapshot aligned to [`FUNDAMENTAL_FIELDS`].
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub visible: i32,
    pub values: Vec<f64>,
    /// Always true for AV today (period-end visibility only).
    pub fell_back: bool,
}

pub(crate) fn income_statement_url(av_symbol: &str, api_key: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=INCOME_STATEMENT&symbol={av_symbol}&apikey={api_key}")
}

pub(crate) fn balance_sheet_url(av_symbol: &str, api_key: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=BALANCE_SHEET&symbol={av_symbol}&apikey={api_key}")
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

fn field(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| obj.get(*k).and_then(as_f64))
}

fn str_field(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        obj.get(*k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "None" && *s != "-")
            .map(str::to_string)
    })
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

fn check_av_error(root: &serde_json::Map<String, Value>) -> Result<(), String> {
    for err_key in ["Error Message", "Information", "Note"] {
        if let Some(msg) = root.get(err_key).and_then(Value::as_str) {
            return Err(format!("Alpha Vantage {err_key}: {msg}"));
        }
    }
    Ok(())
}

/// Extract annual report objects keyed by fiscal period-end date string.
pub(crate) fn extract_annual_reports(value: &Value) -> Result<BTreeMap<String, Value>, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "fundamentals payload is not a JSON object".to_string())?;
    check_av_error(root)?;
    let mut out = BTreeMap::new();
    let Some(arr) = root.get("annualReports").and_then(Value::as_array) else {
        return Ok(out);
    };
    for v in arr {
        let Some(obj) = v.as_object() else { continue };
        let key = str_field(obj, &["fiscalDateEnding", "fiscal_date_ending"]).or_else(|| {
            // fallback: any date-like field
            str_field(obj, &["date", "Date"])
        });
        let Some(key) = key else { continue };
        out.insert(key, v.clone());
    }
    Ok(out)
}

fn period_end(obj: &serde_json::Map<String, Value>, key_hint: &str) -> Option<i32> {
    str_field(
        obj,
        &["fiscalDateEnding", "fiscal_date_ending", "date", "Date"],
    )
    .and_then(|s| iso_to_i32(&s))
    .or_else(|| iso_to_i32(key_hint))
}

type StmtObj = serde_json::Map<String, Value>;

/// Build sorted snapshots from annual IS + BS maps (period-end visibility).
pub(crate) fn build_snapshots(
    is_y: &BTreeMap<String, Value>,
    bs_y: &BTreeMap<String, Value>,
) -> Vec<Snapshot> {
    let mut periods: BTreeMap<i32, (Option<&StmtObj>, Option<&StmtObj>)> = BTreeMap::new();

    for (k, v) in is_y {
        let Some(obj) = v.as_object() else { continue };
        let Some(pe) = period_end(obj, k) else {
            continue;
        };
        periods.entry(pe).or_default().0 = Some(obj);
    }
    for (k, v) in bs_y {
        let Some(obj) = v.as_object() else { continue };
        let Some(pe) = period_end(obj, k) else {
            continue;
        };
        periods.entry(pe).or_default().1 = Some(obj);
    }

    let mut ordered: Vec<(i32, Option<&StmtObj>, Option<&StmtObj>)> = periods
        .into_iter()
        .map(|(pe, (is, bs))| (pe, is, bs))
        .collect();
    ordered.sort_by_key(|(pe, _, _)| *pe);

    let mut snaps = Vec::new();
    let mut prev_is: Option<&StmtObj> = None;

    for (pe, is, bs) in ordered {
        let Some(is) = is else {
            continue;
        };
        let rev = field(is, &["totalRevenue", "revenue"]);
        let gp = field(is, &["grossProfit"]);
        let ni = field(is, &["netIncome", "netIncomeApplicableToCommonShares"]);
        let oi = field(is, &["operatingIncome", "ebit"]);

        let equity = bs.and_then(|b| {
            field(
                b,
                &[
                    "totalShareholderEquity",
                    "totalStockholderEquity",
                    "totalShareholdersEquity",
                ],
            )
        });
        let assets = bs.and_then(|b| field(b, &["totalAssets"]));
        let liab = bs.and_then(|b| field(b, &["totalLiabilities", "totalLiab"]));
        let debt = bs.and_then(|b| {
            field(
                b,
                &[
                    "shortLongTermDebtTotal",
                    "longTermDebt",
                    "shortTermDebt",
                    "currentLongTermDebt",
                    "netDebt",
                ],
            )
        });
        let recv = bs.and_then(|b| {
            field(
                b,
                &[
                    "currentNetReceivables",
                    "netReceivables",
                    "accountsReceivable",
                ],
            )
        });

        let prev_rev = prev_is.and_then(|p| field(p, &["totalRevenue", "revenue"]));
        let prev_ni =
            prev_is.and_then(|p| field(p, &["netIncome", "netIncomeApplicableToCommonShares"]));
        let prev_oi = prev_is.and_then(|p| field(p, &["operatingIncome", "ebit"]));
        let prev_gp = prev_is.and_then(|p| field(p, &["grossProfit"]));

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

        // AV has no filing_date → always period-end (fell_back = true).
        snaps.push(Snapshot {
            visible: pe,
            values,
            fell_back: true,
        });
        prev_is = Some(is);
    }

    snaps.sort_by_key(|s| s.visible);
    snaps
}

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

/// Fetch IS + BS annual, densify onto `price_days`, write fundamentals file.
pub(crate) fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &impl ObjectSink,
    layout_sym: &str,
    av_symbol: &str,
    api_key: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{layout_sym}: fetching fundamentals ({av_symbol})…");
    let is_val = fetcher.get_json(&income_statement_url(av_symbol, api_key))?;
    let bs_val = fetcher.get_json(&balance_sheet_url(av_symbol, api_key))?;
    let is_y = extract_annual_reports(&is_val)?;
    let bs_y = extract_annual_reports(&bs_val)?;
    let snapshots = build_snapshots(&is_y, &bs_y);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let fell_back = snapshots.iter().filter(|s| s.fell_back).count();
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{layout_sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{layout_sym}: wrote {} fundamental rows ({} annual snapshots; {fell_back} use fiscal \
         period-end visibility — AV has no filing_date, may be optimistic)",
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

    fn sample_is() -> Value {
        json!({
            "symbol": "IBM",
            "annualReports": [
                {
                    "fiscalDateEnding": "2023-12-31",
                    "totalRevenue": "100.0",
                    "grossProfit": "40.0",
                    "netIncome": "20.0",
                    "operatingIncome": "30.0"
                },
                {
                    "fiscalDateEnding": "2024-12-31",
                    "totalRevenue": "110.0",
                    "grossProfit": "44.0",
                    "netIncome": "22.0",
                    "operatingIncome": "33.0"
                }
            ]
        })
    }

    fn sample_bs() -> Value {
        json!({
            "symbol": "IBM",
            "annualReports": [
                {
                    "fiscalDateEnding": "2023-12-31",
                    "totalAssets": "200.0",
                    "totalLiabilities": "80.0",
                    "totalShareholderEquity": "120.0",
                    "currentNetReceivables": "10.0",
                    "longTermDebt": "50.0"
                },
                {
                    "fiscalDateEnding": "2024-12-31",
                    "totalAssets": "220.0",
                    "totalLiabilities": "88.0",
                    "totalShareholderEquity": "132.0",
                    "currentNetReceivables": "11.0",
                    "longTermDebt": "55.0"
                }
            ]
        })
    }

    #[test]
    fn extract_and_build_snapshots() {
        let is_y = extract_annual_reports(&sample_is()).unwrap();
        let bs_y = extract_annual_reports(&sample_bs()).unwrap();
        assert_eq!(is_y.len(), 2);
        let snaps = build_snapshots(&is_y, &bs_y);
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].visible, 20231231);
        assert!(snaps[0].fell_back);
        // revenue 100
        assert!((snaps[0].values[10] - 100.0).abs() < 1e-9);
        // net_margin 20/100
        assert!((snaps[0].values[4] - 0.2).abs() < 1e-9);
        // roe 20/120
        assert!((snaps[0].values[3] - 20.0 / 120.0).abs() < 1e-9);
        // revenue growth 10%
        assert!((snaps[1].values[11] - 0.1).abs() < 1e-9);
        assert!(snaps[0].values[0].is_nan()); // pe
    }

    #[test]
    fn densify_marks_events_on_period_end() {
        let snaps = vec![
            Snapshot {
                visible: 20240102,
                values: vec![1.0; FUNDAMENTAL_FIELDS.len()],
                fell_back: true,
            },
            Snapshot {
                visible: 20240104,
                values: vec![2.0; FUNDAMENTAL_FIELDS.len()],
                fell_back: true,
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
    fn extract_rejects_note_envelope() {
        let v = json!({"Note": "rate limited"});
        assert!(extract_annual_reports(&v).unwrap_err().contains("Note"));
    }

    struct QueueHttp {
        bodies: std::cell::RefCell<Vec<Vec<u8>>>,
    }
    impl HttpClient for QueueHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            let mut q = self.bodies.borrow_mut();
            if q.is_empty() {
                Err(HttpError::Status(500))
            } else {
                Ok(q.remove(0))
            }
        }
    }

    #[test]
    fn sync_fundamentals_writes_file() {
        let bodies = vec![
            serde_json::to_vec(&sample_is()).unwrap(),
            serde_json::to_vec(&sample_bs()).unwrap(),
        ];
        let http = QueueHttp {
            bodies: std::cell::RefCell::new(bodies),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_av_fund_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        let days = [20230101, 20240102, 20250101];
        let wrote = sync_fundamentals(&fetcher, &store, "IBM", "IBM", "tok", &days).unwrap();
        assert!(wrote);
        assert!(dir.join("fundamentals/IBM.csv.gz").exists());
    }

    #[test]
    fn sync_fundamentals_empty_returns_false() {
        let empty = br#"{"symbol":"X","annualReports":[]}"#;
        let http = QueueHttp {
            bodies: std::cell::RefCell::new(vec![empty.to_vec(), empty.to_vec()]),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_av_fund_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        let wrote = sync_fundamentals(&fetcher, &store, "X", "X", "tok", &[20240102]).unwrap();
        assert!(!wrote);
    }
}
