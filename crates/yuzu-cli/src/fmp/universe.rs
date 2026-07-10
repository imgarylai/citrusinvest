//! Universe discovery and symbol-list filters.

use super::config::SyncConfig;
use super::http::Fetcher;
use super::industry::flag;
use super::util::num;
use super::HttpClient;
use super::FMP_BASE;

/// The default exchange filter — the three US major exchanges. (AMEX is now
/// NYSE American, but FMP still labels it `AMEX`.)
pub const US_EXCHANGES: &str = "NASDAQ,NYSE,AMEX";

/// Filters for [`build_symbol_list`] — a screened market universe.
#[derive(Default)]
pub struct SymbolFilter {
    /// Only symbols at/above this company market cap (`0.0` = no floor).
    pub min_market_cap: f64,
    /// Restrict to one or more exchanges (comma-separated FMP codes, e.g.
    /// [`US_EXCHANGES`]). `None`, an empty string, or `"all"` = every exchange.
    pub exchange: Option<String>,
    /// Keep ETFs / funds (default: stocks only).
    pub include_etf: bool,
    /// Cap the number of returned symbols (`None` = the API default).
    pub limit: Option<usize>,
}

/// Build a screened symbol universe from FMP's screener
/// (`/stable/company-screener`) — the "establish the sync list first" step so a
/// whole-market backtest has a persisted, reviewable symbol list to sync. The
/// filters are pushed to the API *and* re-applied client-side as a safety net.
/// Returns tickers, sorted and de-duplicated.
pub fn build_symbol_list<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
    filter: &SymbolFilter,
) -> Result<Vec<String>, String> {
    let fetcher = Fetcher::new(http, cfg);
    let mut url = format!("{FMP_BASE}/stable/company-screener?apikey={api_key}");
    if filter.min_market_cap > 0.0 {
        url.push_str(&format!(
            "&marketCapMoreThan={}",
            filter.min_market_cap as u64
        ));
    }
    if !filter.include_etf {
        url.push_str("&isEtf=false&isFund=false");
    }
    if let Some(ex) = &filter.exchange {
        let ex = ex.trim();
        // Empty / "all" is the escape hatch for every exchange (no filter).
        if !ex.is_empty() && !ex.eq_ignore_ascii_case("all") {
            url.push_str(&format!("&exchange={ex}"));
        }
    }
    if let Some(n) = filter.limit {
        url.push_str(&format!("&limit={n}"));
    }
    let rows = fetcher.get_rows(&url)?;
    let mut syms: Vec<String> = rows
        .iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            // Re-apply the screen client-side in case the API ignores a param.
            if !filter.include_etf && (flag(obj, "isEtf") || flag(obj, "isFund")) {
                return None;
            }
            if filter.min_market_cap > 0.0 {
                if let Some(mc) = num(obj, &["marketCap", "marketCapitalization"]) {
                    if mc < filter.min_market_cap {
                        return None;
                    }
                }
            }
            obj.get("symbol")?
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect();
    syms.sort();
    syms.dedup();
    Ok(syms)
}

/// Parse a market-cap threshold with an optional magnitude suffix — `k`, `m`,
/// `b`, `t` (thousand / million / billion / trillion), case-insensitive. Plain
/// numbers and scientific notation pass through. Examples: `1b` → 1e9,
/// `500m` → 5e8, `2.5t` → 2.5e12, `1e9` → 1e9, `0` → 0.
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
    // Strip the suffix only when one matched (ASCII, so 1-byte).
    let digits = if mult == 1.0 { s } else { &s[..s.len() - 1] };
    let val: f64 = digits
        .trim()
        .parse()
        .map_err(|_| format!("invalid market cap '{s}' (try 1b, 500m, or a plain number)"))?;
    if val < 0.0 || !val.is_finite() {
        return Err(format!("market cap must be a non-negative number: '{s}'"));
    }
    Ok(val * mult)
}

/// Parse a symbols-list file into tickers. One ticker per line; the first
/// comma-separated field is taken (so a `symbol,...` CSV works), and blank
/// lines, `#` comments, and a literal `symbol` header are skipped.
pub fn parse_symbols_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let first = line.split(',').next()?.trim();
            if first.is_empty() || first.eq_ignore_ascii_case("symbol") {
                return None;
            }
            Some(first.to_string())
        })
        .collect()
}
