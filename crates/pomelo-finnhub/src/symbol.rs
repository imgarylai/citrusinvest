//! Layout tickers (`AAPL`) ↔ Finnhub symbols.
//!
//! Finnhub US equities are typically bare (`AAPL`). Some international codes
//! use exchange suffixes (e.g. `TSCO.L`) — layout key is the code before the
//! last `.` when present.

/// Normalize a user ticker into `(layout_symbol, finnhub_symbol)`.
///
/// - `AAPL` → (`AAPL`, `AAPL`)
/// - `TSCO.L` → (`TSCO`, `TSCO.L`)
/// - Empty → `None`
pub fn split_symbol(raw: &str, _default_exchange: &str) -> Option<(String, String)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some((code, ex)) = s.rsplit_once('.') {
        let code = code.trim();
        let ex = ex.trim();
        if code.is_empty() || ex.is_empty() {
            return None;
        }
        let layout = code.to_ascii_uppercase();
        // Finnhub often uses short exchange suffixes (L, TO, …) — preserve as given case for API
        let fh = format!("{layout}.{}", ex.to_ascii_uppercase());
        return Some((layout, fh));
    }
    let layout = s.to_ascii_uppercase();
    Some((layout.clone(), layout))
}

/// Layout symbol only.
pub fn layout_symbol(raw: &str) -> Option<String> {
    split_symbol(raw, "US").map(|(layout, _)| layout)
}

/// Parse a comma-separated list or whitespace/newline-separated file body into
/// unique layout tickers (order preserved, first wins). `#` starts a comment
/// to end of line.
pub fn parse_symbols_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        for part in line.split(|c: char| c == ',' || c.is_whitespace()) {
            let t = part.trim();
            if t.is_empty() {
                continue;
            }
            let Some(layout) = layout_symbol(t) else {
                continue;
            };
            if seen.insert(layout.clone()) {
                out.push(layout);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_ticker_stays_bare() {
        assert_eq!(
            split_symbol("aapl", "US"),
            Some(("AAPL".into(), "AAPL".into()))
        );
    }

    #[test]
    fn dotted_symbol_splits_layout() {
        assert_eq!(
            split_symbol("TSCO.L", "US"),
            Some(("TSCO".into(), "TSCO.L".into()))
        );
    }

    #[test]
    fn rejects_empty() {
        assert!(split_symbol("", "US").is_none());
        assert!(split_symbol("  ", "US").is_none());
        assert!(split_symbol(".", "US").is_none());
    }

    #[test]
    fn parse_list_dedupes_and_strips() {
        let text = "AAPL, msft\n# comment\naapl\nTSCO.L\n";
        assert_eq!(
            parse_symbols_list(text),
            vec!["AAPL".to_string(), "MSFT".to_string(), "TSCO".to_string()]
        );
    }
}
