//! Layout tickers (`AAPL`) ↔ EODHD codes (`AAPL.US`).

/// Normalize a user/layout ticker or EODHD code into `(layout_symbol, eodhd_code)`.
///
/// - `AAPL` + default exchange `US` → (`AAPL`, `AAPL.US`)
/// - `AAPL.US` → (`AAPL`, `AAPL.US`)
/// - `VOD.LSE` → (`VOD`, `VOD.LSE`)
///
/// Empty / whitespace-only input → `None`.
pub fn split_symbol(raw: &str, default_exchange: &str) -> Option<(String, String)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // EODHD uses a single `.` between code and exchange (exchange codes are
    // alphanumeric, e.g. US, LSE, TO). Prefer the last `.` so we don't break
    // exotic codes that embed dots (rare); still covers AAPL.US / BRK-B.US.
    if let Some((code, ex)) = s.rsplit_once('.') {
        let code = code.trim();
        let ex = ex.trim();
        if code.is_empty() || ex.is_empty() {
            return None;
        }
        let layout = code.to_ascii_uppercase();
        let eodhd = format!("{layout}.{}", ex.to_ascii_uppercase());
        return Some((layout, eodhd));
    }
    let layout = s.to_ascii_uppercase();
    let ex = default_exchange.trim();
    if ex.is_empty() {
        return None;
    }
    let eodhd = format!("{layout}.{}", ex.to_ascii_uppercase());
    Some((layout, eodhd))
}

/// Layout symbol only (strip exchange suffix if present).
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
            // Allow `AAPL.US` in files; store layout form.
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
    fn bare_ticker_gets_default_exchange() {
        assert_eq!(
            split_symbol("aapl", "US"),
            Some(("AAPL".into(), "AAPL.US".into()))
        );
        assert_eq!(
            split_symbol("VOD", "LSE"),
            Some(("VOD".into(), "VOD.LSE".into()))
        );
    }

    #[test]
    fn eodhd_code_splits() {
        assert_eq!(
            split_symbol("AAPL.US", "TO"),
            Some(("AAPL".into(), "AAPL.US".into()))
        );
        assert_eq!(
            split_symbol("vod.lse", "US"),
            Some(("VOD".into(), "VOD.LSE".into()))
        );
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(split_symbol("  ", "US"), None);
        assert_eq!(split_symbol(".", "US"), None);
        assert_eq!(split_symbol("AAPL.", "US"), None);
        assert_eq!(split_symbol(".US", "US"), None);
    }

    #[test]
    fn parse_list_dedupes_and_strips() {
        let text = "# comment\nAAPL, msft\nAAPL.US\nGOOGL\n";
        assert_eq!(
            parse_symbols_list(text),
            vec!["AAPL".to_string(), "MSFT".to_string(), "GOOGL".to_string()]
        );
    }
}
