//! Universe discovery via Finnhub `/stock/symbol` (exchange listing).
//!
//! Finnhub's `/stock/symbol?exchange=US` returns every symbol on an exchange
//! with a security `type` (`Common Stock`, `ETP`, …). This is the honest
//! universe helper — **not** a market-cap screener (Finnhub's cap screener is
//! plan-gated and its filter surface differs; see `docs/data-sources.md` §
//! Finnhub). Index **membership** PIT lives in [`crate::index`], not here.

use serde_json::Value;

use super::config::SyncConfig;
use super::http::Fetcher;
use super::symbol::layout_symbol;
use super::HttpClient;
use super::FINNHUB_BASE;

/// Filters for [`build_symbol_list`].
#[derive(Default, Clone)]
pub struct SymbolFilter {
    /// Security `type` match (e.g. `Common Stock`). Empty or `all` = any type.
    pub security_type: String,
    /// Cap results after filter + sort.
    pub limit: Option<usize>,
}

/// Fetch an exchange's listing and apply filters → layout tickers.
///
/// `exchange` is Finnhub's exchange/country code (e.g. `US`). Large unfiltered
/// runs return the whole exchange universe.
pub fn build_symbol_list<H: HttpClient>(
    http: &H,
    api_key: &str,
    exchange: &str,
    cfg: &SyncConfig,
    filter: &SymbolFilter,
) -> Result<Vec<String>, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    let ex = exchange.trim();
    if ex.is_empty() {
        return Err("empty exchange".to_string());
    }
    let fetcher = Fetcher::new(http, cfg);
    let value = fetcher.get_json(&format!(
        "{FINNHUB_BASE}/stock/symbol?exchange={ex}&token={api_key}"
    ))?;
    // Finnhub returns an array on success; an object usually signals an error.
    if let Some(err) = value
        .as_object()
        .and_then(|o| o.get("error"))
        .and_then(Value::as_str)
    {
        return Err(format!("Finnhub error: {err}"));
    }
    let rows = value
        .as_array()
        .ok_or_else(|| "Finnhub /stock/symbol did not return a list".to_string())?;
    Ok(filter_symbol_rows(rows, filter))
}

/// Apply the security-type and limit filters to `/stock/symbol` rows.
pub(crate) fn filter_symbol_rows(rows: &[Value], filter: &SymbolFilter) -> Vec<String> {
    let type_filter = filter.security_type.trim();
    let type_all = type_filter.is_empty() || type_filter.eq_ignore_ascii_case("all");

    let mut out: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            if !type_all {
                let ty = obj.get("type").and_then(Value::as_str).unwrap_or("");
                if !ty.eq_ignore_ascii_case(type_filter) {
                    return None;
                }
            }
            let raw = obj.get("symbol").and_then(Value::as_str)?;
            layout_symbol(raw)
        })
        .collect();
    out.sort();
    out.dedup();
    if let Some(n) = filter.limit {
        out.truncate(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use serde_json::json;
    use std::time::Duration;

    fn rows() -> Value {
        json!([
            {"symbol": "AAPL", "displaySymbol": "AAPL", "type": "Common Stock", "mic": "XNAS"},
            {"symbol": "SPY", "displaySymbol": "SPY", "type": "ETP", "mic": "ARCX"},
            {"symbol": "ibm", "displaySymbol": "IBM", "type": "Common Stock", "mic": "XNYS"},
            {"displaySymbol": "NOSYM", "type": "Common Stock"}
        ])
    }

    #[test]
    fn filter_by_type_and_limit() {
        let f = SymbolFilter {
            security_type: "Common Stock".into(),
            limit: None,
        };
        assert_eq!(
            filter_symbol_rows(rows().as_array().unwrap(), &f),
            vec!["AAPL".to_string(), "IBM".to_string()]
        );

        let f_all = SymbolFilter {
            security_type: "all".into(),
            limit: Some(2),
        };
        // sorted: AAPL, IBM, SPY → limit 2
        assert_eq!(
            filter_symbol_rows(rows().as_array().unwrap(), &f_all),
            vec!["AAPL".to_string(), "IBM".to_string()]
        );
    }

    struct OkHttp(Vec<u8>);
    impl HttpClient for OkHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.0.clone())
        }
    }

    fn cfg() -> SyncConfig {
        SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    #[test]
    fn build_symbol_list_from_array() {
        let body = serde_json::to_vec(&rows()).unwrap();
        let filter = SymbolFilter {
            security_type: "Common Stock".into(),
            limit: None,
        };
        let cfg = cfg();
        let out = build_symbol_list(&OkHttp(body), "tok", "US", &cfg, &filter).unwrap();
        assert_eq!(out, vec!["AAPL".to_string(), "IBM".to_string()]);
    }

    #[test]
    fn build_symbol_list_rejects_empty_key_and_exchange() {
        let cfg = cfg();
        let f = SymbolFilter::default();
        assert!(build_symbol_list(&OkHttp(b"[]".to_vec()), "", "US", &cfg, &f).is_err());
        assert!(build_symbol_list(&OkHttp(b"[]".to_vec()), "tok", "  ", &cfg, &f).is_err());
    }

    #[test]
    fn build_symbol_list_error_object() {
        let cfg = cfg();
        let f = SymbolFilter::default();
        let err = build_symbol_list(
            &OkHttp(br#"{"error":"access denied"}"#.to_vec()),
            "tok",
            "US",
            &cfg,
            &f,
        )
        .unwrap_err();
        assert!(err.contains("access denied"), "{err}");
    }

    #[test]
    fn build_symbol_list_non_array_errors() {
        let cfg = cfg();
        let f = SymbolFilter::default();
        assert!(build_symbol_list(&OkHttp(b"42".to_vec()), "tok", "US", &cfg, &f).is_err());
    }
}
