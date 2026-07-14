//! Sector/industry map from `/stock/profile2` → `tracked/universe.csv.gz`.
//!
//! Finnhub's `finnhubIndustry` is its own taxonomy (≠ FMP/AV sectors — see
//! `docs/data-sources.md` § Finnhub), so don't mix vendor industry strings
//! mid-sample. `marketCapitalization` from profile2 is denominated in **millions**
//! of the listing currency; we scale it to absolute units (`× 1e6`) so the
//! `market_cap` column matches the FMP/AV/EODHD universe convention.
//!
//! ## Delisted (accepted gap, #227)
//!
//! Finnhub has **no** clean delisted feed comparable to Alpha Vantage's
//! `LISTING_STATUS&state=delisted` (spike #208 rated delisted **P**). We do not
//! fabricate one, and there is deliberately no `--include-delisted` union: dead
//! names cannot be enumerated exhaustively from a single Finnhub call, so a
//! Finnhub-only universe is survivor-biased unless the user supplies an external
//! dead-name list. Enumerating the *active* universe belongs to the screener
//! phase (#229), not here.

use std::collections::BTreeMap;

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::ObjectSource;
use serde_json::Value;

use super::http::Fetcher;
use super::HttpClient;
use super::FINNHUB_BASE;

/// Object key the industry snapshot is written under.
pub const INDUSTRY_KEY: &str = "tracked/universe.csv.gz";

/// Bits of Finnhub `/stock/profile2` used for the industry map.
#[derive(Debug, Clone, Default)]
pub(crate) struct Profile {
    pub(crate) industry: Option<String>,
    pub(crate) market_cap: Option<f64>,
}

pub(crate) fn profile_url(fh_symbol: &str, api_key: &str) -> String {
    format!("{FINNHUB_BASE}/stock/profile2?symbol={fh_symbol}&token={api_key}")
}

fn str_field(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "None" && *s != "-")
        .map(str::to_string)
}

fn f64_field(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() || s == "None" || s == "-" {
                None
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

/// Parse a `/stock/profile2` JSON object (or error envelope). An empty object
/// (`{}`) — Finnhub's response for an unknown/dead ticker — yields an empty
/// [`Profile`].
pub(crate) fn parse_profile(value: &Value) -> Result<Profile, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "profile2 payload is not a JSON object".to_string())?;
    if let Some(err) = root.get("error").and_then(Value::as_str) {
        return Err(format!("Finnhub error: {err}"));
    }
    // Market cap arrives in millions of the listing currency → absolute units.
    let market_cap = root
        .get("marketCapitalization")
        .and_then(f64_field)
        .map(|m| m * 1e6);
    Ok(Profile {
        industry: root.get("finnhubIndustry").and_then(str_field),
        market_cap,
    })
}

/// Fetch `/stock/profile2` for one Finnhub symbol; `None` if it carries no
/// industry and no market cap (dead/unknown ticker).
pub(crate) fn fetch_profile<H: HttpClient>(
    fetcher: &Fetcher<H>,
    fh_symbol: &str,
    api_key: &str,
) -> Result<Option<Profile>, String> {
    let value = fetcher.get_json(&profile_url(fh_symbol, api_key))?;
    let p = parse_profile(&value)?;
    if p.industry.is_none() && p.market_cap.is_none() {
        return Ok(None);
    }
    Ok(Some(p))
}

/// Read existing `tracked/universe.csv.gz` so resume does not drop sectors.
pub(crate) fn load_existing_industry(
    src: &impl ObjectSource,
) -> BTreeMap<String, (String, Option<f64>)> {
    let Some(bytes) = src.get(INDUSTRY_KEY).ok().flatten() else {
        return BTreeMap::new();
    };
    let text = decode_csv_text(&bytes);
    parse_industry_csv(&text)
        .into_iter()
        .map(|(sym, sector)| (sym, (sector, None)))
        .collect()
}

