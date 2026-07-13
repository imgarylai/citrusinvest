//! Alpha Vantage daily adjusted prices → layout OHLCV.

use std::collections::BTreeMap;

use pomelo_data::csv_io::{parse_series, Field, OhlcvRow};
use pomelo_data::{ObjectSource, PRICES_DIR};
use serde_json::Value;

use super::config::SyncConfig;
use super::util::{iso_to_i32, num_av_field};
use super::ALPHA_VANTAGE_BASE;

/// Build `TIME_SERIES_DAILY_ADJUSTED` URL for one AV symbol.
///
/// Uses `outputsize=full` so multi-year backtests are possible when the key
/// allows it (compact ≈ last 100 bars only — see spike #207).
pub(crate) fn price_url(av_symbol: &str, api_key: &str) -> String {
    format!(
        "{ALPHA_VANTAGE_BASE}?function=TIME_SERIES_DAILY_ADJUSTED\
         &symbol={av_symbol}&outputsize=full&datatype=json&apikey={api_key}"
    )
}

/// Extract the daily series map from an AV JSON payload, or a clear error.
pub(crate) fn extract_daily_series(
    value: &Value,
) -> Result<&serde_json::Map<String, Value>, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "Alpha Vantage price payload is not a JSON object".to_string())?;

    for err_key in ["Error Message", "Information", "Note"] {
        if let Some(msg) = root.get(err_key).and_then(Value::as_str) {
            return Err(format!("Alpha Vantage {err_key}: {msg}"));
        }
    }

    if let Some(Value::Object(m)) = root.get("Time Series (Daily)") {
        return Ok(m);
    }
    // Fallback: any key containing "Time Series" and "Daily"
    for (k, v) in root {
        if k.contains("Time Series") && k.contains("Daily") {
            if let Value::Object(m) = v {
                return Ok(m);
            }
        }
    }
    Err("Alpha Vantage response missing Time Series (Daily)".into())
}

/// Parse AV daily adjusted JSON into layout [`OhlcvRow`]s.
///
/// AV returns raw O/H/L/C + `adjusted close`. Scale OHLC by
/// `adjusted_close / close` (same policy as EODHD / docs § Alpha Vantage).
pub(crate) fn parse_price_payload(
    value: &Value,
    cfg: &SyncConfig,
) -> Result<Vec<OhlcvRow>, String> {
    let series = extract_daily_series(value)?;
    let mut out: Vec<OhlcvRow> = series
        .iter()
        .filter_map(|(date_s, cell)| {
            let day = iso_to_i32(date_s)?;
            if day < cfg.from || day > cfg.to {
                return None;
            }
            let obj = cell.as_object()?;
            let close = num_av_field(obj, "close")?;
            let adj_close = num_av_field(obj, "adjusted close").unwrap_or(close);
            let factor = if close != 0.0 && close.is_finite() {
                adj_close / close
            } else {
                1.0
            };
            let open = num_av_field(obj, "open").unwrap_or(close);
            let high = num_av_field(obj, "high").unwrap_or(close);
            let low = num_av_field(obj, "low").unwrap_or(close);
            let volume = num_av_field(obj, "volume").unwrap_or(0.0);
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
    Ok(out)
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

    fn sample_payload() -> Value {
        json!({
            "Meta Data": {"1. Information": "Daily"},
            "Time Series (Daily)": {
                "2024-01-04": {
                    "1. open": "11.0",
                    "2. high": "12.0",
                    "3. low": "10.5",
                    "4. close": "11.5",
                    "5. adjusted close": "11.5",
                    "6. volume": "1200"
                },
                "2024-01-03": {
                    "1. open": "10.1",
                    "2. high": "11.5",
                    "3. low": "9.8",
                    "4. close": "10.8",
                    "5. adjusted close": "10.8",
                    "6. volume": "1100"
                },
                "2024-01-02": {
                    "1. open": "102.0",
                    "2. high": "110.0",
                    "3. low": "98.0",
                    "4. close": "100.0",
                    "5. adjusted close": "50.0",
                    "6. volume": "1000"
                },
                "2024-01-01": {
                    "1. open": "1.0",
                    "2. high": "1.0",
                    "3. low": "1.0",
                    "4. close": "1.0",
                    "5. adjusted close": "1.0",
                    "6. volume": "1"
                }
            }
        })
    }

    #[test]
    fn price_url_shape() {
        let u = price_url("IBM", "TOK");
        assert!(u.contains("function=TIME_SERIES_DAILY_ADJUSTED"));
        assert!(u.contains("symbol=IBM"));
        assert!(u.contains("outputsize=full"));
        assert!(u.contains("apikey=TOK"));
    }

    #[test]
    fn scales_ohlc_and_filters_window() {
        let out = parse_price_payload(&sample_payload(), &cfg()).unwrap();
        assert_eq!(out.len(), 3); // 01-02..01-04
        assert_eq!(out[0].day, 20240102);
        assert!((out[0].adj_close - 50.0).abs() < 1e-9);
        assert!((out[0].adj_open - 51.0).abs() < 1e-9);
        assert!((out[0].adj_high - 55.0).abs() < 1e-9);
        assert!((out[0].adj_low - 49.0).abs() < 1e-9);
        assert_eq!(out[0].volume, 1000.0);
        assert_eq!(out[2].day, 20240104);
    }

    #[test]
    fn error_message_surfaces() {
        let v = json!({"Error Message": "Invalid API call."});
        let err = parse_price_payload(&v, &cfg()).unwrap_err();
        assert!(err.contains("Invalid API call"), "{err}");
    }

    #[test]
    fn note_rate_limit_surfaces() {
        let v = json!({"Note": "Thank you for using Alpha Vantage! Our standard API call frequency is…"});
        let err = parse_price_payload(&v, &cfg()).unwrap_err();
        assert!(err.contains("Note"), "{err}");
    }

    #[test]
    fn zero_close_uses_factor_one() {
        let v = json!({
            "Time Series (Daily)": {
                "2024-01-02": {
                    "1. open": "1.0",
                    "2. high": "1.0",
                    "3. low": "1.0",
                    "4. close": "0.0",
                    "5. adjusted close": "5.0",
                    "6. volume": "1"
                }
            }
        });
        let out = parse_price_payload(&v, &cfg()).unwrap();
        assert_eq!(out[0].adj_close, 5.0);
        assert_eq!(out[0].adj_open, 1.0);
    }

    #[test]
    fn read_existing_prices_roundtrip() {
        use pomelo_data::csv_io::write_series;
        use pomelo_data::LocalSource;

        let dir = std::env::temp_dir().join("pomelo_av_read_existing");
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
        assert_eq!(r.volume, 9.0);
        assert!(read_existing_prices(&src, "MISSING").is_empty());
    }
}
