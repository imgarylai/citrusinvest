//! Finnhub daily candles → layout OHLCV.

use std::collections::BTreeMap;

use pomelo_data::csv_io::{parse_series, Field, OhlcvRow};
use pomelo_data::{ObjectSource, PRICES_DIR};
use serde_json::Value;

use super::config::SyncConfig;
use super::util::{i32_to_unix, unix_to_i32};
use super::FINNHUB_BASE;

/// Build the `/stock/candle` URL for one Finnhub symbol (daily, adjusted).
///
/// `from`/`to` are UNIX seconds. We request `adjusted=true` so split/dividend
/// adjusted OHLC come back directly — no local `adj_close/close` rescale like
/// AV/EODHD. The window is padded one day on each side so a caller's local
/// calendar bounds still capture the boundary trading days regardless of the
/// exchange time zone; [`parse_price_payload`] re-filters to `[from, to]`.
///
/// Unadjusted risk: some plans ignore `adjusted` and return raw candles — see
/// `docs/data-sources.md` § Finnhub (Adjusted OHLCV row). Free/low tiers also
/// cap the range per request, so long histories need multiple stitched windows
/// (handled by the caller's `--from`/`--to` plus `--append`).
pub(crate) fn price_url(fh_symbol: &str, cfg: &SyncConfig, api_key: &str) -> String {
    let from = i32_to_unix(cfg.from).saturating_sub(86_400);
    let to = i32_to_unix(cfg.to).saturating_add(86_400);
    format!(
        "{FINNHUB_BASE}/stock/candle?symbol={fh_symbol}&resolution=D&adjusted=true\
         &from={from}&to={to}&token={api_key}"
    )
}

/// Parse a Finnhub `/stock/candle` payload into layout [`OhlcvRow`]s.
///
/// Finnhub returns parallel column arrays (`o/h/l/c/v/t`) plus a status `s`
/// (`"ok"` / `"no_data"`). With `adjusted=true` the OHLC are already adjusted,
/// so we map them straight through and only re-filter to the `[from, to]`
/// window. Rows are returned oldest-first, deduped by day.
pub(crate) fn parse_price_payload(
    value: &Value,
    cfg: &SyncConfig,
) -> Result<Vec<OhlcvRow>, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "Finnhub candle payload is not a JSON object".to_string())?;

    // Explicit API error object (e.g. bad key / access denied).
    if let Some(err) = root.get("error").and_then(Value::as_str) {
        return Err(format!("Finnhub error: {err}"));
    }

    match root.get("s").and_then(Value::as_str) {
        Some("ok") => {}
        Some("no_data") => return Ok(Vec::new()),
        Some(other) => return Err(format!("Finnhub candle status: {other}")),
        None => return Err("Finnhub candle response missing status `s`".into()),
    }

    let col = |key: &str| -> Vec<f64> {
        root.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect())
            .unwrap_or_default()
    };
    let t = col("t");
    let o = col("o");
    let h = col("h");
    let l = col("l");
    let c = col("c");
    let v = col("v");

    // Close is the required series; fall back to it for any missing OHLC cell.
    let pick = |arr: &[f64], i: usize, default: f64| {
        arr.get(i)
            .copied()
            .filter(|x| x.is_finite())
            .unwrap_or(default)
    };

    let mut out: Vec<OhlcvRow> = Vec::with_capacity(t.len());
    for (i, &ts) in t.iter().enumerate() {
        if !ts.is_finite() {
            continue;
        }
        let day = unix_to_i32(ts as i64);
        if day < cfg.from || day > cfg.to {
            continue;
        }
        let close = pick(&c, i, f64::NAN);
        if !close.is_finite() {
            continue;
        }
        out.push(OhlcvRow {
            day,
            adj_open: pick(&o, i, close),
            adj_high: pick(&h, i, close),
            adj_low: pick(&l, i, close),
            adj_close: close,
            volume: pick(&v, i, 0.0),
        });
    }
    out.sort_by_key(|r| r.day);
    out.dedup_by_key(|r| r.day);
    Ok(out)
}

/// Read an existing `prices/{sym}.csv.gz` back into a day-keyed map for
/// `--append` merges (later-window rows overwrite earlier ones on collision).
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

    // 2024-01-02..04 at 00:00 UTC.
    const T2: i64 = 1_704_153_600;
    const T3: i64 = T2 + 86_400;
    const T4: i64 = T3 + 86_400;

    fn sample_payload() -> Value {
        json!({
            "s": "ok",
            "t": [T2, T3, T4],
            "o": [9.5, 10.1, 11.0],
            "h": [11.0, 11.5, 12.0],
            "l": [9.0, 9.8, 10.5],
            "c": [10.0, 10.8, 11.5],
            "v": [1000, 1100, 1200]
        })
    }

    #[test]
    fn price_url_shape() {
        let u = price_url("AAPL", &cfg(), "TOK");
        assert!(u.contains("/stock/candle?symbol=AAPL"));
        assert!(u.contains("resolution=D"));
        assert!(u.contains("adjusted=true"));
        assert!(u.contains("token=TOK"));
        // window padded by a day on each side
        assert!(u.contains(&format!("from={}", T2 - 86_400)));
        assert!(u.contains(&format!("to={}", i32_to_unix(20240104) + 86_400)));
    }

    #[test]
    fn maps_adjusted_candles_through() {
        let out = parse_price_payload(&sample_payload(), &cfg()).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].day, 20240102);
        assert_eq!(out[0].adj_open, 9.5);
        assert_eq!(out[0].adj_high, 11.0);
        assert_eq!(out[0].adj_low, 9.0);
        assert_eq!(out[0].adj_close, 10.0);
        assert_eq!(out[0].volume, 1000.0);
        assert_eq!(out[2].day, 20240104);
        assert_eq!(out[2].adj_close, 11.5);
    }

    #[test]
    fn filters_window() {
        let mut c = cfg();
        c.from = 20240103;
        c.to = 20240103;
        let out = parse_price_payload(&sample_payload(), &c).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].day, 20240103);
    }

    #[test]
    fn no_data_status_is_empty() {
        let v = json!({ "s": "no_data" });
        assert!(parse_price_payload(&v, &cfg()).unwrap().is_empty());
    }

    #[test]
    fn error_object_surfaces() {
        let v = json!({ "error": "Access denied." });
        let err = parse_price_payload(&v, &cfg()).unwrap_err();
        assert!(err.contains("Access denied"), "{err}");
    }

    #[test]
    fn missing_status_errors() {
        let v = json!({ "t": [T2], "c": [10.0] });
        assert!(parse_price_payload(&v, &cfg()).is_err());
    }

    #[test]
    fn missing_ohlc_falls_back_to_close() {
        // No o/h/l arrays → open/high/low default to close.
        let v = json!({ "s": "ok", "t": [T2], "c": [10.0] });
        let out = parse_price_payload(&v, &cfg()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].adj_open, 10.0);
        assert_eq!(out[0].adj_high, 10.0);
        assert_eq!(out[0].adj_low, 10.0);
        assert_eq!(out[0].volume, 0.0);
    }

    #[test]
    fn read_existing_prices_roundtrip() {
        use pomelo_data::csv_io::write_series;
        use pomelo_data::LocalSource;

        let dir = std::env::temp_dir().join("pomelo_fh_read_existing");
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
