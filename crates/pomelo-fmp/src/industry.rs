//! Company profile fetch and industry snapshot I/O.

use std::collections::BTreeMap;

use pomelo_data::industry::parse_industry_csv;
use pomelo_data::ObjectSource;
use serde_json::Value;

use super::http::Fetcher;
use super::util::num;
use super::HttpClient;
use super::{FMP_BASE, INDUSTRY_KEY};

pub(crate) fn profile_url(sym: &str, key: &str) -> String {
    format!("{FMP_BASE}/stable/profile?symbol={sym}&apikey={key}")
}

/// The bits of the company profile the sync uses: sector (for the industry
/// map), the finer `industry` (for `pe_industry_pctile` cohorts), market cap
/// (cap screen), and security-type flags (stock-only screen).
pub(crate) struct Profile {
    pub(crate) sector: Option<String>,
    pub(crate) industry: Option<String>,
    pub(crate) market_cap: Option<f64>,
    pub(crate) is_etf: bool,
    pub(crate) is_fund: bool,
}

/// A boolean profile flag that FMP may serialize as a JSON bool or a string.
pub(crate) fn flag(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    match obj.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Fetch the company profile. `Ok(None)` = the endpoint returned no row for the
/// symbol (unknown ticker on this plan).
pub(crate) fn fetch_profile<H: HttpClient>(
    fetcher: &Fetcher<H>,
    sym: &str,
    api_key: &str,
) -> Result<Option<Profile>, String> {
    let rows = fetcher.get_rows(&profile_url(sym, api_key))?;
    let Some(obj) = rows.first().and_then(Value::as_object) else {
        return Ok(None);
    };
    let field = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Ok(Some(Profile {
        sector: field("sector"),
        industry: field("industry"),
        market_cap: num(obj, &["marketCap", "marketCapitalization"]),
        is_etf: flag(obj, "isEtf"),
        is_fund: flag(obj, "isFund"),
    }))
}

/// Read the existing `tracked/universe.csv.gz` back into the accumulator so a
/// resumed run does not drop sectors for symbols it skipped.
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

/// Decode CSV bytes that may be gzip (`.csv.gz`, what we write) or plain UTF-8.
pub(crate) fn decode_csv_text(bytes: &[u8]) -> String {
    use std::io::Read;
    // gzip magic 1f 8b → gunzip; otherwise treat as plain text.
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

/// Encode the industry map as gzip CSV: `symbol,sector,market_cap`.
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
