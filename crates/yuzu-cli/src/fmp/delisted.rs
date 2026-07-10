//! Delisted-universe discovery for survivorship-honest syncs (#124 / #26).
//!
//! Actively-trading screeners (`company-screener`) only see names that still
//! trade, so a sync built from them is survivors-only — every backtest over it
//! carries textbook survivorship bias at the data layer, where the engine can't
//! see it. FMP's `delisted-companies` endpoint lists securities that stopped
//! trading; unioning those tickers into the sync universe lets their
//! `prices/{SYM}.csv.gz` files land and simply **end at the delisting date**.
//! The engine's NaN-streak detection (`delist_after`, done since #14) does the
//! rest — no engine change, no active truncation here.

use std::collections::HashSet;

use serde_json::Value;

use super::config::SyncConfig;
use super::http::Fetcher;
use super::util::iso_to_i32;
use super::HttpClient;
use super::FMP_BASE;

/// A delisted security from FMP's `delisted-companies` endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelistedSymbol {
    pub symbol: String,
    /// FMP exchange label (`NASDAQ`, `NYSE`, `AMEX`, `OTC`, …); may be empty.
    pub exchange: String,
    /// Delisting date, packed `YYYYMMDD`. `None` when FMP omitted or garbled it —
    /// the symbol is still usable (its price file ends wherever the feed stops).
    pub delisted_date: Option<i32>,
}

/// Cap on pages walked from `delisted-companies` (each page ≈ 100 rows). A
/// safety bound so a misbehaving feed can't spin forever; a hit is logged and
/// surfaced as truncation, never a silent stop.
pub(crate) const MAX_DELISTED_PAGES: u32 = 200;

pub(crate) fn delisted_url(page: u32, key: &str) -> String {
    format!("{FMP_BASE}/stable/delisted-companies?page={page}&apikey={key}")
}

pub(crate) fn parse_delisted_rows(rows: &[Value]) -> Vec<DelistedSymbol> {
    rows.iter()
        .filter_map(|r| {
            let obj = r.as_object()?;
            let symbol = obj
                .get("symbol")?
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            let exchange = obj
                .get("exchange")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let delisted_date = obj
                .get("delistedDate")
                .and_then(Value::as_str)
                .and_then(iso_to_i32);
            Some(DelistedSymbol {
                symbol,
                exchange,
                delisted_date,
            })
        })
        .collect()
}

/// Parse a comma-separated exchange filter into an uppercase set. `None`, an
/// empty string, or `"all"` means "keep every exchange" (returns `None`).
pub(crate) fn exchange_filter(spec: Option<&str>) -> Option<HashSet<String>> {
    let spec = spec?.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return None;
    }
    let set: HashSet<String> = spec
        .split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

pub(crate) fn keep_exchange(wanted: &Option<HashSet<String>>, exchange: &str) -> bool {
    match wanted {
        None => true,
        Some(set) => set.contains(&exchange.trim().to_ascii_uppercase()),
    }
}

/// Fetch the delisted universe, paging `delisted-companies` until an empty page
/// or [`MAX_DELISTED_PAGES`]. `exchanges` filters client-side (comma-separated
/// FMP codes; `None` / empty / `"all"` keeps every exchange — note that
/// `delisted-companies` carries no market cap, so a `--min-market-cap` floor
/// cannot apply to these names). Returns rows sorted and de-duplicated by symbol.
pub fn fetch_delisted<H: HttpClient>(
    http: &H,
    api_key: &str,
    cfg: &SyncConfig,
    exchanges: Option<&str>,
) -> Result<Vec<DelistedSymbol>, String> {
    let fetcher = Fetcher::new(http, cfg);
    let wanted = exchange_filter(exchanges);
    let mut out: Vec<DelistedSymbol> = Vec::new();
    for page in 0..MAX_DELISTED_PAGES {
        let rows = fetcher.get_rows(&delisted_url(page, api_key))?;
        if rows.is_empty() {
            break;
        }
        for d in parse_delisted_rows(&rows) {
            if keep_exchange(&wanted, &d.exchange) {
                out.push(d);
            }
        }
        if page == MAX_DELISTED_PAGES - 1 {
            eprintln!(
                "delisted: hit page cap ({MAX_DELISTED_PAGES}); list may be truncated (older names dropped)"
            );
        }
    }
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out.dedup_by(|a, b| a.symbol == b.symbol);
    Ok(out)
}
