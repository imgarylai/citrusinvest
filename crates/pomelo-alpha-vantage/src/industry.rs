//! Sector map from `OVERVIEW` → `tracked/universe.csv.gz`.

use std::collections::BTreeMap;

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::ObjectSource;
use serde_json::Value;

use super::http::Fetcher;
use super::HttpClient;
use super::ALPHA_VANTAGE_BASE;

/// Object key the industry snapshot is written under.
pub const INDUSTRY_KEY: &str = "tracked/universe.csv.gz";

/// Bits of Alpha Vantage `OVERVIEW` used for the industry map.
#[derive(Debug, Clone, Default)]
pub(crate) struct Profile {
    pub(crate) sector: Option<String>,
    pub(crate) industry: Option<String>,
    pub(crate) market_cap: Option<f64>,
}

pub(crate) fn overview_url(av_symbol: &str, api_key: &str) -> String {
    format!("{ALPHA_VANTAGE_BASE}?function=OVERVIEW&symbol={av_symbol}&apikey={api_key}")
}

fn str_field(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "None" && *s != "-")
        .map(str::to_string)
}

fn f64_field(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() || s == "None" || s == "-" {
                None
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

/// Parse an OVERVIEW JSON object (or error envelope).
pub(crate) fn parse_overview(value: &Value) -> Result<Profile, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "OVERVIEW payload is not a JSON object".to_string())?;
    for err_key in ["Error Message", "Information", "Note"] {
        if let Some(msg) = root.get(err_key).and_then(Value::as_str) {
            return Err(format!("Alpha Vantage {err_key}: {msg}"));
        }
    }
    // Empty object / missing Symbol = no data for this ticker.
    if root.get("Symbol").and_then(Value::as_str).is_none()
        && root.get("Name").and_then(Value::as_str).is_none()
    {
        return Ok(Profile::default());
    }
    Ok(Profile {
        sector: root.get("Sector").and_then(str_field),
        industry: root.get("Industry").and_then(str_field),
        market_cap: root.get("MarketCapitalization").and_then(f64_field),
    })
}

/// Fetch OVERVIEW for one AV symbol.
pub(crate) fn fetch_overview<H: HttpClient>(
    fetcher: &Fetcher<H>,
    av_symbol: &str,
    api_key: &str,
) -> Result<Option<Profile>, String> {
    let value = fetcher.get_json(&overview_url(av_symbol, api_key))?;
    let p = parse_overview(&value)?;
    if p.sector.is_none() && p.industry.is_none() && p.market_cap.is_none() {
        return Ok(None);
    }
    Ok(Some(p))
}

/// Read existing `tracked/universe.csv.gz` so resume does not drop sectors.
pub(crate) fn load_existing_industry(
    src: &impl ObjectSource,
) -> BTreeMap<String, (String, Option<f64>)> {
    let Some(bytes) = src.get(INDUSTRY_KEY).ok().flatten() else {
        return BTreeMap::new();
    };
    let text = decode_csv_text(&bytes);
    parse_industry_csv(&text)
        .into_iter()
        .map(|(sym, sector)| (sym, (sector, None)))
        .collect()
}

pub(crate) fn decode_csv_text(bytes: &[u8]) -> String {
    use std::io::Read;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = String::new();
        if flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .is_ok()
        {
            return out;
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Encode industry map as gzip CSV: `symbol,sector,market_cap`.
pub(crate) fn encode_industry(
    industry: &BTreeMap<String, (String, Option<f64>)>,
) -> Result<Vec<u8>, std::io::Error> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut csv = String::from("symbol,sector,market_cap\n");
    for (sym, (sector, mcap)) in industry {
        let mcap = mcap.map(|m| m.to_string()).unwrap_or_default();
        csv.push_str(&format!("{sym},{sector},{mcap}\n"));
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(csv.as_bytes())?;
    enc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_overview_fields() {
        let v = json!({
            "Symbol": "IBM",
            "Sector": "TECHNOLOGY",
            "Industry": "INFORMATION TECHNOLOGY SERVICES",
            "MarketCapitalization": "270273413000",
            "AssetType": "Common Stock"
        });
        let p = parse_overview(&v).unwrap();
        assert_eq!(p.sector.as_deref(), Some("TECHNOLOGY"));
        assert_eq!(
            p.industry.as_deref(),
            Some("INFORMATION TECHNOLOGY SERVICES")
        );
        assert_eq!(p.market_cap, Some(270273413000.0));
    }

    #[test]
    fn overview_error_envelope() {
        let v = json!({"Note": "rate limited"});
        assert!(parse_overview(&v).unwrap_err().contains("Note"));
    }

    #[test]
    fn encode_decode_industry_roundtrip() {
        use pomelo_data::LocalSource;

        let mut map = BTreeMap::new();
        map.insert("AAPL".into(), ("TECHNOLOGY".into(), Some(1e12)));
        map.insert("XOM".into(), ("ENERGY".into(), None));
        let bytes = encode_industry(&map).unwrap();
        let dir = std::env::temp_dir().join("pomelo_av_industry_rt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tracked")).unwrap();
        std::fs::write(dir.join("tracked/universe.csv.gz"), &bytes).unwrap();
        let loaded = load_existing_industry(&LocalSource::new(&dir));
        assert_eq!(
            loaded.get("AAPL").map(|(s, _)| s.as_str()),
            Some("TECHNOLOGY")
        );
        assert_eq!(loaded.get("XOM").map(|(s, _)| s.as_str()), Some("ENERGY"));
    }

    #[test]
    fn overview_url_shape() {
        let u = overview_url("IBM", "TOK");
        assert!(u.contains("function=OVERVIEW"));
        assert!(u.contains("symbol=IBM"));
        assert!(u.contains("apikey=TOK"));
    }
}
