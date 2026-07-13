//! EOD price URL construction, row parsing (adj OHLC scale), and merge reads.

use std::collections::BTreeMap;

use pomelo_data::csv_io::{parse_series, Field, OhlcvRow};
use pomelo_data::{ObjectSource, PRICES_DIR};
use serde_json::Value;

use super::config::SyncConfig;
use super::util::{i32_to_iso, iso_to_i32, num};
use super::EODHD_BASE;

/// Build the EODHD end-of-day URL for one EODHD code (`AAPL.US`).
pub(crate) fn price_url(eodhd_code: &str, cfg: &SyncConfig, api_token: &str) -> String {
    format!(
        "{EODHD_BASE}/eod/{eodhd_code}?api_token={api_token}&fmt=json&period=d&order=a&from={}&to={}",
        i32_to_iso(cfg.from),
        i32_to_iso(cfg.to)
    )
}

/// Parse EODHD EOD JSON rows into layout [`OhlcvRow`]s.
///
/// EODHD returns raw OHLC + `adjusted_close` (split+dividend adjusted). We scale
/// OHLC by `adjusted_close / close` so citrusquant gets a full adj OHLC bar
/// (see docs/data-sources.md § EODHD mapping).
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
            let close = num(obj, &["close"])?;
            let adj_close = num(obj, &["adjusted_close", "adjustedClose"]).unwrap_or(close);
            let factor = if close != 0.0 && close.is_finite() {
                adj_close / close
            } else {
                1.0
            };
            let open = num(obj, &["open"]).unwrap_or(close);
            let high = num(obj, &["high"]).unwrap_or(close);
            let low = num(obj, &["low"]).unwrap_or(close);
            let volume = num(obj, &["volume"]).unwrap_or(0.0);
            Some(OhlcvRow {
                day,
                adj_open: open * factor,
                adj_high: high * factor,
                adj_low: low * factor,
                adj_close,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> SyncConfig {
        SyncConfig {
            from: 20240102,
            to: 20240104,
            ..SyncConfig::default()
        }
    }

    #[test]
    fn scales_ohlc_by_adj_close_over_close() {
        // close=100, adj=50 → factor 0.5; open 102 → 51
        let rows = vec![json!({
            "date": "2024-01-02",
            "open": 102.0,
            "high": 110.0,
            "low": 98.0,
            "close": 100.0,
            "adjusted_close": 50.0,
            "volume": 1000
        })];
        let out = parse_price_rows(&rows, &cfg());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].day, 20240102);
        assert!((out[0].adj_close - 50.0).abs() < 1e-9);
        assert!((out[0].adj_open - 51.0).abs() < 1e-9);
        assert!((out[0].adj_high - 55.0).abs() < 1e-9);
        assert!((out[0].adj_low - 49.0).abs() < 1e-9);
        assert_eq!(out[0].volume, 1000.0);
    }

    #[test]
    fn filters_to_date_window_and_sorts() {
        let rows = vec![
            json!({"date":"2024-01-04","open":3.0,"high":3.0,"low":3.0,"close":3.0,"adjusted_close":3.0,"volume":1}),
            json!({"date":"2024-01-01","open":1.0,"high":1.0,"low":1.0,"close":1.0,"adjusted_close":1.0,"volume":1}),
            json!({"date":"2024-01-03","open":2.0,"high":2.0,"low":2.0,"close":2.0,"adjusted_close":2.0,"volume":1}),
        ];
        let out = parse_price_rows(&rows, &cfg());
        assert_eq!(
            out.iter().map(|r| r.day).collect::<Vec<_>>(),
            vec![20240103, 20240104]
        );
    }

    #[test]
    fn price_url_contains_code_and_dates() {
        let u = price_url("AAPL.US", &cfg(), "TOK");
        assert!(u.contains("/eod/AAPL.US?"));
        assert!(u.contains("api_token=TOK"));
        assert!(u.contains("from=2024-01-02"));
        assert!(u.contains("to=2024-01-04"));
        assert!(u.contains("fmt=json"));
    }

    #[test]
    fn zero_close_uses_factor_one() {
        let rows = vec![json!({
            "date": "2024-01-02",
            "open": 1.0,
            "high": 1.0,
            "low": 1.0,
            "close": 0.0,
            "adjusted_close": 5.0,
            "volume": 1
        })];
        let out = parse_price_rows(&rows, &cfg());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].adj_close, 5.0);
        assert_eq!(out[0].adj_open, 1.0); // factor 1.0 when close==0
    }

    #[test]
    fn missing_adj_close_falls_back_to_close() {
        let rows = vec![json!({
            "date": "2024-01-02",
            "open": 9.0,
            "high": 11.0,
            "low": 8.0,
            "close": 10.0,
            "volume": 7
        })];
        let out = parse_price_rows(&rows, &cfg());
        assert_eq!(out[0].adj_close, 10.0);
        assert_eq!(out[0].adj_open, 9.0);
        assert_eq!(out[0].volume, 7.0);
    }

    #[test]
    fn skips_non_object_and_bad_dates_dedupes() {
        let rows = vec![
            json!("skip-me"),
            json!({"date": "nope", "close": 1.0}),
            json!({"date": "2024-01-02", "close": 1.0, "adjusted_close": 1.0}),
            json!({"date": "2024-01-02", "close": 2.0, "adjusted_close": 2.0}),
        ];
        let out = parse_price_rows(&rows, &cfg());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].adj_close, 1.0); // first wins after dedup_by_key
    }

    #[test]
    fn read_existing_prices_roundtrip() {
        use pomelo_data::csv_io::write_series;
        use pomelo_data::LocalSource;

        let dir = std::env::temp_dir().join("pomelo_eodhd_read_existing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prices")).unwrap();
        let rows = vec![OhlcvRow {
            day: 20240102,
            adj_open: 1.0,
            adj_high: 2.0,
            adj_low: 0.5,
            adj_close: 1.5,
            volume: 9.0,
        }];
        let bytes = write_series(&rows).unwrap();
        std::fs::write(dir.join("prices/AAA.csv.gz"), bytes).unwrap();
        let src = LocalSource::new(&dir);
        let map = read_existing_prices(&src, "AAA");
        assert_eq!(map.len(), 1);
        let r = map.get(&20240102).unwrap();
        assert_eq!(r.adj_close, 1.5);
        assert_eq!(r.adj_open, 1.0);
        assert_eq!(r.adj_high, 2.0);
        assert_eq!(r.adj_low, 0.5);
        assert_eq!(r.volume, 9.0);

        assert!(read_existing_prices(&src, "MISSING").is_empty());
    }
}
