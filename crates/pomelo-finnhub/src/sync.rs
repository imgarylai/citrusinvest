//! Orchestrate multi-symbol Finnhub → data-layout sync.
//!
//! Prices (#226): `/stock/candle` (resolution=D, adjusted) → `prices/{SYM}.csv.gz`
//! with resume/append modes.
//!
//! Industry (#227): `--include-industry` adds `/stock/profile2` sector/market-cap
//! → `tracked/universe.csv.gz`.
//!
//! Fundamentals (#228): `--include-fundamentals` densifies annual
//! `/stock/financials-reported` (filing-date visibility) → `fundamentals/{SYM}.csv.gz`.
//!
//! Snapshot factors (#230): `--include-snapshot-factors` writes best-effort
//! `panels/{analyst_upside_pct,consensus_rating,fcf_yield,pe_industry_pctile}`.
//! Index membership is driven from the CLI via `--index` (see `index` module).

use std::collections::BTreeMap;
use std::path::Path;

use pomelo_data::csv_io::write_series;
use pomelo_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::fundamentals::sync_fundamentals;
use super::http::{Fetcher, HttpClient};
use super::industry::{encode_industry, fetch_profile, load_existing_industry, INDUSTRY_KEY};
use super::price::{parse_price_payload, price_url, read_existing_prices};
use super::snapshot::{compute_symbol, SnapshotAccum};
use super::symbol::split_symbol;

/// Finnhub API root (no trailing slash).
pub const FINNHUB_BASE: &str = "https://finnhub.io/api/v1";

/// Sync `symbols` from Finnhub into the local `out` tree — convenience wrapper
/// over [`sync_into`] for the common on-disk case.
pub fn sync<H: HttpClient>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    out: &Path,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    sync_into(http, api_key, symbols, &LocalSource::new(out), cfg)
}

