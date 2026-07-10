//! Annual fundamentals fetch, densify, and write.

use std::collections::BTreeMap;

use serde_json::Value;
use yuzu_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use yuzu_data::{LocalSource, ObjectSink};

use super::http::Fetcher;
use super::util::{iso_to_i32, num};
use super::HttpClient;
use super::FMP_BASE;

/// Candidate JSON field names for each [`FUNDAMENTAL_FIELDS`] entry, in the same
/// order. FMP has renamed fields across API versions, so each slot lists the
/// aliases we accept; the first present numeric one wins. Unmatched → `NaN`
/// (a "missing" factor the engine treats as absent).
const FUNDAMENTAL_KEYS: &[&[&str]] = &[
    &["priceToEarningsRatio", "peRatio", "priceEarningsRatio"], // pe
    &["priceToSalesRatio", "priceSalesRatio"],                  // ps
    &["priceToBookRatio", "priceBookValueRatio", "pbRatio"],    // pb
    &["returnOnEquity"],                                        // roe
    &["netProfitMargin", "netIncomeMargin"],                    // net_margin
    &["debtToEquityRatio", "debtEquityRatio", "debtToEquity"],  // debt_to_equity
    &["marketCap", "marketCapitalization"],                     // market_cap
    &["grossProfitMargin"],                                     // gross_margin
    &["receivablesTurnover"],                                   // receivables_turnover
    &[
        "debtToAssetsRatio",
        "debtToAssets",
        "totalDebtToTotalAssets",
    ], // debt_to_assets
    &["revenue"],                                               // revenue
    &["revenueGrowth", "growthRevenue"],                        // revenue_growth
    &["epsgrowth", "epsGrowth", "growthEPS"],                   // eps_growth
    &["operatingIncomeGrowth", "growthOperatingIncome"],        // operating_income_growth
    &["netIncomeGrowth", "growthNetIncome"],                    // net_income_growth
    &["grossProfitGrowth", "growthGrossProfit"],                // gross_profit_growth
];

pub(crate) fn annual_url(endpoint: &str, sym: &str, key: &str) -> String {
    // A generous limit: dense forward-fill covers the trading calendar from
    // whatever annual snapshots the plan returns.
    format!("{FMP_BASE}/stable/{endpoint}?symbol={sym}&period=annual&limit=40&apikey={key}")
}
pub(crate) fn merge_fundamentals(bodies: &[Vec<Value>]) -> Vec<(i32, Vec<f64>)> {
    let mut by_date: BTreeMap<i32, serde_json::Map<String, Value>> = BTreeMap::new();
    for rows in bodies {
        for row in rows {
            let Some(obj) = row.as_object() else { continue };
            let Some(d) = obj.get("date").and_then(Value::as_str).and_then(iso_to_i32) else {
                continue;
            };
            let entry = by_date.entry(d).or_default();
            for (k, v) in obj {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    by_date
        .into_iter()
        .map(|(d, obj)| {
            let values = FUNDAMENTAL_KEYS
                .iter()
                .map(|keys| num(&obj, keys).unwrap_or(f64::NAN))
                .collect();
            (d, values)
        })
        .collect()
}
pub(crate) fn densify_fundamentals(
    snapshots: &[(i32, Vec<f64>)],
    price_days: &[i32],
) -> Vec<FundamentalRow> {
    let nfields = FUNDAMENTAL_FIELDS.len();
    let mut rows = Vec::with_capacity(price_days.len());
    let mut si = 0usize;
    let mut current = vec![f64::NAN; nfields];
    for &day in price_days {
        let mut event = 0.0;
        while si < snapshots.len() && snapshots[si].0 <= day {
            current = snapshots[si].1.clone();
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
pub(crate) fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &LocalSource,
    sym: &str,
    api_key: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{sym}: fetching fundamentals…");
    let bodies: Vec<Vec<Value>> = ["ratios", "key-metrics", "financial-growth"]
        .iter()
        .map(|ep| fetcher.get_rows(&annual_url(ep, sym, api_key)))
        .collect::<Result<_, _>>()?;
    let snapshots = merge_fundamentals(&bodies);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "{sym}: wrote {} fundamental rows ({} annual snapshots)",
        rows.len(),
        snapshots.len()
    );
    Ok(true)
}