pub(crate) fn decode_csv_text(bytes: &[u8]) -> String {
    use std::io::Read;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = String::new();
        if flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .is_ok()
        {
            return out;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Encode industry map as gzip CSV: `symbol,sector,market_cap`.
pub(crate) fn encode_industry(
    industry: &BTreeMap<String, (String, Option<f64>)>,
) -> Result<Vec<u8>, std::io::Error> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut csv = String::from("symbol,sector,market_cap\n");
    for (sym, (sector, mcap)) in industry {
        let mcap = mcap.map(|m| m.to_string()).unwrap_or_default();
        csv.push_str(&format!("{sym},{sector},{mcap}\n"));
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(csv.as_bytes())?;
    enc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncConfig;
    use crate::http::{HttpClient, HttpError};
    use serde_json::json;
    use std::time::Duration;

    fn no_throttle() -> SyncConfig {
        SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    struct OneShot(Result<Vec<u8>, HttpError>);
    impl HttpClient for OneShot {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            self.0.clone()
        }
    }

    #[test]
    fn str_field_filters_placeholders() {
        assert_eq!(
            str_field(&json!("Technology")).as_deref(),
            Some("Technology")
        );
        assert_eq!(str_field(&json!("  Retail ")).as_deref(), Some("Retail"));
        assert!(str_field(&json!("")).is_none());
        assert!(str_field(&json!("None")).is_none());
        assert!(str_field(&json!("-")).is_none());
        assert!(str_field(&json!(42)).is_none());
    }

    #[test]
    fn f64_field_parses_numbers_and_strings() {
        assert_eq!(f64_field(&json!(2500000.0)), Some(2_500_000.0));
        assert_eq!(f64_field(&json!("2500000")), Some(2_500_000.0));
        assert_eq!(f64_field(&json!("  1.5 ")), Some(1.5));
        assert_eq!(f64_field(&json!("None")), None);
        assert_eq!(f64_field(&json!("-")), None);
        assert_eq!(f64_field(&json!("")), None);
        assert_eq!(f64_field(&json!(true)), None);
    }

    #[test]
    fn parse_profile_string_mcap_and_industry_only() {
        // Market cap as a numeric string still scales millions → absolute.
        let p =
            parse_profile(&json!({"finnhubIndustry": "Retail", "marketCapitalization": "1000"}))
                .unwrap();
        assert_eq!(p.industry.as_deref(), Some("Retail"));
        assert_eq!(p.market_cap, Some(1e9));

        // Industry present, market cap absent.
        let p = parse_profile(&json!({"finnhubIndustry": "Energy"})).unwrap();
        assert_eq!(p.industry.as_deref(), Some("Energy"));
        assert!(p.market_cap.is_none());
    }

    #[test]
    fn parse_profile_non_object_errors() {
        let err = parse_profile(&json!([1, 2, 3])).unwrap_err();
        assert!(err.contains("not a JSON object"), "{err}");
    }

    #[test]
    fn fetch_profile_ok_none_and_err() {
        let cfg = no_throttle();
        // Populated profile → Some.
        let http = OneShot(Ok(
            br#"{"finnhubIndustry":"Technology","marketCapitalization":1000}"#.to_vec(),
        ));
        let fetcher = Fetcher::new(&http, &cfg);
        let got = fetch_profile(&fetcher, "AAPL", "tok").unwrap();
        assert_eq!(got.unwrap().industry.as_deref(), Some("Technology"));

        // Empty object (dead ticker) → None.
        let http = OneShot(Ok(b"{}".to_vec()));
        let fetcher = Fetcher::new(&http, &cfg);
        assert!(fetch_profile(&fetcher, "DEAD", "tok").unwrap().is_none());

        // Transport/status failure → Err.
        let http = OneShot(Err(HttpError::Status(403)));
        let fetcher = Fetcher::new(&http, &cfg);
        assert!(fetch_profile(&fetcher, "AAPL", "tok").is_err());
    }

    #[test]
    fn decode_csv_text_reads_plain_bytes() {
        // Non-gzip bytes fall through to lossy UTF-8.
        assert_eq!(
            decode_csv_text(b"symbol,sector,market_cap\n"),
            "symbol,sector,market_cap\n"
        );
    }

    #[test]
    fn load_existing_industry_missing_is_empty() {
        use pomelo_data::LocalSource;
        let dir = std::env::temp_dir().join("pomelo_fh_industry_missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_existing_industry(&LocalSource::new(&dir)).is_empty());
    }

    #[test]
    fn parse_profile_fields_and_mcap_scale() {
        let v = json!({
            "ticker": "AAPL",
            "name": "Apple Inc",
            "finnhubIndustry": "Technology",
            "marketCapitalization": 2500000.0, // millions
            "exchange": "NASDAQ NMS - GLOBAL MARKET"
        });
        let p = parse_profile(&v).unwrap();
        assert_eq!(p.industry.as_deref(), Some("Technology"));
        // 2,500,000 million → 2.5e12 absolute
        assert_eq!(p.market_cap, Some(2.5e12));
    }

    #[test]
    fn empty_object_is_default_profile() {
        let p = parse_profile(&json!({})).unwrap();
        assert!(p.industry.is_none());
        assert!(p.market_cap.is_none());
    }

    #[test]
    fn error_object_surfaces() {
        let err = parse_profile(&json!({"error": "Access denied."})).unwrap_err();
        assert!(err.contains("Access denied"), "{err}");
    }

    #[test]
    fn profile_url_shape() {
        let u = profile_url("AAPL", "TOK");
        assert!(u.contains("/stock/profile2?symbol=AAPL"));
        assert!(u.contains("token=TOK"));
    }

    #[test]
    fn encode_decode_industry_roundtrip() {
        use pomelo_data::LocalSource;

        let mut map = BTreeMap::new();
        map.insert("AAPL".into(), ("Technology".into(), Some(2.5e12)));
        map.insert("XOM".into(), ("Energy".into(), None));
        let bytes = encode_industry(&map).unwrap();
        let dir = std::env::temp_dir().join("pomelo_fh_industry_rt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tracked")).unwrap();
        std::fs::write(dir.join("tracked/universe.csv.gz"), &bytes).unwrap();
        let loaded = load_existing_industry(&LocalSource::new(&dir));
        assert_eq!(
            loaded.get("AAPL").map(|(s, _)| s.as_str()),
            Some("Technology")
        );
        assert_eq!(loaded.get("XOM").map(|(s, _)| s.as_str()), Some("Energy"));
    }
}
