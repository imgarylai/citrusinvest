//! Build the `symbol -> sector` map the yuzu-core neutralization ops need,
//! from the `tracked/*.csv.gz` snapshot (`symbol,sector,market_cap`).

use std::collections::HashMap;

/// Parse a `symbol,sector,...` CSV into `symbol -> sector`. A leading header row
/// (`symbol,...`) is skipped; rows with an empty sector are dropped.
pub fn parse_industry_csv(csv: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in csv.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split(',');
        let (Some(sym), Some(sector)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (sym, sector) = (sym.trim(), sector.trim());
        if sym.is_empty() || sector.is_empty() || sym == "symbol" {
            continue;
        }
        out.insert(sym.to_string(), sector.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_symbol_sector_skipping_header_and_blanks() {
        let csv = "symbol,sector,market_cap\nNVDA,Technology,5103000000000\nXOM,Energy,470000000000\nUNKNOWN,,1000\n";
        let m = parse_industry_csv(csv);
        assert_eq!(m.get("NVDA").map(String::as_str), Some("Technology"));
        assert_eq!(m.get("XOM").map(String::as_str), Some("Energy"));
        assert!(!m.contains_key("UNKNOWN")); // empty sector dropped
        assert!(!m.contains_key("symbol")); // header skipped
        assert_eq!(m.len(), 2);
    }
}
