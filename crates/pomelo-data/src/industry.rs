//! Build the `symbol -> sector` map the yuzu-core neutralization ops need,
//! from the `tracked/*.csv.gz` snapshot (`symbol,sector,market_cap`).

use std::collections::HashMap;

use crate::error::DataError;
use crate::format::read_csv_text;
use crate::source::ObjectSource;

/// Load `tracked/universe.csv.gz` (or `.csv`) from `src` into the
/// `symbol → sector` map the engine's industry ops consume. `Ok(None)` when
/// the snapshot is absent or has no usable rows — the runner then leaves the
/// map empty and industry ops treat every symbol as unmapped.
pub fn load_industry_map<S: ObjectSource>(
    src: &S,
) -> Result<Option<HashMap<String, String>>, DataError> {
    for ext in [".csv.gz", ".csv"] {
        if let Some(bytes) = src.get(&format!("tracked/universe{ext}"))? {
            let map = parse_industry_csv(&read_csv_text(&bytes)?);
            return Ok((!map.is_empty()).then_some(map));
        }
    }
    Ok(None)
}

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

    #[test]
    fn loads_the_tracked_snapshot_when_present() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = std::env::temp_dir().join("pomelo_data_industry_map");
        let _ = std::fs::remove_dir_all(&dir);
        let src = crate::LocalSource::new(&dir);
        // Absent tree → no map, no error.
        assert!(load_industry_map(&src).unwrap().is_none());

        std::fs::create_dir_all(dir.join("tracked")).unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"symbol,sector,market_cap\nAAA,Tech,1\nBBB,Energy,2\n")
            .unwrap();
        std::fs::write(dir.join("tracked/universe.csv.gz"), enc.finish().unwrap()).unwrap();
        let map = load_industry_map(&src).unwrap().unwrap();
        assert_eq!(map.get("AAA").map(String::as_str), Some("Tech"));
        assert_eq!(map.len(), 2);
    }
}
