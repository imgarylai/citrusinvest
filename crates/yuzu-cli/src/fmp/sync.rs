//! Orchestrate a multi-symbol FMP → local data-layout sync.

use std::collections::BTreeMap;
use std::path::Path;

use yuzu_data::csv_io::{write_series, OhlcvRow};
use yuzu_data::{LocalSource, ObjectSink, ObjectSource, PRICES_DIR};

use super::config::{SyncConfig, SyncSummary, WriteMode};
use super::fundamentals::sync_fundamentals;
use super::http::Fetcher;
use super::industry::{encode_industry, fetch_profile, load_existing_industry, Profile};
use super::price::{parse_price_rows, price_url, read_existing_prices};
use super::HttpClient;
use super::INDUSTRY_KEY;

/// Sync `symbols` from FMP into the local `out` tree per [`SyncConfig`] — a thin
/// convenience wrapper over [`sync_into`] for the common on-disk case.
pub fn sync<H: HttpClient>(
    http: &H,
    api_key: &str,
    symbols: &[String],
    out: &Path,
    cfg: &SyncConfig,
) -> Result<SyncSummary, String> {
    sync_into(http, api_key, symbols, &LocalSource::new(out), cfg)
}

/// Storage-agnostic core: sync `symbols` from FMP into any `store` — local disk
/// ([`LocalSource`]) or an S3/R2 bucket (`yuzu-source-s3`'s `S3Source`) — so the
/// CLI and a backend service produce **byte-identical** trees for the same
/// inputs. Prices are always fetched; fundamentals and industry are opt-in.
/// Progress and per-symbol failures are logged to stderr (API key redacted); a
/// per-symbol failure is recorded and the batch continues.
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

    let fetcher = Fetcher::new(http, cfg);
    let mut summary = SyncSummary::default();

    // For --include-industry, start from the existing snapshot so resumed /
    // skipped symbols keep their sector rows.
    let mut industry: BTreeMap<String, (String, Option<f64>)> = if cfg.include_industry {
        load_existing_industry(store)
    } else {
        BTreeMap::new()
    };

    // One profile GET per symbol serves the ETF/fund screen, the market-cap
    // screen, and the industry map — fetch it only when at least one needs it.
    let need_profile = cfg.skip_non_stocks || cfg.min_market_cap > 0.0 || cfg.include_industry;

    for sym in symbols {
        let price_key = format!("{PRICES_DIR}/{sym}.csv.gz");
        if cfg.mode == WriteMode::Resume && store.get(&price_key).ok().flatten().is_some() {
            eprintln!("{sym}: already present, skipping (resume)");
            summary.symbols_skipped += 1;
            continue;
        }

        // Screen before fetching prices so filtered symbols cost no price request.
        // A profile hiccup fails *open* (keep the symbol) — a transient error on
        // the secondary endpoint must not drop the primary price sync.
        let mut profile: Option<Profile> = None;
        if need_profile {
            match fetch_profile(&fetcher, sym, api_key) {
                Ok(Some(p)) => {
                    if cfg.skip_non_stocks && (p.is_etf || p.is_fund) {
                        let kind = if p.is_etf { "ETF" } else { "fund" };
                        eprintln!("{sym}: {kind}, skipping (pass --include-etf to keep)");
                        summary.symbols_filtered += 1;
                        continue;
                    }
                    if cfg.min_market_cap > 0.0 {
                        match p.market_cap {
                            Some(mc) if mc < cfg.min_market_cap => {
                                eprintln!(
                                    "{sym}: market cap {mc:.0} < {:.0}, skipping",
                                    cfg.min_market_cap
                                );
                                summary.symbols_filtered += 1;
                                continue;
                            }
                            None => eprintln!("{sym}: market cap unknown, keeping (cannot screen)"),
                            _ => {}
                        }
                    }
                    profile = Some(p);
                }
                Ok(None) if cfg.skip_non_stocks || cfg.min_market_cap > 0.0 => {
                    eprintln!("{sym}: no profile data, cannot screen (keeping)");
                }
                Ok(None) => {}
                Err(e) => eprintln!("{sym}: profile unavailable, cannot screen (keeping): {e}"),
            }
        }

        eprintln!("{sym}: fetching prices…");
        let fetched = match fetcher
            .get_rows(&price_url(sym, cfg, api_key))
            .map(|rows| parse_price_rows(&rows, cfg))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{sym}: price fetch failed: {e}");
                summary.failures.push((sym.clone(), e));
                continue;
            }
        };
        if fetched.is_empty() {
            let msg = "no price rows in range".to_string();
            eprintln!("{sym}: {msg}");
            summary.failures.push((sym.clone(), msg));
            continue;
        }

        // Merge onto existing history when appending.
        let rows: Vec<OhlcvRow> = if cfg.mode == WriteMode::Append {
            let mut by_day = read_existing_prices(store, sym);
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
                    eprintln!("{sym}: write failed: {e}");
                    summary.failures.push((sym.clone(), e));
                    continue;
                }
            }
            Err(e) => {
                eprintln!("{sym}: encode failed: {e}");
                summary.failures.push((sym.clone(), e));
                continue;
            }
        }
        summary.symbols_written += 1;
        summary.price_rows += rows.len();
        eprintln!("{sym}: wrote {} price rows", rows.len());

        let price_days: Vec<i32> = rows.iter().map(|r| r.day).collect();

        if cfg.include_fundamentals {
            match sync_fundamentals(&fetcher, store, sym, api_key, &price_days) {
                Ok(true) => summary.fundamentals_written += 1,
                Ok(false) => {}
                Err(e) => {
                    eprintln!("{sym}: fundamentals skipped: {e}");
                    summary.failures.push((format!("{sym} (fundamentals)"), e));
                }
            }
        }

        if cfg.include_industry {
            // Reuse the profile fetched above for the screen.
            match profile
                .as_ref()
                .and_then(|p| p.sector.as_ref().map(|s| (s.clone(), p.market_cap)))
            {
                Some((sector, mcap)) => {
                    industry.insert(sym.clone(), (sector, mcap));
                }
                None => eprintln!("{sym}: no sector in profile"),
            }
        }
    }

    if cfg.include_industry && !industry.is_empty() {
        let bytes = encode_industry(&industry).map_err(|e| e.to_string())?;
        store.put(INDUSTRY_KEY, &bytes).map_err(|e| e.to_string())?;
        summary.industry_written = true;
        eprintln!("wrote {} industry rows to {INDUSTRY_KEY}", industry.len());
    }

    Ok(summary)
}
