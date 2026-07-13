//! Universe discovery via Alpha Vantage `LISTING_STATUS` (active listings).
//!
//! AV has **no** FMP/EODHD-style market-cap screener. The honest universe helper
//! is the full active listing CSV, optionally filtered by exchange / asset type.
//! Index **membership** PIT is not available from AV (#207 / #217) — do not
//! invent `panels/in_sp500` here.

use super::config::SyncConfig;
use super::delisted::{listing_status_url, parse_listing_status_csv, DelistedSymbol};
use super::http::Fetcher;
use super::HttpClient;

/// Filters for [`build_symbol_list`].
#[derive(Default, Clone)]
pub struct SymbolFilter {
    /// Exchange column match (e.g. `NYSE`, `NASDAQ`). Empty or `all` = no filter.
    pub exchange: String,
    /// Asset type match (e.g. `Stock`). Empty = any type.
    pub asset_type: String,
    /// Cap results after filter + sort.
    pub limit: Option<usize>,
}

/// Fetch active listings and apply filters → layout tickers.
///
/// Uses `LISTING_STATUS&state=active` (CSV). This is **not** a cap-sorted
/// screener; large unfiltered runs return the whole exchange universe.
pub fn build_symbol_list<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
    filter: &SymbolFilter,
) -> Result<Vec<String>, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    let fetcher = Fetcher::new(http, cfg);
    let body = fetcher.get(&listing_status_url(api_key, "active"))?;
    let text = String::from_utf8_lossy(&body);
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
    let rows = parse_listing_status_csv(&text)?;
    Ok(filter_listing_rows(&rows, filter))
}

/// Apply exchange / asset-type / limit filters to listing rows.
pub(crate) fn filter_listing_rows(rows: &[DelistedSymbol], filter: &SymbolFilter) -> Vec<String> {
    let ex_filter = filter.exchange.trim();
    let ex_all = ex_filter.is_empty() || ex_filter.eq_ignore_ascii_case("all");
    let type_filter = filter.asset_type.trim();
    let type_all = type_filter.is_empty() || type_filter.eq_ignore_ascii_case("all");

    let mut out: Vec<String> = rows
        .iter()
        .filter(|r| {
            if !ex_all && !r.exchange.eq_ignore_ascii_case(ex_filter) {
                return false;
            }
            if !type_all && !r.asset_type.eq_ignore_ascii_case(type_filter) {
                return false;
            }
            true
        })
        .map(|r| r.symbol.clone())
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
    use std::time::Duration;

    #[test]
    fn filter_by_exchange_and_type() {
        let rows = vec![
            DelistedSymbol {
                symbol: "AAPL".into(),
                exchange: "NASDAQ".into(),
                asset_type: "Stock".into(),
            },
            DelistedSymbol {
                symbol: "SPY".into(),
                exchange: "NYSE ARCA".into(),
                asset_type: "ETF".into(),
            },
            DelistedSymbol {
                symbol: "IBM".into(),
                exchange: "NYSE".into(),
                asset_type: "Stock".into(),
            },
        ];
        let f = SymbolFilter {
            exchange: "NASDAQ".into(),
            asset_type: "Stock".into(),
            limit: None,
        };
        assert_eq!(filter_listing_rows(&rows, &f), vec!["AAPL".to_string()]);

        let f2 = SymbolFilter {
            exchange: "all".into(),
            asset_type: "Stock".into(),
            limit: Some(1),
        };
        // sorted: AAPL, IBM → limit 1
        assert_eq!(filter_listing_rows(&rows, &f2), vec!["AAPL".to_string()]);
    }

    struct OkHttp(Vec<u8>);
    impl HttpClient for OkHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn build_symbol_list_from_active_csv() {
        let csv = b"symbol,name,exchange,assetType,ipoDate,delistingDate,status\n\
AAPL,Apple,NASDAQ,Stock,1980-12-12,,Active\n\
IBM,IBM,NYSE,Stock,1915-01-01,,Active\n\
SPY,SPDR,NYSE ARCA,ETF,1993-01-01,,Active\n";
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let filter = SymbolFilter {
            exchange: "all".into(),
            asset_type: "Stock".into(),
            limit: None,
        };
        let out = build_symbol_list(&OkHttp(csv.to_vec()), "tok", &cfg, &filter).unwrap();
        assert_eq!(out, vec!["AAPL".to_string(), "IBM".to_string()]);
    }

    #[test]
    fn build_symbol_list_rejects_empty_key() {
        let cfg = SyncConfig::default();
        let f = SymbolFilter::default();
        assert!(build_symbol_list(&OkHttp(vec![]), "", &cfg, &f).is_err());
    }

    #[test]
    fn build_symbol_list_note_envelope() {
        let body = br#"{"Note":"rate limited"}"#;
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let err = build_symbol_list(
            &OkHttp(body.to_vec()),
            "tok",
            &cfg,
            &SymbolFilter::default(),
        )
        .unwrap_err();
        assert!(err.contains("Note"), "{err}");
    }
}
