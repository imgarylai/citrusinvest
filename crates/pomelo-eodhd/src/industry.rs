//! Sector / industry map from fundamentals → `tracked/universe.csv.gz`.

use std::collections::BTreeMap;

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::ObjectSource;
use serde_json::Value;

use super::http::Fetcher;
use super::util::num;
use super::HttpClient;
use super::EODHD_BASE;

/// Object key the industry snapshot is written under.
pub const INDUSTRY_KEY: &str = "tracked/universe.csv.gz";

/// Bits of EODHD fundamentals `General` / `Highlights` used for the industry map.
#[derive(Debug, Clone, Default)]
pub(crate) struct Profile {
    pub(crate) sector: Option<String>,
    pub(crate) industry: Option<String>,
    pub(crate) market_cap: Option<f64>,
    pub(crate) security_type: Option<String>,
}

/// Fundamentals filter URL (cheap partial payload).
pub(crate) fn profile_url(eodhd_code: &str, api_token: &str) -> String {
    format!(
        "{EODHD_BASE}/v1.1/fundamentals/{eodhd_code}?api_token={api_token}&fmt=json\
         &filter=General::Sector,General::Industry,General::Type,Highlights::MarketCapitalization"
    )
}

fn str_field(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse a filtered fundamentals object (flat `General::Sector` keys or nested).
pub(crate) fn parse_profile(value: &Value) -> Profile {
    let mut p = Profile::default();
    if let Value::Object(map) = value {
        // Flat filter form from EODHD multi-filter.
        if let Some(s) = map.get("General::Sector").and_then(str_field) {
            p.sector = Some(s);
        }
        if let Some(s) = map.get("General::Industry").and_then(str_field) {
            p.industry = Some(s);
        }
        if let Some(s) = map.get("General::Type").and_then(str_field) {
            p.security_type = Some(s);
        }
        if let Some(m) = map
            .get("Highlights::MarketCapitalization")
            .and_then(|v| match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            })
        {
            p.market_cap = Some(m);
        }
        // Nested full fundamentals form.
        if p.sector.is_none() {
            if let Some(g) = map.get("General").and_then(Value::as_object) {
                p.sector = g.get("Sector").and_then(str_field);
                p.industry = g.get("Industry").and_then(str_field).or(p.industry);
                p.security_type = g.get("Type").and_then(str_field).or(p.security_type);
            }
        }
        if p.market_cap.is_none() {
            if let Some(h) = map.get("Highlights").and_then(Value::as_object) {
                p.market_cap = num(h, &["MarketCapitalization", "marketCapitalization"]);
            }
        }
    }
    p
}

/// Fetch profile/sector for one EODHD code.
pub(crate) fn fetch_profile<H: HttpClient>(
    fetcher: &Fetcher<H>,
    eodhd_code: &str,
    api_token: &str,
) -> Result<Option<Profile>, String> {
    let value = fetcher.get_json(&profile_url(eodhd_code, api_token))?;
    let p = parse_profile(&value);
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
    fn parse_flat_filter_form() {
        let v = json!({
            "General::Sector": "Technology",
            "General::Industry": "Consumer Electronics",
            "General::Type": "Common Stock",
            "Highlights::MarketCapitalization": 1e12
        });
        let p = parse_profile(&v);
        assert_eq!(p.sector.as_deref(), Some("Technology"));
        assert_eq!(p.industry.as_deref(), Some("Consumer Electronics"));
        assert_eq!(p.market_cap, Some(1e12));
    }

    #[test]
    fn parse_nested_form() {
        let v = json!({
            "General": {"Sector": "Energy", "Industry": "Oil", "Type": "Common Stock"},
            "Highlights": {"MarketCapitalization": 100.0}
        });
        let p = parse_profile(&v);
        assert_eq!(p.sector.as_deref(), Some("Energy"));
        assert_eq!(p.market_cap, Some(100.0));
    }

    #[test]
    fn encode_decode_industry_roundtrip() {
        use pomelo_data::LocalSource;

        let mut map = BTreeMap::new();
        map.insert("AAPL".into(), ("Technology".into(), Some(1e12)));
        map.insert("XOM".into(), ("Energy".into(), None));
        let bytes = encode_industry(&map).unwrap();
        let dir = std::env::temp_dir().join("pomelo_eodhd_industry_rt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tracked")).unwrap();
        std::fs::write(dir.join("tracked/universe.csv.gz"), &bytes).unwrap();
        let loaded = load_existing_industry(&LocalSource::new(&dir));
        assert_eq!(
            loaded.get("AAPL").map(|(s, _)| s.as_str()),
            Some("Technology")
        );
        assert_eq!(loaded.get("XOM").map(|(s, _)| s.as_str()), Some("Energy"));
    }
}
