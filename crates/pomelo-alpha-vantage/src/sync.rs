//! Orchestrate multi-symbol Alpha Vantage → data-layout sync.

use std::collections::BTreeMap;
use std::path::Path;

use pomelo_data::csv_io::write_series;
use pomelo_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::fundamentals::sync_fundamentals;
use super::http::{Fetcher, HttpClient};
use super::industry::{encode_industry, fetch_overview, load_existing_industry, INDUSTRY_KEY};
use super::price::{parse_price_payload, price_url, read_existing_prices};
use super::snapshot::{compute_symbol, SnapshotAccum};
use super::symbol::split_symbol;

/// Alpha Vantage query API root (no trailing slash).
pub const ALPHA_VANTAGE_BASE: &str = "https://www.alphavantage.co/query";

/// Sync `symbols` from Alpha Vantage into the local `out` tree — convenience
/// wrapper over [`sync_into`] for the common on-disk case.
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
/// Always fetches `TIME_SERIES_DAILY_ADJUSTED` → `prices/{SYM}.csv.gz`.
/// With `include_fundamentals`, densifies annual IS/BS into
/// `fundamentals/{SYM}.csv.gz` (period-end visibility). With
/// `include_industry`, builds `tracked/universe.csv.gz` from OVERVIEW.
/// With `include_snapshot_factors`, best-effort panels (analyst / fcf / pe_industry).
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
        let Some((layout, av)) = split_symbol(raw, &cfg.default_exchange) else {
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
                match fetch_overview(&fetcher, &av, api_key) {
                    Ok(Some(p)) => {
                        if let Some(sector) = p.sector {
                            industry.insert(layout.clone(), (sector, p.market_cap));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("{layout}: industry fetch skipped: {e}"),
                }
            }
            continue;
        }

        eprintln!("{layout}: fetching prices ({av})…");
        let fetched = match fetcher
            .get_json(&price_url(&av, api_key))
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
            match sync_fundamentals(&fetcher, store, &layout, &av, api_key, &price_days) {
                Ok(true) => summary.fundamentals_written += 1,
                Ok(false) => eprintln!("{layout}: no annual fundamentals snapshots"),
                Err(e) => {
                    eprintln!("{layout}: fundamentals skipped: {e}");
                    summary
                        .failures
                        .push((format!("{layout} (fundamentals)"), e));
                }
            }
        }

        if cfg.include_industry {
            match fetch_overview(&fetcher, &av, api_key) {
                Ok(Some(p)) => {
                    if let Some(sector) = p.sector {
                        industry.insert(layout.clone(), (sector, p.market_cap));
                    } else {
                        eprintln!("{layout}: no Sector in OVERVIEW");
                    }
                }
                Ok(None) => eprintln!("{layout}: no OVERVIEW for industry map"),
                Err(e) => {
                    eprintln!("{layout}: industry fetch failed: {e}");
                    summary.failures.push((format!("{layout} (industry)"), e));
                }
            }
        }

        if let Some(acc) = snapshots.as_mut() {
            let last_close = rows.last().map(|r| r.adj_close).unwrap_or(f64::NAN);
            let snap = compute_symbol(&fetcher, &av, api_key, &price_days, last_close);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpClient, HttpError};
    use pomelo_data::csv_io::parse_series;
    use pomelo_data::{Field, LocalSource};
    use std::cell::RefCell;
    use std::time::Duration;

    struct MockHttp {
        /// If URL matches substring → body; first match wins. Empty body uses last.
        routes: Vec<(String, RefCell<Vec<u8>>)>,
        default: Vec<u8>,
    }
    impl HttpClient for MockHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            for (pat, body) in &self.routes {
                if url.contains(pat) {
                    return Ok(body.borrow().clone());
                }
            }
            Ok(self.default.clone())
        }
    }

    struct FailHttp;
    impl HttpClient for FailHttp {
        fn get(&self, _url: &str) -> Result<Vec<u8>, HttpError> {
            Err(HttpError::Status(500))
        }
    }

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

    const IBM_TS: &str = r#"{
        "Time Series (Daily)": {
            "2024-01-04": {
                "1. open": "11.0", "2. high": "12.0", "3. low": "10.5",
                "4. close": "11.5", "5. adjusted close": "11.5", "6. volume": "1200"
            },
            "2024-01-03": {
                "1. open": "10.1", "2. high": "11.5", "3. low": "9.8",
                "4. close": "10.8", "5. adjusted close": "10.8", "6. volume": "1100"
            },
            "2024-01-02": {
                "1. open": "9.5", "2. high": "11.0", "3. low": "9.0",
                "4. close": "10.0", "5. adjusted close": "10.0", "6. volume": "1000"
            }
        }
    }"#;

    const IBM_OV: &str = r#"{
        "Symbol": "IBM",
        "Sector": "TECHNOLOGY",
        "Industry": "INFORMATION TECHNOLOGY SERVICES",
        "MarketCapitalization": "1000000000"
    }"#;

    #[test]
    fn rejects_empty_key_and_symbols() {
        let dir = std::env::temp_dir().join("pomelo_av_rej");
        let http = MockHttp {
            routes: vec![],
            default: IBM_TS.as_bytes().to_vec(),
        };
        assert!(sync(&http, "", &["AAPL".into()], &dir, &cfg()).is_err());
        assert!(sync(&http, "tok", &[], &dir, &cfg()).is_err());
    }

    #[test]
    fn syncs_prices_to_layout() {
        let dir = std::env::temp_dir().join("pomelo_av_prices");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            routes: vec![],
            default: IBM_TS.as_bytes().to_vec(),
        };
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 3);
        assert!(summary.failures.is_empty());

        let src = LocalSource::new(&dir);
        let bytes = src.get("prices/IBM.csv.gz").unwrap().unwrap();
        assert_eq!(
            parse_series(&bytes, Field::AdjClose).unwrap(),
            vec![(20240102, 10.0), (20240103, 10.8), (20240104, 11.5)]
        );
    }

    #[test]
    fn writes_industry_map_when_flagged() {
        let dir = std::env::temp_dir().join("pomelo_av_industry");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            routes: vec![
                (
                    "TIME_SERIES_DAILY_ADJUSTED".into(),
                    RefCell::new(IBM_TS.as_bytes().to_vec()),
                ),
                ("OVERVIEW".into(), RefCell::new(IBM_OV.as_bytes().to_vec())),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert!(summary.industry_written);
        let text = crate::industry::decode_csv_text(
            &LocalSource::new(&dir).get(INDUSTRY_KEY).unwrap().unwrap(),
        );
        assert!(text.contains("IBM,TECHNOLOGY,1000000000"), "{text}");
    }

    #[test]
    fn resume_skips_existing() {
        let dir = std::env::temp_dir().join("pomelo_av_resume");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            routes: vec![],
            default: IBM_TS.as_bytes().to_vec(),
        };
        sync(&http, "demo", &["IBM".into()], &dir, &cfg()).unwrap();
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.symbols_skipped, 1);
    }

    #[test]
    fn append_merges_existing_history() {
        let dir = std::env::temp_dir().join("pomelo_av_append");
        let _ = std::fs::remove_dir_all(&dir);
        let day1 = r#"{"Time Series (Daily)":{
            "2024-01-02":{"1. open":"9.5","2. high":"11.0","3. low":"9.0","4. close":"10.0","5. adjusted close":"10.0","6. volume":"1000"}
        }}"#;
        let day2 = r#"{"Time Series (Daily)":{
            "2024-01-03":{"1. open":"10.1","2. high":"11.5","3. low":"9.8","4. close":"10.8","5. adjusted close":"10.8","6. volume":"1100"}
        }}"#;
        let http = MockHttp {
            routes: vec![],
            default: day1.as_bytes().to_vec(),
        };
        let mut c = cfg();
        c.from = 20240102;
        c.to = 20240102;
        sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();

        let http2 = MockHttp {
            routes: vec![],
            default: day2.as_bytes().to_vec(),
        };
        c.from = 20240103;
        c.to = 20240103;
        c.mode = WriteMode::Append;
        let summary = sync(&http2, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.price_rows, 2);
    }

    #[test]
    fn fetch_failure_recorded() {
        let dir = std::env::temp_dir().join("pomelo_av_fail");
        let _ = std::fs::remove_dir_all(&dir);
        let summary = sync(&FailHttp, "tok", &["IBM".into()], &dir, &cfg()).unwrap();
        assert_eq!(summary.symbols_written, 0);
        assert_eq!(summary.failures.len(), 1);
    }

    #[test]
    fn rejects_inverted_dates() {
        let dir = std::env::temp_dir().join("pomelo_av_dates");
        let http = MockHttp {
            routes: vec![],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.from = 20250101;
        c.to = 20200101;
        assert!(sync(&http, "tok", &["IBM".into()], &dir, &c).is_err());
    }

    #[test]
    fn invalid_symbols_only_errors() {
        let dir = std::env::temp_dir().join("pomelo_av_bad");
        let http = MockHttp {
            routes: vec![],
            default: b"{}".to_vec(),
        };
        let err = sync(&http, "tok", &[".".into(), "  ".into()], &dir, &cfg()).unwrap_err();
        assert!(err.contains("no valid symbols"), "{err}");
    }

    #[test]
    fn resume_fills_missing_industry() {
        let dir = std::env::temp_dir().join("pomelo_av_resume_ind");
        let _ = std::fs::remove_dir_all(&dir);
        let http = MockHttp {
            routes: vec![
                (
                    "TIME_SERIES_DAILY_ADJUSTED".into(),
                    RefCell::new(IBM_TS.as_bytes().to_vec()),
                ),
                ("OVERVIEW".into(), RefCell::new(IBM_OV.as_bytes().to_vec())),
            ],
            default: b"{}".to_vec(),
        };
        // First pass: prices only
        sync(&http, "demo", &["IBM".into()], &dir, &cfg()).unwrap();
        // Resume with industry
        let mut c = cfg();
        c.mode = WriteMode::Resume;
        c.include_industry = true;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_skipped, 1);
        assert!(summary.industry_written);
        assert!(dir.join("tracked/universe.csv.gz").exists());
    }

    #[test]
    fn snapshot_factors_flag_writes_panels() {
        let dir = std::env::temp_dir().join("pomelo_av_snap_sync");
        let _ = std::fs::remove_dir_all(&dir);
        let ov = r#"{
          "Symbol":"S0","AnalystTargetPrice":"120","AnalystRatingStrongBuy":"5",
          "AnalystRatingBuy":"0","AnalystRatingHold":"0","AnalystRatingSell":"0",
          "AnalystRatingStrongSell":"0","PERatio":"25","MarketCapitalization":"1000",
          "Industry":"Software"
        }"#;
        let cf = r#"{"symbol":"S0","annualReports":[{"fiscalDateEnding":"2024-09-30","freeCashFlow":"100"}]}"#;
        let http = MockHttp {
            routes: vec![
                (
                    "TIME_SERIES_DAILY_ADJUSTED".into(),
                    RefCell::new(IBM_TS.as_bytes().to_vec()),
                ),
                ("OVERVIEW".into(), RefCell::new(ov.as_bytes().to_vec())),
                ("CASH_FLOW".into(), RefCell::new(cf.as_bytes().to_vec())),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_snapshot_factors = true;
        let syms: Vec<String> = (0..5).map(|i| format!("S{i}")).collect();
        let summary = sync(&http, "demo", &syms, &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 5);
        assert!(summary.snapshot_factor_panels >= 1);
        assert!(dir.join("panels/analyst_upside_pct.csv.gz").exists());
    }

    #[test]
    fn fundamentals_flag_writes_dense_file() {
        let dir = std::env::temp_dir().join("pomelo_av_fund_sync");
        let _ = std::fs::remove_dir_all(&dir);
        let is = r#"{"symbol":"IBM","annualReports":[{
            "fiscalDateEnding":"2023-12-31","totalRevenue":"100","grossProfit":"40",
            "netIncome":"20","operatingIncome":"30"
        }]}"#;
        let bs = r#"{"symbol":"IBM","annualReports":[{
            "fiscalDateEnding":"2023-12-31","totalAssets":"200","totalLiabilities":"80",
            "totalShareholderEquity":"120","currentNetReceivables":"10","longTermDebt":"50"
        }]}"#;
        let http = MockHttp {
            routes: vec![
                (
                    "TIME_SERIES_DAILY_ADJUSTED".into(),
                    RefCell::new(IBM_TS.as_bytes().to_vec()),
                ),
                (
                    "INCOME_STATEMENT".into(),
                    RefCell::new(is.as_bytes().to_vec()),
                ),
                ("BALANCE_SHEET".into(), RefCell::new(bs.as_bytes().to_vec())),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_fundamentals = true;
        // price days after fiscal end so densify has a visible snapshot
        c.from = 20240102;
        c.to = 20240104;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert_eq!(summary.fundamentals_written, 1);
        assert!(dir.join("fundamentals/IBM.csv.gz").exists());
    }

    #[test]
    fn industry_missing_sector_skips_row() {
        let dir = std::env::temp_dir().join("pomelo_av_ind_empty");
        let _ = std::fs::remove_dir_all(&dir);
        let empty_ov = r#"{"Symbol":"IBM","Name":"IBM"}"#;
        let http = MockHttp {
            routes: vec![
                (
                    "TIME_SERIES_DAILY_ADJUSTED".into(),
                    RefCell::new(IBM_TS.as_bytes().to_vec()),
                ),
                (
                    "OVERVIEW".into(),
                    RefCell::new(empty_ov.as_bytes().to_vec()),
                ),
            ],
            default: b"{}".to_vec(),
        };
        let mut c = cfg();
        c.include_industry = true;
        let summary = sync(&http, "demo", &["IBM".into()], &dir, &c).unwrap();
        assert_eq!(summary.symbols_written, 1);
        assert!(!summary.industry_written);
    }
}
