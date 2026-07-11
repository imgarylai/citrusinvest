//! Annual fundamentals fetch, densify, and write.

use std::collections::BTreeMap;

use pomelo_data::fundamentals::{
    write_fundamentals, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS,
};
use pomelo_data::ObjectSink;
use serde_json::Value;

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

/// Candidate JSON field names carrying the date a report actually became
/// public — the SEC filing / acceptance date — newest-API spelling first. FMP
/// has shipped both `filingDate` and the older misspelled `fillingDate`;
/// `acceptedDate` is the acceptance timestamp (may include a time suffix, which
/// [`iso_to_i32`] tolerates). Preferred over the fiscal period-end `date` so a
/// snapshot only becomes visible once it was disclosed, not on the (earlier)
/// period-end day — otherwise every fundamentals backtest peeks ahead (#131).
pub(crate) const FILING_DATE_KEYS: &[&str] = &["filingDate", "fillingDate", "acceptedDate"];

/// One merged fundamentals snapshot for a single fiscal period.
pub(crate) struct Snapshot {
    /// Day the data became public: the filing / acceptance date when present,
    /// otherwise the fiscal period-end (see [`Snapshot::fell_back`]). The engine
    /// only "sees" a snapshot on/after this day.
    pub visible: i32,
    /// [`FUNDAMENTAL_FIELDS`]-aligned values; unmatched fields are `NaN`.
    pub values: Vec<f64>,
    /// `true` when no filing date was available and `visible` fell back to the
    /// fiscal period-end — an optimistic (potentially lookahead) visibility.
    pub fell_back: bool,
}

pub(crate) fn annual_url(endpoint: &str, sym: &str, key: &str) -> String {
    // A generous limit: dense forward-fill covers the trading calendar from
    // whatever annual snapshots the plan returns.
    format!("{FMP_BASE}/stable/{endpoint}?symbol={sym}&period=annual&limit=40&apikey={key}")
}
pub(crate) fn merge_fundamentals(bodies: &[Vec<Value>]) -> Vec<Snapshot> {
    // Group by the fiscal period-end `date` — the stable key every endpoint
    // shares (a filing date may differ per endpoint or be absent). Fields from
    // later bodies only fill gaps ("first present wins"), so an endpoint added
    // purely for its filing date never clobbers an existing value.
    let mut by_period: BTreeMap<i32, serde_json::Map<String, Value>> = BTreeMap::new();
    for rows in bodies {
        for row in rows {
            let Some(obj) = row.as_object() else { continue };
            let Some(d) = obj.get("date").and_then(Value::as_str).and_then(iso_to_i32) else {
                continue;
            };
            let entry = by_period.entry(d).or_default();
            for (k, v) in obj {
                entry.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    let mut snapshots: Vec<Snapshot> = by_period
        .into_iter()
        .map(|(period_end, obj)| {
            let values = FUNDAMENTAL_KEYS
                .iter()
                .map(|keys| num(&obj, keys).unwrap_or(f64::NAN))
                .collect();
            // Visible on the filing/acceptance day when disclosed; otherwise
            // fall back to the period-end (flagged so the caller can report it).
            let filed = FILING_DATE_KEYS
                .iter()
                .find_map(|k| obj.get(*k).and_then(Value::as_str).and_then(iso_to_i32));
            let (visible, fell_back) = match filed {
                Some(f) => (f, false),
                None => (period_end, true),
            };
            Snapshot {
                visible,
                values,
                fell_back,
            }
        })
        .collect();
    // Apply snapshots in the order the market saw them: a later fiscal period is
    // usually filed later, but a restatement can invert that, so sort by
    // visibility rather than trusting the period-end ordering.
    snapshots.sort_by_key(|s| s.visible);
    snapshots
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
pub(crate) fn sync_fundamentals<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sink: &impl ObjectSink,
    sym: &str,
    api_key: &str,
    price_days: &[i32],
) -> Result<bool, String> {
    eprintln!("{sym}: fetching fundamentals…");
    // Primary factor sources — a failure here fails the symbol's fundamentals.
    let mut bodies: Vec<Vec<Value>> = ["ratios", "key-metrics", "financial-growth"]
        .iter()
        .map(|ep| fetcher.get_rows(&annual_url(ep, sym, api_key)))
        .collect::<Result<_, _>>()?;
    // Filing/acceptance dates come from `income-statement` — the ratio/metric
    // endpoints carry only the fiscal period-end `date`, so without it every
    // snapshot would peek ahead (#131). It also backfills any factor (e.g.
    // `revenue`) the ratio endpoints leave unset. Best-effort: if a plan doesn't
    // serve it, snapshots fall back to period-end visibility (flagged below)
    // rather than failing the whole fundamentals sync.
    match fetcher.get_rows(&annual_url("income-statement", sym, api_key)) {
        Ok(rows) => bodies.push(rows),
        Err(e) => eprintln!(
            "{sym}: income-statement unavailable ({e}); \
             fundamentals fall back to fiscal period-end visibility"
        ),
    }
    let snapshots = merge_fundamentals(&bodies);
    if snapshots.is_empty() {
        return Ok(false);
    }
    let fell_back = snapshots.iter().filter(|s| s.fell_back).count();
    let rows = densify_fundamentals(&snapshots, price_days);
    let bytes = write_fundamentals(&rows).map_err(|e| e.to_string())?;
    sink.put(&format!("{FUNDAMENTALS_DIR}/{sym}.csv.gz"), &bytes)
        .map_err(|e| e.to_string())?;
    if fell_back > 0 {
        eprintln!(
            "{sym}: wrote {} fundamental rows ({} annual snapshots; {fell_back} had no filing \
             date → visible on fiscal period-end, may be optimistic)",
            rows.len(),
            snapshots.len(),
        );
    } else {
        eprintln!(
            "{sym}: wrote {} fundamental rows ({} annual snapshots, filing-date visibility)",
            rows.len(),
            snapshots.len(),
        );
    }
    Ok(true)
}