/// Storage-agnostic core: sync into any `store` (local disk or S3/R2).
///
/// Fetches adjusted daily candles → `prices/{SYM}.csv.gz` for each symbol.
/// `WriteMode::Resume` skips symbols that already have a price file;
/// `WriteMode::Append` merges the fetched window into the existing file. With
/// `include_industry`, also builds `tracked/universe.csv.gz` from
/// `/stock/profile2`; with `include_fundamentals`, densifies annual
/// `/stock/financials-reported` into `fundamentals/{SYM}.csv.gz`; with
/// `include_snapshot_factors`, writes best-effort `panels/` factors.
pub fn sync_into<H: HttpClient, S: ObjectSink + ObjectSource>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    store: &S,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    if api_key.trim().is_empty() {
        return Err("empty API key".to_string());
    }
    if symbols.is_empty() {
        return Err("no symbols requested".to_string());
    }
    if cfg.from > cfg.to {
        return Err(format!("from ({}) is after to ({})", cfg.from, cfg.to));
    }
    if cfg.default_exchange.trim().is_empty() {
        return Err("default_exchange is empty".to_string());
    }

    let fetcher = Fetcher::new(http, cfg);
    let mut summary = SyncSummary::default();
    let mut any_valid = false;
    let mut industry: BTreeMap<String, (String, Option<f64>)> = if cfg.include_industry {
        load_existing_industry(store)
    } else {
        BTreeMap::new()
    };
    let mut snapshots = cfg.include_snapshot_factors.then(SnapshotAccum::new);

    for raw in symbols {
        let Some((layout, fh)) = split_symbol(raw, &cfg.default_exchange) else {
            summary
                .failures
                .push((raw.clone(), "invalid symbol".into()));
            continue;
        };
        any_valid = true;

        let price_key = format!("{PRICES_DIR}/{layout}.csv.gz");
        if cfg.mode == WriteMode::Resume && store.get(&price_key).ok().flatten().is_some() {
            eprintln!("{layout}: already present, skipping (resume)");
            summary.symbols_skipped += 1;
            if cfg.include_industry && !industry.contains_key(&layout) {
                collect_industry(&fetcher, &fh, api_key, &layout, &mut industry, &mut summary);
            }
            continue;
        }

        eprintln!("{layout}: fetching candles ({fh})…");
        let fetched = match fetcher
            .get_json(&price_url(&fh, cfg, api_key))
            .and_then(|v| parse_price_payload(&v, cfg))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{layout}: price fetch failed: {e}");
                summary.failures.push((layout, e));
                continue;
            }
        };
        if fetched.is_empty() {
            let msg = "no price rows in range".to_string();
            eprintln!("{layout}: {msg}");
            summary.failures.push((layout, msg));
            continue;
        }

        let rows = if cfg.mode == WriteMode::Append {
            let mut by_day = read_existing_prices(store, &layout);
            for r in fetched {
                by_day.insert(r.day, r);
            }
            by_day.into_values().collect()
        } else {
            fetched
        };

        match write_series(&rows).map_err(|e| e.to_string()) {
            Ok(bytes) => {
                if let Err(e) = store.put(&price_key, &bytes) {
                    let e = e.to_string();
                    eprintln!("{layout}: write failed: {e}");
                    summary.failures.push((layout, e));
                    continue;
                }
            }
            Err(e) => {
                eprintln!("{layout}: encode failed: {e}");
                summary.failures.push((layout, e));
                continue;
            }
        }

        let price_days: Vec<i32> = rows.iter().map(|r| r.day).collect();

        if cfg.include_fundamentals {
            match sync_fundamentals(&fetcher, store, &layout, &fh, api_key, &price_days) {
                Ok(true) => summary.fundamentals_written += 1,
                Ok(false) => eprintln!("{layout}: no annual filings"),
                Err(e) => {
                    eprintln!("{layout}: fundamentals skipped: {e}");
                    summary
                        .failures
                        .push((format!("{layout} (fundamentals)"), e));
                }
            }
        }

        if cfg.include_industry {
            collect_industry(&fetcher, &fh, api_key, &layout, &mut industry, &mut summary);
        }

        if let Some(acc) = snapshots.as_mut() {
            let last_close = rows.last().map(|r| r.adj_close).unwrap_or(f64::NAN);
            let snap = compute_symbol(&fetcher, &fh, api_key, &price_days, last_close);
            acc.push(layout.clone(), snap, &price_days);
        }

        summary.symbols_written += 1;
        summary.price_rows += rows.len();
        eprintln!("{layout}: wrote {} price rows", rows.len());
    }

    if cfg.include_industry && !industry.is_empty() {
        let bytes = encode_industry(&industry).map_err(|e| e.to_string())?;
        store.put(INDUSTRY_KEY, &bytes).map_err(|e| e.to_string())?;
        summary.industry_written = true;
        eprintln!("wrote {} industry rows to {INDUSTRY_KEY}", industry.len());
    }

    if let Some(acc) = snapshots {
        summary.snapshot_factor_panels = acc.write_panels(store).map_err(|e| e.to_string())?;
    }

    if !any_valid {
        return Err("no valid symbols after normalization".to_string());
    }
    Ok(summary)
}

