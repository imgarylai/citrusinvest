//! Price URL construction, row parsing, and local merge reads.

use std::collections::BTreeMap;

use pomelo_data::csv_io::{parse_series, Field, OhlcvRow};
use pomelo_data::{ObjectSource, PRICES_DIR};
use serde_json::Value;

use super::config::SyncConfig;
use super::util::{i32_to_iso, iso_to_i32, num};
use super::FMP_BASE;

pub(crate) fn price_url(sym: &str, cfg: &SyncConfig, key: &str) -> String {
    format!(
        "{FMP_BASE}/stable/historical-price-eod/dividend-adjusted?symbol={sym}&from={}&to={}&apikey={key}",
        i32_to_iso(cfg.from),
        i32_to_iso(cfg.to)
    )
}
pub(crate) fn parse_price_rows(rows: &[Value], cfg: &SyncConfig) -> Vec<OhlcvRow> {
    let mut out: Vec<OhlcvRow> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            let day = obj
                .get("date")
                .and_then(Value::as_str)
                .and_then(iso_to_i32)?;
            if day < cfg.from || day > cfg.to {
                return None;
            }
            let close = num(obj, &["adjClose", "close"])?;
            let open = num(obj, &["adjOpen", "open"]).unwrap_or(close);
            let high = num(obj, &["adjHigh", "high"]).unwrap_or(close);
            let low = num(obj, &["adjLow", "low"]).unwrap_or(close);
            let volume = num(obj, &["volume"]).unwrap_or(0.0);
            Some(OhlcvRow {
                day,
                adj_open: open,
                adj_high: high,
                adj_low: low,
                adj_close: close,
                volume,
            })
        })
        .collect();
    out.sort_by_key(|r| r.day);
    out.dedup_by_key(|r| r.day);
    out
}
pub(crate) fn read_existing_prices(src: &impl ObjectSource, sym: &str) -> BTreeMap<i32, OhlcvRow> {
    let key = format!("{PRICES_DIR}/{sym}.csv.gz");
    let Some(bytes) = src.get(&key).ok().flatten() else {
        return BTreeMap::new();
    };
    let col = |f| parse_series(&bytes, f).unwrap_or_default();
    let mut rows: BTreeMap<i32, OhlcvRow> = BTreeMap::new();
    for (d, v) in col(Field::AdjClose) {
        rows.entry(d).or_insert(OhlcvRow {
            day: d,
            adj_open: v,
            adj_high: v,
            adj_low: v,
            adj_close: v,
            volume: 0.0,
        });
    }
    for (d, v) in col(Field::AdjOpen) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_open = v;
        }
    }
    for (d, v) in col(Field::AdjHigh) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_high = v;
        }
    }
    for (d, v) in col(Field::AdjLow) {
        if let Some(r) = rows.get_mut(&d) {
            r.adj_low = v;
        }
    }
    for (d, v) in col(Field::Volume) {
        if let Some(r) = rows.get_mut(&d) {
            r.volume = v;
        }
    }
    rows
}
