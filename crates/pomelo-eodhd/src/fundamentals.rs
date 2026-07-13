//! Annual fundamentals from EODHD statements → dense `fundamentals/{SYM}.csv.gz`.
//!
//! Builds layout [`FUNDAMENTAL_FIELDS`] from yearly Income Statement + Balance
//! Sheet (ratios + YoY growth). Visibility uses `filing_date` when present
//! (else fiscal period-end). Price multiples (pe/ps/pb) and market_cap are left
//! NaN historically — they need a price series; use statement factors for
//! dense history.

use std::collections::BTreeMap;

use pomelo_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use pomelo_data::ObjectSink;
use serde_json::Value;

use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::EODHD_BASE;

/// One fiscal-period snapshot aligned to [`FUNDAMENTAL_FIELDS`].
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub visible: i32,
    pub values: Vec<f64>,
    pub fell_back: bool,
}

pub(crate) fn fundamentals_url(eodhd_code: &str, api_token: &str) -> String {
    format!(
        "{EODHD_BASE}/v1.1/fundamentals/{eodhd_code}?api_token={api_token}&fmt=json\
         &filter=Financials::Income_Statement::yearly,Financials::Balance_Sheet::yearly"
    )
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
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
            .filter(|s| !s.is_empty())
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

/// Extract yearly IS/BS maps from a filtered fundamentals payload.
pub(crate) fn extract_yearly(value: &Value) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    let mut is_y = BTreeMap::new();
    let mut bs_y = BTreeMap::new();
    let Some(root) = value.as_object() else {
        return (is_y, bs_y);
    };

    // Multi-filter flat keys.
    if let Some(Value::Object(m)) = root.get("Financials::Income_Statement::yearly") {
        for (k, v) in m {
            is_y.insert(k.clone(), v.clone());
        }
    }
    if let Some(Value::Object(m)) = root.get("Financials::Balance_Sheet::yearly") {
        for (k, v) in m {
            bs_y.insert(k.clone(), v.clone());
        }
    }

    // Nested Financials form.
    if is_y.is_empty() || bs_y.is_empty() {
        if let Some(fin) = root.get("Financials").and_then(Value::as_object) {
            if is_y.is_empty() {
                if let Some(m) = fin
                    .get("Income_Statement")
                    .and_then(|x| x.get("yearly"))
                    .and_then(Value::as_object)
                {
                    for (k, v) in m {
                        is_y.insert(k.clone(), v.clone());
                    }
                }
            }
            if bs_y.is_empty() {
                if let Some(m) = fin
                    .get("Balance_Sheet")
                    .and_then(|x| x.get("yearly"))
                    .and_then(Value::as_object)
                {
                    for (k, v) in m {
                        bs_y.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    (is_y, bs_y)
}

fn period_end(obj: &serde_json::Map<String, Value>, key_hint: &str) -> Option<i32> {
    str_field(obj, &["date", "Date"])
        .and_then(|s| iso_to_i32(&s))
        .or_else(|| iso_to_i32(key_hint))
}

fn filing_day(obj: &serde_json::Map<String, Value>, period_end: i32) -> (i32, bool) {
    match str_field(
        obj,
        &["filing_date", "filingDate", "fillingDate", "acceptedDate"],
    )
    .and_then(|s| iso_to_i32(&s))
    {
        Some(f) => (f, false),
        None => (period_end, true),
    }
}

type StmtObj = serde_json::Map<String, Value>;

/// Build sorted snapshots from yearly IS + BS maps.
pub(crate) fn build_snapshots(
    is_y: &BTreeMap<String, Value>,
    bs_y: &BTreeMap<String, Value>,
) -> Vec<Snapshot> {
    // Align periods by period-end date: (income_statement, balance_sheet).
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

        let equity = bs.and_then(|b| field(b, &["totalStockholderEquity"]));
        let assets = bs.and_then(|b| field(b, &["totalAssets"]));
        let liab = bs.and_then(|b| field(b, &["totalLiab", "totalLiabilities"]));
        let debt = bs.and_then(|b| {
            field(
                b,
                &[
                    "shortLongTermDebtTotal",
                    "longTermDebt",
                    "netDebt",
                    "shortTermDebt",
                ],
            )
        });
        let recv = bs.and_then(|b| field(b, &["netReceivables"]));

        let prev_rev = prev_is.and_then(|p| field(p, &["totalRevenue", "revenue"]));
        let prev_ni =
            prev_is.and_then(|p| field(p, &["netIncome", "netIncomeApplicableToCommonShares"]));
        let prev_oi = prev_is.and_then(|p| field(p, &["operatingIncome", "ebit"]));
        let prev_gp = prev_is.and_then(|p| field(p, &["grossProfit"]));

        // FUNDAMENTAL_FIELDS order.
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
            yoy(ni, prev_ni),                // eps_growth (proxy: NI YoY; no EPS field)
            yoy(oi, prev_oi),                // operating_income_growth
            yoy(ni, prev_ni),                // net_income_growth
            yoy(gp, prev_gp),                // gross_profit_growth
        ];
        debug_assert_eq!(values.len(), FUNDAMENTAL_FIELDS.len());

        let (visible, fell_back) = filing_day(is, pe);
        // Prefer BS filing_date if IS missing.
        let (visible, fell_back) = if fell_back {
            if let Some(b) = bs {
                filing_day(b, pe)
            } else {
                (visible, fell_back)
            }
        } else {
            (visible, fell_back)
        };

        snaps.push(Snapshot {
            visible,
            values,
            fell_back,
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

/// Fetch, densify, and write fundamentals for one layout symbol.
pub(crate) fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &impl ObjectSink,
    layout_sym: &str,
    eodhd_code: &str,
    api_token: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{layout_sym}: fetching fundamentals ({eodhd_code})…");
    let value = fetcher.get_json(&fundamentals_url(eodhd_code, api_token))?;
    let (is_y, bs_y) = extract_yearly(&value);
    let snapshots = build_snapshots(&is_y, &bs_y);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let fell_back = snapshots.iter().filter(|s| s.fell_back).count();
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{layout_sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    if fell_back > 0 {
        eprintln!(
            "{layout_sym}: wrote {} fundamental rows ({} annual snapshots; {fell_back} had no filing \
             date → visible on fiscal period-end, may be optimistic)",
            rows.len(),
            snapshots.len(),
        );
    } else {
        eprintln!(
            "{layout_sym}: wrote {} fundamental rows ({} annual snapshots, filing-date visibility)",
            rows.len(),
            snapshots.len(),
        );
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_payload() -> Value {
        json!({
            "Financials::Income_Statement::yearly": {
                "2023-09-30": {
                    "date": "2023-09-30",
                    "filing_date": "2023-11-03",
                    "totalRevenue": 100.0,
                    "grossProfit": 40.0,
                    "netIncome": 20.0,
                    "operatingIncome": 30.0
                },
                "2024-09-30": {
                    "date": "2024-09-30",
                    "filing_date": "2024-11-01",
                    "totalRevenue": 110.0,
                    "grossProfit": 44.0,
                    "netIncome": 22.0,
                    "operatingIncome": 33.0
                }
            },
            "Financials::Balance_Sheet::yearly": {
                "2023-09-30": {
                    "date": "2023-09-30",
                    "filing_date": "2023-11-03",
                    "totalAssets": 200.0,
                    "totalLiab": 80.0,
                    "totalStockholderEquity": 120.0,
                    "netReceivables": 10.0,
                    "shortLongTermDebtTotal": 50.0
                },
                "2024-09-30": {
                    "date": "2024-09-30",
                    "filing_date": "2024-11-01",
                    "totalAssets": 220.0,
                    "totalLiab": 88.0,
                    "totalStockholderEquity": 132.0,
                    "netReceivables": 11.0,
                    "shortLongTermDebtTotal": 55.0
                }
            }
        })
    }

    #[test]
    fn extract_and_build_snapshots() {
        let (is_y, bs_y) = extract_yearly(&sample_payload());
        assert_eq!(is_y.len(), 2);
        let snaps = build_snapshots(&is_y, &bs_y);
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].visible, 20231103);
        assert!(!snaps[0].fell_back);
        // revenue 100
        assert!((snaps[0].values[10] - 100.0).abs() < 1e-9);
        // net_margin 20/100
        assert!((snaps[0].values[4] - 0.2).abs() < 1e-9);
        // roe 20/120
        assert!((snaps[0].values[3] - 20.0 / 120.0).abs() < 1e-9);
        // second period revenue growth 10%
        assert!((snaps[1].values[11] - 0.1).abs() < 1e-9);
        // pe left NaN
        assert!(snaps[0].values[0].is_nan());
    }

    #[test]
    fn densify_marks_events() {
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
        assert_eq!(rows[2].values[0], 1.0);
        assert_eq!(rows[2].report_event, 0.0);
        assert_eq!(rows[3].values[0], 2.0);
        assert_eq!(rows[3].report_event, 1.0);
    }

    #[test]
    fn fallback_visibility_without_filing_date() {
        let payload = json!({
            "Financials::Income_Statement::yearly": {
                "2024-09-30": {
                    "date": "2024-09-30",
                    "totalRevenue": 10.0,
                    "grossProfit": 4.0,
                    "netIncome": 2.0,
                    "operatingIncome": 3.0
                }
            },
            "Financials::Balance_Sheet::yearly": {
                "2024-09-30": {
                    "date": "2024-09-30",
                    "totalAssets": 20.0,
                    "totalLiab": 8.0,
                    "totalStockholderEquity": 12.0
                }
            }
        });
        let (is_y, bs_y) = extract_yearly(&payload);
        let snaps = build_snapshots(&is_y, &bs_y);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].visible, 20240930);
        assert!(snaps[0].fell_back);
    }

    #[test]
    fn extract_nested_financials_form() {
        let payload = json!({
            "Financials": {
                "Income_Statement": {
                    "yearly": {
                        "2024-12-31": {
                            "date": "2024-12-31",
                            "filing_date": "2025-02-15",
                            "totalRevenue": "50",
                            "grossProfit": "20",
                            "netIncome": "10",
                            "operatingIncome": "12"
                        }
                    }
                },
                "Balance_Sheet": {
                    "yearly": {
                        "2024-12-31": {
                            "date": "2024-12-31",
                            "filing_date": "2025-02-15",
                            "totalAssets": "100",
                            "totalLiab": "40",
                            "totalStockholderEquity": "60",
                            "netReceivables": "5",
                            "shortLongTermDebtTotal": "15"
                        }
                    }
                }
            }
        });
        let (is_y, bs_y) = extract_yearly(&payload);
        assert_eq!(is_y.len(), 1);
        assert_eq!(bs_y.len(), 1);
        let snaps = build_snapshots(&is_y, &bs_y);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].visible, 20250215);
        assert!((snaps[0].values[10] - 50.0).abs() < 1e-9);
    }

    #[test]
    fn extract_non_object_root_empty() {
        let (is_y, bs_y) = extract_yearly(&json!([]));
        assert!(is_y.is_empty());
        assert!(bs_y.is_empty());
        assert!(build_snapshots(&is_y, &bs_y).is_empty());
    }

    #[test]
    fn sync_fundamentals_writes_file() {
        use crate::config::SyncConfig;
        use crate::http::{Fetcher, HttpClient, HttpError};
        use pomelo_data::LocalSource;
        use std::time::Duration;

        struct OkHttp(Vec<u8>);
        impl HttpClient for OkHttp {
            fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
                Ok(self.0.clone())
            }
        }

        let body = serde_json::to_vec(&sample_payload()).unwrap();
        let http = OkHttp(body);
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_eodhd_fund_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        let days = [20240102, 20240103, 20241102];
        let wrote = sync_fundamentals(&fetcher, &store, "AAPL", "AAPL.US", "tok", &days).unwrap();
        assert!(wrote);
        assert!(dir.join("fundamentals/AAPL.csv.gz").exists());
    }

    #[test]
    fn sync_fundamentals_empty_returns_false() {
        use crate::config::SyncConfig;
        use crate::http::{Fetcher, HttpClient, HttpError};
        use pomelo_data::LocalSource;
        use std::time::Duration;

        struct OkHttp;
        impl HttpClient for OkHttp {
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
        let http = OkHttp;
        let fetcher = Fetcher::new(&http, &cfg);
        let dir = std::env::temp_dir().join("pomelo_eodhd_fund_empty_unit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = LocalSource::new(&dir);
        let wrote = sync_fundamentals(&fetcher, &store, "X", "X.US", "tok", &[20240102]).unwrap();
        assert!(!wrote);
    }
}