/// Fetch `/stock/profile2` for one symbol and fold its industry/market-cap into
/// the accumulator. Failures are recorded but never abort the batch.
fn collect_industry<H: HttpClient>(
    fetcher: &Fetcher<H>,
    fh: &str,
    api_key: &str,
    layout: &str,
    industry: &mut BTreeMap<String, (String, Option<f64>)>,
    summary: &mut SyncSummary,
) {
    match fetch_profile(fetcher, fh, api_key) {
        Ok(Some(p)) => {
            if let Some(sector) = p.industry {
                industry.insert(layout.to_string(), (sector, p.market_cap));
            } else {
                eprintln!("{layout}: no finnhubIndustry in profile2");
            }
        }
        Ok(None) => eprintln!("{layout}: no profile2 for industry map"),
        Err(e) => {
            eprintln!("{layout}: industry fetch failed: {e}");
            summary.failures.push((format!("{layout} (industry)"), e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use pomelo_data::csv_io::parse_series;
    use pomelo_data::{Field, LocalSource};
    use std::time::Duration;

    // 2024-01-02..04 at 00:00 UTC.
    const T2: i64 = 1_704_153_600;
    const T3: i64 = T2 + 86_400;
    const T4: i64 = T3 + 86_400;

    struct OkHttp {
        body: Vec<u8>,
    }
    impl HttpClient for OkHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Ok(self.body.clone())
        }
    }

    struct FailHttp;
    impl HttpClient for FailHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Status(500))
        }
    }

    /// Routes by URL substring (first match wins), else a default body.
    struct RouteHttp {
        routes: Vec<(&'static str, Vec<u8>)>,
        default: Vec<u8>,
    }
    impl HttpClient for RouteHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            for (pat, body) in &self.routes {
                if url.contains(pat) {
                    return Ok(body.clone());
                }
            }
            Ok(self.default.clone())
        }
    }

    const PROFILE2: &str = r#"{"ticker":"AAPL","name":"Apple","finnhubIndustry":"Technology","marketCapitalization":2500000}"#;

    // As-reported filing with a filedDate inside the candle window.
    const FINANCIALS: &str = r#"{"cik":"1","data":[{"year":2023,"form":"10-K",
        "endDate":"2023-12-31 00:00:00","filedDate":"2024-01-02 00:00:00",
        "report":{"ic":[{"concept":"us-gaap_Revenues","value":100},
            {"concept":"us-gaap_NetIncomeLoss","value":20}],
        "bs":[{"concept":"us-gaap_StockholdersEquity","value":120}]}}]}"#;

    fn cfg() -> SyncConfig {
        SyncConfig {
            from: 20240102,
            to: 20240104,
            rate_limit_per_min: 0,
            max_retries: 0,
            backoff_base: Duration::ZERO,
            ..SyncConfig::default()
        }
    }

    fn candle(ts: &[i64]) -> Vec<u8> {
        let n = ts.len();
        let arr = |base: f64| -> String {
            (0..n)
                .map(|i| (base + i as f64).to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let t = ts
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"s":"ok","t":[{t}],"o":[{}],"h":[{}],"l":[{}],"c":[{}],"v":[{}]}}"#,
            arr(9.0),
            arr(11.0),
            arr(8.0),
            arr(10.0),
            arr(1000.0),
        )
        .into_bytes()
    }

    #[test]
    fn rejects_empty_key_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_fh_rej");
        let http = OkHttp {
            body: candle(&[T2]),
        };
        assert!(sync(&http, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&http, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn syncs_prices_to_layout() {
        let dir = std::env::temp_dir().join("pomelo_fh_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: candle(&[T2, T3, T4]),
        };
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 3);
        assert!(summary.failures.is_empty());

        let src = LocalSource::new(&dir);
        let bytes = src.get("prices/AAPL.csv.gz").unwrap().unwrap();
        assert_eq!(
            parse_series(&bytes, Field::AdjClose).unwrap(),
            vec![(20240102, 10.0), (20240103, 11.0), (20240104, 12.0)]
        );
    }

    #[test]
    fn resume_skips_existing() {
        let dir = std::env::temp_dir().join("pomelo_fh_resume");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: candle(&[T2, T3, T4]),
        };
        sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.symbols_skipped, 1);
    }

    #[test]
    fn append_merges_existing_history() {
        let dir = std::env::temp_dir().join("pomelo_fh_append");
        let _ = std::fs::remove_dir_all(&dir);

        let mut c = cfg();
        c.from = 20240102;
        c.to = 20240102;
        let http = OkHttp {
            body: candle(&[T2]),
        };
        sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();

        c.from = 20240103;
        c.to = 20240103;
        c.mode = WriteMode::Append;
        let http2 = OkHttp {
            body: candle(&[T3]),
        };
        let summary = sync(&http2, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 2);
    }

    #[test]
    fn fetch_failure_recorded() {
        let dir = std::env::temp_dir().join("pomelo_fh_fail");
        let _ = std::fs::remove_dir_all(&dir);
        let summary = sync(&FailHttp, "tok", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
    }

    #[test]
    fn no_data_recorded_as_failure() {
        let dir = std::env::temp_dir().join("pomelo_fh_nodata");
        let _ = std::fs::remove_dir_all(&dir);
        let http = OkHttp {
            body: br#"{"s":"no_data"}"#.to_vec(),
        };
        let summary = sync(&http, "tok", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.failures[0].1.contains("no price rows"));
    }

    #[test]
    fn rejects_inverted_dates() {
        let dir = std::env::temp_dir().join("pomelo_fh_dates");
        let http = OkHttp {
            body: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20200101;
        assert!(sync(&http, "tok", &["AAPL".into()], &dir, &c).is_err());
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_fh_bad");
        let http = OkHttp {
            body: b"{}".to_vec(),
        };
        let err = sync(&http, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }

    #[test]
    fn writes_industry_map_when_flagged() {
        let dir = std::env::temp_dir().join("pomelo_fh_industry");
        let _ = std::fs::remove_dir_all(&dir);
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("stock/profile2", PROFILE2.as_bytes().to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert!(summary.industry_written);

        let text = crate::industry::decode_csv_text(
            &LocalSource::new(&dir).get(INDUSTRY_KEY).unwrap().unwrap(),
        );
        // market cap scaled from 2,500,000 million → 2.5e12 absolute
        assert!(text.contains("AAPL,Technology,2500000000000"), "{text}");
    }

    #[test]
    fn industry_absent_without_flag() {
        let dir = std::env::temp_dir().join("pomelo_fh_no_industry");
        let _ = std::fs::remove_dir_all(&dir);
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("stock/profile2", PROFILE2.as_bytes().to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert!(!summary.industry_written);
        assert!(!dir.join("tracked/universe.csv.gz").exists());
    }

    #[test]
    fn empty_profile_skips_industry_row() {
        let dir = std::env::temp_dir().join("pomelo_fh_empty_profile");
        let _ = std::fs::remove_dir_all(&dir);
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("stock/profile2", b"{}".to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert!(!summary.industry_written);
    }

    #[test]
    fn price_write_failure_recorded() {
        use pomelo_data::error::DataError;
        use pomelo_data::{ObjectSink, ObjectSource};

        // A store whose writes always fail; reads are empty.
        struct FailSink;
        impl ObjectSource for FailSink {
            fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, DataError> {
                Ok(None)
            }
        }
        impl ObjectSink for FailSink {
            fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), DataError> {
                Err(DataError::Io("disk full".into()))
            }
        }

        let http = OkHttp {
            body: candle(&[T2, T3, T4]),
        };
        let summary = sync_into(&http, "demo", &["AAPL".into()], &FailSink, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
        assert!(
            summary.failures[0].1.contains("disk full"),
            "{:?}",
            summary.failures
        );
    }

    #[test]
    fn fundamentals_flag_writes_dense_file() {
        let dir = std::env::temp_dir().join("pomelo_fh_fund_flag");
        let _ = std::fs::remove_dir_all(&dir);
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("financials-reported", FINANCIALS.as_bytes().to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_fundamentals = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.fundamentals_written, 1);
        assert!(dir.join("fundamentals/AAPL.csv.gz").exists());
    }

    #[test]
    fn snapshot_factors_flag_writes_panels() {
        let dir = std::env::temp_dir().join("pomelo_fh_snap_flag");
        let _ = std::fs::remove_dir_all(&dir);
        let rec =
            br#"[{"period":"2024-03-01","strongBuy":2,"buy":3,"hold":0,"sell":0,"strongSell":0}]"#;
        let pt = br#"{"targetMean":120.0}"#;
        let metric =
            br#"{"metric":{"peTTM":25.0,"marketCapitalization":1000.0,"freeCashFlowTTM":100.0}}"#;
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("stock/recommendation", rec.to_vec()),
                ("stock/price-target", pt.to_vec()),
                ("stock/metric", metric.to_vec()),
                ("stock/profile2", PROFILE2.as_bytes().to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_snapshot_factors = true;
        // Five symbols so the pe_industry_pctile cohort meets MIN_COHORT.
        let syms: Vec<String> = (0..5).map(|i| format!("S{i}")).collect();
        let summary = sync(&http, "demo", &syms, &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 5);
        assert!(summary.snapshot_factor_panels >= 3);
        assert!(dir.join("panels/consensus_rating.csv.gz").exists());
        assert!(dir.join("panels/pe_industry_pctile.csv.gz").exists());
    }

    #[test]
    fn fundamentals_error_recorded_not_fatal() {
        let dir = std::env::temp_dir().join("pomelo_fh_fund_flag_err");
        let _ = std::fs::remove_dir_all(&dir);
        // Candle OK; financials-reported hard-fails.
        struct CandleOkFinErr(Vec<u8>);
        impl HttpClient for CandleOkFinErr {
            fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
                if url.contains("stock/candle") {
                    Ok(self.0.clone())
                } else {
                    Err(HttpError::Status(403))
                }
            }
        }
        let http = CandleOkFinErr(candle(&[T2, T3, T4]));
        let mut c = cfg();
        c.include_fundamentals = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.fundamentals_written, 0);
        assert!(summary
            .failures
            .iter()
            .any(|(s, _)| s.contains("(fundamentals)")));
    }

    #[test]
    fn profile_without_industry_writes_no_row() {
        let dir = std::env::temp_dir().join("pomelo_fh_profile_no_ind");
        let _ = std::fs::remove_dir_all(&dir);
        // Market cap present but no `finnhubIndustry` → no universe row.
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                (
                    "stock/profile2",
                    br#"{"marketCapitalization":1000}"#.to_vec(),
                ),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert!(!summary.industry_written);
    }

    #[test]
    fn industry_fetch_error_recorded_not_fatal() {
        let dir = std::env::temp_dir().join("pomelo_fh_ind_err");
        let _ = std::fs::remove_dir_all(&dir);
        // Candles succeed; profile2 hard-fails (e.g. plan-gated).
        struct CandleOkProfileErr(Vec<u8>);
        impl HttpClient for CandleOkProfileErr {
            fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
                if url.contains("stock/candle") {
                    Ok(self.0.clone())
                } else {
                    Err(HttpError::Status(403))
                }
            }
        }
        let http = CandleOkProfileErr(candle(&[T2, T3, T4]));
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        // Price still written; industry failure recorded, batch not aborted.
        assert_eq!(summary.symbols_written, 1);
        assert!(!summary.industry_written);
        assert!(summary
            .failures
            .iter()
            .any(|(s, _)| s.contains("(industry)")));
    }

    #[test]
    fn resume_fills_missing_industry() {
        let dir = std::env::temp_dir().join("pomelo_fh_resume_ind");
        let _ = std::fs::remove_dir_all(&dir);
        let http = RouteHttp {
            routes: vec![
                ("stock/candle", candle(&[T2, T3, T4])),
                ("stock/profile2", PROFILE2.as_bytes().to_vec()),
            ],
            default: b"{}".to_vec(),
        };
        // First pass: prices only.
        sync(&http, "demo", &["AAPL".into()], &dir, &cfg()).unwrap();
        // Resume with industry: price file present → skipped, industry still filled.
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        c.include_industry = true;
        let summary = sync(&http, "demo", &["AAPL".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_skipped, 1);
        assert!(summary.industry_written);
        assert!(dir.join("tracked/universe.csv.gz").exists());
    }
}
