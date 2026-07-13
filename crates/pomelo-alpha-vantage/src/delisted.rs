//! Delisted-universe discovery via Alpha Vantage `LISTING_STATUS`.

use super::config::SyncConfig;
use super::http::Fetcher;
use super::symbol::layout_symbol;
use super::HttpClient;
use super::ALPHA_VANTAGE_BASE;

/// A delisted security from `LISTING_STATUS&state=delisted` (CSV).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelistedSymbol {
    /// Layout ticker, e.g. `FOO`.
    pub symbol: String,
    /// Exchange column from the feed when present.
    pub exchange: String,
    /// `assetType` column when present.
    pub asset_type: String,
}

pub(crate) fn listing_status_url(api_key: &str, state: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=LISTING_STATUS&state={state}&apikey={api_key}")
}

/// Parse LISTING_STATUS CSV body into delisted layout symbols.
///
/// Header (typical):
/// `symbol,name,exchange,assetType,ipoDate,delistingDate,status`
pub(crate) fn parse_listing_status_csv(text: &str) -> Result<Vec<DelistedSymbol>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| "LISTING_STATUS CSV is empty".to_string())?;
    // Detect error JSON/text masquerading as CSV.
    if header.contains("Error Message")
        || header.contains("\"Note\"")
        || header.starts_with('{')
        || header.contains("Invalid API")
    {
        return Err(format!(
            "Alpha Vantage LISTING_STATUS error: {}",
            header.trim()
        ));
    }

    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    let idx = |name: &str| {
        cols.iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("LISTING_STATUS missing column {name}"))
    };
    let i_sym = idx("symbol")?;
    let i_ex = idx("exchange").ok();
    let i_type = idx("assetType").or_else(|_| idx("asset_type")).ok();

    let mut out = Vec::new();
    for line in lines {
        // Simple CSV split (AV fields rarely embed commas in symbol/exchange).
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() <= i_sym {
            continue;
        }
        let raw = parts[i_sym];
        let Some(symbol) = layout_symbol(raw).or_else(|| {
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_ascii_uppercase())
            }
        }) else {
            continue;
        };
        let exchange = i_ex
            .and_then(|i| parts.get(i).copied())
            .unwrap_or("")
            .to_ascii_uppercase();
        let asset_type = i_type
            .and_then(|i| parts.get(i).copied())
            .unwrap_or("")
            .to_string();
        out.push(DelistedSymbol {
            symbol,
            exchange,
            asset_type,
        });
    }
    Ok(out)
}

/// Fetch delisted tickers via `LISTING_STATUS&state=delisted`.
pub fn fetch_delisted<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
) -> Result<Vec<DelistedSymbol>, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    let fetcher = Fetcher::new(http, cfg);
    let body = fetcher.get(&listing_status_url(api_key, "delisted"))?;
    let text = String::from_utf8_lossy(&body);
    // JSON error envelopes
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(obj) = v.as_object() {
                for err_key in ["Error Message", "Information", "Note"] {
                    if let Some(msg) = obj.get(err_key).and_then(|x| x.as_str()) {
                        return Err(format!("Alpha Vantage {err_key}: {msg}"));
                    }
                }
            }
        }
    }
    let mut out = parse_listing_status_csv(&text)?;
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out.dedup_by(|a, b| a.symbol == b.symbol);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use std::time::Duration;

    #[test]
    fn parse_listing_csv_basic() {
        let csv = "\
symbol,name,exchange,assetType,ipoDate,delistingDate,status
FOO,Foo Inc,NYSE,Stock,2000-01-01,2020-01-01,Delisted
bar,Bar LLC,NASDAQ,ETF,2010-01-01,2021-01-01,Delisted
,empty,NYSE,Stock,,,Delisted
";
        let out = parse_listing_status_csv(csv).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "FOO");
        assert_eq!(out[0].exchange, "NYSE");
        assert_eq!(out[1].symbol, "BAR");
        assert_eq!(out[1].asset_type, "ETF");
    }

    #[test]
    fn listing_status_url_shape() {
        let u = listing_status_url("TOK", "delisted");
        assert!(u.contains("function=LISTING_STATUS"));
        assert!(u.contains("state=delisted"));
        assert!(u.contains("apikey=TOK"));
    }

    struct OkHttp(Vec<u8>);
    impl HttpClient for OkHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn fetch_delisted_parses_csv() {
        let csv = b"symbol,name,exchange,assetType,ipoDate,delistingDate,status\n\
ZZZ,Z Co,NYSE,Stock,2000-01-01,2020-01-01,Delisted\n";
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let out = fetch_delisted(&OkHttp(csv.to_vec()), "tok", &cfg).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbol, "ZZZ");
    }

    #[test]
    fn fetch_delisted_rejects_empty_key() {
        let cfg = SyncConfig::default();
        assert!(fetch_delisted(&OkHttp(vec![]), "", &cfg).is_err());
    }

    #[test]
    fn parse_listing_rejects_json_error_header() {
        let err = parse_listing_status_csv(r#"{"Error Message":"bad key"}"#).unwrap_err();
        assert!(err.contains("error") || err.contains("Error"), "{err}");
    }

    #[test]
    fn fetch_delisted_json_note_envelope() {
        let body = br#"{"Note":"Thank you for using Alpha Vantage!"}"#;
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let err = fetch_delisted(&OkHttp(body.to_vec()), "tok", &cfg).unwrap_err();
        assert!(err.contains("Note"), "{err}");
    }

    #[test]
    fn parse_listing_dedupes_and_uppercases() {
        let csv = "\
symbol,name,exchange,assetType,ipoDate,delistingDate,status
foo,F,NYSE,Stock,,,Delisted
FOO,F2,NASDAQ,Stock,,,Delisted
";
        let out = parse_listing_status_csv(csv).unwrap();
        // sort+dedup happens in fetch; parse returns both then fetch dedupes
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "FOO");
        assert_eq!(out[1].symbol, "FOO");
    }
}
