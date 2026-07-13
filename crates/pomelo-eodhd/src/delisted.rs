//! Delisted-universe discovery via EODHD exchange-symbol-list.

use serde_json::Value;

use super::config::SyncConfig;
use super::http::Fetcher;
use super::symbol::layout_symbol;
use super::HttpClient;
use super::EODHD_BASE;

/// A delisted security from `exchange-symbol-list?delisted=1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelistedSymbol {
    /// Layout ticker (exchange suffix stripped), e.g. `AAPL`.
    pub symbol: String,
    /// EODHD exchange code (`US`, `LSE`, …).
    pub exchange: String,
    /// Security type from the feed when present (`Common Stock`, …).
    pub security_type: String,
}

pub(crate) fn delisted_url(exchange: &str, api_token: &str) -> String {
    format!(
        "{EODHD_BASE}/exchange-symbol-list/{exchange}?api_token={api_token}&fmt=json&delisted=1"
    )
}

pub(crate) fn parse_delisted_rows(rows: &[Value], default_exchange: &str) -> Vec<DelistedSymbol> {
    rows.iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            let code = obj
                .get("Code")
                .or_else(|| obj.get("code"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let exchange = obj
                .get("Exchange")
                .or_else(|| obj.get("exchange"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(default_exchange)
                .to_ascii_uppercase();
            // Prefer layout form; Code is usually bare (AAPL) on this endpoint.
            let symbol = layout_symbol(code).or_else(|| {
                if code.contains('.') {
                    None
                } else {
                    Some(code.to_ascii_uppercase())
                }
            })?;
            let security_type = obj
                .get("Type")
                .or_else(|| obj.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            Some(DelistedSymbol {
                symbol,
                exchange,
                security_type,
            })
        })
        .collect()
}

/// Fetch delisted tickers for one exchange (e.g. `US`).
///
/// EODHD returns the full inactive list in one call (not paged like FMP).
pub fn fetch_delisted<H: HttpClient>(
    http: &H,
    api_token: &str,
    cfg: &SyncConfig,
    exchange: &str,
) -> Result<Vec<DelistedSymbol>, String> {
    if api_token.trim().is_empty() {
        return Err("empty API token".to_string());
    }
    let ex = exchange.trim();
    if ex.is_empty() {
        return Err("exchange is empty".to_string());
    }
    let fetcher = Fetcher::new(http, cfg);
    let rows = fetcher.get_rows(&delisted_url(ex, api_token))?;
    let mut out = parse_delisted_rows(&rows, ex);
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out.dedup_by(|a, b| a.symbol == b.symbol);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpError;
    use std::time::Duration;

    use serde_json::json;

    #[test]
    fn parse_delisted_rows_basic() {
        let rows = vec![
            json!({"Code":"FOO","Exchange":"US","Type":"Common Stock"}),
            json!({"Code":"bar","Exchange":"US","Type":"ETF"}),
            json!({"Code":"","Exchange":"US"}),
            json!({"code":"BAZ","exchange":"LSE"}),
        ];
        let out = parse_delisted_rows(&rows, "US");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].symbol, "FOO");
        assert_eq!(out[1].symbol, "BAR");
        assert_eq!(out[2].symbol, "BAZ");
        assert_eq!(out[2].exchange, "LSE");
    }

    #[test]
    fn delisted_url_shape() {
        let u = delisted_url("US", "TOK");
        assert!(u.contains("/exchange-symbol-list/US?"));
        assert!(u.contains("delisted=1"));
        assert!(u.contains("api_token=TOK"));
    }

    struct OkHttp(Vec<u8>);
    impl HttpClient for OkHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn fetch_delisted_parses_and_dedupes() {
        let body = br#"[
            {"Code":"AAA","Exchange":"US","Type":"Common Stock"},
            {"Code":"AAA","Exchange":"US","Type":"Common Stock"},
            {"Code":"BBB","Exchange":"US","Type":"Common Stock"}
        ]"#;
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let out = fetch_delisted(&OkHttp(body.to_vec()), "tok", &cfg, "US").unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "AAA");
        assert_eq!(out[1].symbol, "BBB");
    }

    #[test]
    fn fetch_delisted_rejects_empty_token() {
        let cfg = SyncConfig::default();
        assert!(fetch_delisted(&OkHttp(b"[]".to_vec()), "", &cfg, "US").is_err());
        assert!(fetch_delisted(&OkHttp(b"[]".to_vec()), "tok", &cfg, "").is_err());
    }
}
