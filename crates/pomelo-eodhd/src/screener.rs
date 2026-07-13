//! Universe discovery via EODHD Screener API.

use super::config::SyncConfig;
use super::http::Fetcher;
use super::symbol::layout_symbol;
use super::HttpClient;
use super::EODHD_BASE;

/// Filters for [`build_symbol_list`].
#[derive(Default, Clone)]
pub struct SymbolFilter {
    /// Only symbols at/above this company market cap (`0.0` = no floor), USD.
    pub min_market_cap: f64,
    /// Exchange code for the screener (`us`, `NYSE`, …). Empty = no exchange filter.
    pub exchange: String,
    /// Cap results (`None` = API default 50; max 100 per request — we page).
    pub limit: Option<usize>,
}

/// Build a screened symbol universe from EODHD's screener.
///
/// Pages with `limit=100` until exhausted or `filter.limit` is reached.
pub fn build_symbol_list<H: HttpClient>(
    http: &H,
    api_token: &str,
    cfg: &SyncConfig,
    filter: &SymbolFilter,
) -> Result<Vec<String>, String> {
    if api_token.trim().is_empty() {
        return Err("empty API token".to_string());
    }
    let fetcher = Fetcher::new(http, cfg);
    let page_size = 100u32;
    let hard_cap = filter.limit.unwrap_or(10_000);
    let mut out = Vec::new();
    let mut offset = 0u32;

    while out.len() < hard_cap {
        let mut filters: Vec<String> = Vec::new();
        if filter.min_market_cap > 0.0 {
            // EODHD filter JSON: ["market_capitalization",">",N]
            filters.push(format!(
                "[\"market_capitalization\",\">\",{}]",
                filter.min_market_cap as u64
            ));
        }
        let ex = filter.exchange.trim();
        if !ex.is_empty() && !ex.eq_ignore_ascii_case("all") {
            filters.push(format!(
                "[\"exchange\",\"=\",\"{}\"]",
                ex.to_ascii_lowercase()
            ));
        }
        let filters_param = if filters.is_empty() {
            String::new()
        } else {
            format!("&filters=[{}]", filters.join(","))
        };
        let url = format!(
            "{EODHD_BASE}/screener?api_token={api_token}&sort=market_capitalization.desc\
             &limit={page_size}&offset={offset}{filters_param}"
        );
        let rows = fetcher.get_rows(&url)?;
        if rows.is_empty() {
            break;
        }
        let before = out.len();
        for r in &rows {
            let Some(obj) = r.as_object() else { continue };
            let code = obj
                .get("code")
                .or_else(|| obj.get("Code"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(code) = code else { continue };
            if let Some(layout) = layout_symbol(code).or_else(|| {
                if code.contains('.') {
                    None
                } else {
                    Some(code.to_ascii_uppercase())
                }
            }) {
                out.push(layout);
            }
            if out.len() >= hard_cap {
                break;
            }
        }
        if out.len() == before {
            break; // no progress
        }
        if rows.len() < page_size as usize {
            break;
        }
        offset = offset.saturating_add(page_size);
        if offset > 999 {
            break; // API offset max
        }
    }

    out.sort();
    out.dedup();
    if let Some(n) = filter.limit {
        out.truncate(n);
    }
    Ok(out)
}

/// Parse market-cap threshold with optional `k`/`m`/`b`/`t` suffix.
pub fn parse_market_cap(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty market-cap value".to_string());
    }
    let mult = match s.chars().last().unwrap().to_ascii_lowercase() {
        'k' => 1e3,
        'm' => 1e6,
        'b' => 1e9,
        't' => 1e12,
        _ => 1.0,
    };
    let num = if mult == 1.0 { s } else { &s[..s.len() - 1] };
    let v: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid market-cap value '{s}'"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("invalid market-cap value '{s}'"));
    }
    Ok(v * mult)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpError;
    use std::cell::RefCell;
    use std::time::Duration;

    struct MockHttp {
        body: RefCell<Vec<u8>>,
    }

    impl HttpClient for MockHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.body.borrow().clone())
        }
    }

    #[test]
    fn parse_market_cap_suffixes() {
        assert_eq!(parse_market_cap("1b").unwrap(), 1e9);
        assert_eq!(parse_market_cap("500m").unwrap(), 5e8);
        assert_eq!(parse_market_cap("0").unwrap(), 0.0);
        assert!(parse_market_cap("").is_err());
    }

    #[test]
    fn build_symbol_list_from_screener_json() {
        let body = br#"[
            {"code":"aapl","name":"Apple","exchange":"NASDAQ","market_capitalization":3e12},
            {"code":"MSFT","name":"Microsoft","exchange":"NASDAQ","market_capitalization":2e12}
        ]"#;
        let http = MockHttp {
            body: RefCell::new(body.to_vec()),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let filter = SymbolFilter {
            min_market_cap: 1e9,
            exchange: "us".into(),
            limit: Some(10),
        };
        let syms = build_symbol_list(&http, "tok", &cfg, &filter).unwrap();
        assert_eq!(syms, vec!["AAPL".to_string(), "MSFT".to_string()]);
    }

    #[test]
    fn build_symbol_list_rejects_empty_token_and_truncates() {
        let body = br#"[
            {"code":"AAA"},{"code":"BBB"},{"code":"CCC"}
        ]"#;
        let http = MockHttp {
            body: RefCell::new(body.to_vec()),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        assert!(build_symbol_list(&http, "", &cfg, &SymbolFilter::default()).is_err());
        let filter = SymbolFilter {
            min_market_cap: 0.0,
            exchange: "all".into(),
            limit: Some(2),
        };
        let syms = build_symbol_list(&http, "tok", &cfg, &filter).unwrap();
        assert_eq!(syms.len(), 2);
    }

    #[test]
    fn build_symbol_list_empty_page() {
        let http = MockHttp {
            body: RefCell::new(b"[]".to_vec()),
        };
        let cfg = SyncConfig {
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        };
        let syms = build_symbol_list(&http, "tok", &cfg, &SymbolFilter::default()).unwrap();
        assert!(syms.is_empty());
    }
}
