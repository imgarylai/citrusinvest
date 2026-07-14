//! `lemon run --sync`: fill data gaps before a run.
//!
//! The strategy file only ever *declares* its needs (`#! symbols:`,
//! `#! require:`, `#! data-source: fmp`); fetching happens here in the
//! runner, and only when the person running the file passes `--sync` and
//! supplies their own API key through the environment (`$FMP_API_KEY`).
//! A shared strategy file can never trigger network activity by itself —
//! that is the security stance of issue #246, not an implementation detail.
//!
//! The gap check is by symbol: names in the declared universe with no
//! `prices/<sym>.csv.gz` in the tree. When nothing is missing, `--sync` is a
//! no-op (zero requests). Only the missing names are fetched, so existing
//! files are never rewritten.

use std::path::Path;

use crate::frontmatter::DataSource;

/// Symbols in `requested` with no price file under `root`. An unreadable or
/// absent tree counts as "nothing synced yet" — every symbol is missing.
pub(crate) fn missing_symbols(root: &Path, requested: &[String]) -> Vec<String> {
    let have = pomelo_data::list_symbols(root).unwrap_or_default();
    let have: std::collections::HashSet<&str> = have.iter().map(String::as_str).collect();
    requested
        .iter()
        .filter(|s| !have.contains(s.as_str()))
        .cloned()
        .collect()
}

/// True when the strategy needs fundamentals synced: a `#! require:` entry or
/// a referenced `Data` leaf naming a fundamental series.
pub(crate) fn wants_fundamentals(spec: &serde_json::Value, require: Option<&[String]>) -> bool {
    if require
        .unwrap_or_default()
        .iter()
        .any(|n| pomelo_data::is_fundamental_series(n))
    {
        return true;
    }
    fn walk(node: &serde_json::Value) -> bool {
        match node {
            serde_json::Value::Object(map) => {
                if map.get("op").and_then(serde_json::Value::as_str) == Some("Data") {
                    if let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
                        if pomelo_data::is_fundamental_series(name) {
                            return true;
                        }
                    }
                }
                map.values().any(walk)
            }
            serde_json::Value::Array(arr) => arr.iter().any(walk),
            _ => false,
        }
    }
    walk(spec)
}

/// One gap-fill request: which vendor, which names, into which tree.
pub(crate) struct SyncRequest<'a> {
    pub source: DataSource,
    pub missing: &'a [String],
    pub from: i32,
    pub to: i32,
    pub include_fundamentals: bool,
    pub root: &'a Path,
}

/// Fetch the request's missing symbols. Generic over the HTTP client so tests
/// inject a mock; [`sync_live`] wraps it with the real TLS client. Returns a
/// one-line human summary.
#[cfg_attr(not(feature = "fmp-sync"), allow(dead_code))]
pub(crate) fn sync_missing<H: pomelo_fmp::HttpClient>(
    req: &SyncRequest,
    http: &H,
    key: &str,
) -> Result<String, String> {
    match req.source {
        DataSource::Fmp => {
            let cfg = pomelo_fmp::SyncConfig {
                from: req.from,
                // FMP date bounds are real calendar dates; clamp the runner's
                // open-ended default (99991231).
                to: req.to.min(20991231),
                include_fundamentals: req.include_fundamentals,
                // The user listed these names explicitly — screening out ETFs
                // or small caps here would be a silent drop.
                skip_non_stocks: false,
                ..Default::default()
            };
            let summary = pomelo_fmp::sync(http, key, req.missing, req.root, &cfg)?;
            if !summary.failures.is_empty() {
                return Err(format!(
                    "sync failed for {} of {} symbols: {:?}",
                    summary.failures.len(),
                    req.missing.len(),
                    summary.failures
                ));
            }
            Ok(format!(
                "synced {} symbols ({} price rows) from FMP",
                summary.symbols_written, summary.price_rows
            ))
        }
    }
}

/// The real `--sync` entry point: key from `$FMP_API_KEY`, ureq TLS client.
/// Compiled out without the `fmp-sync` feature (offline build).
#[cfg(feature = "fmp-sync")]
pub(crate) fn sync_live(req: &SyncRequest) -> Result<String, String> {
    let key = std::env::var("FMP_API_KEY")
        .map_err(|_| "--sync needs $FMP_API_KEY set (bring your own key)".to_string())?;
    let http = pomelo_fmp::UreqClient::new();
    sync_missing(req, &http, &key)
}

#[cfg(not(feature = "fmp-sync"))]
pub(crate) fn sync_live(_req: &SyncRequest) -> Result<String, String> {
    Err("this lemon build has no sync support (rebuild with `--features fmp-sync`)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Minimal scripted HTTP mock: URL substring → JSON body (else 404).
    struct MockHttp {
        routes: Vec<(String, String)>,
        hits: RefCell<usize>,
    }

    impl pomelo_fmp::HttpClient for MockHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, pomelo_fmp::HttpError> {
            *self.hits.borrow_mut() += 1;
            for (pat, body) in &self.routes {
                if url.contains(pat) {
                    return Ok(body.clone().into_bytes());
                }
            }
            Err(pomelo_fmp::HttpError::Status(404))
        }
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lemon_cli_sync_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn syms(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    const BBB_PRICES: &str = r#"[
        {"date":"2024-01-02","adjOpen":5.0,"adjHigh":5.0,"adjLow":5.0,"adjClose":5.0,"volume":100},
        {"date":"2024-01-03","adjOpen":4.0,"adjHigh":4.0,"adjLow":4.0,"adjClose":4.0,"volume":100}
    ]"#;

    #[test]
    fn gap_check_reports_missing_and_handles_an_empty_tree() {
        let dir = tmp("gap");
        // Nothing synced yet: everything is missing.
        assert_eq!(
            missing_symbols(&dir, &syms(&["AAA", "BBB"])),
            syms(&["AAA", "BBB"])
        );
        // With AAA present, only BBB is a gap.
        std::fs::create_dir_all(dir.join("prices")).unwrap();
        std::fs::write(dir.join("prices/AAA.csv.gz"), b"x").unwrap();
        assert_eq!(
            missing_symbols(&dir, &syms(&["AAA", "BBB"])),
            syms(&["BBB"])
        );
        assert!(missing_symbols(&dir, &syms(&["AAA"])).is_empty());
    }

    #[test]
    fn fundamentals_wanted_via_require_or_data_leaves() {
        let price_spec = serde_json::json!({"op":"Gt","lhs":{"op":"Data","name":"close"},"rhs":{"op":"Const","value":1.0}});
        assert!(!wants_fundamentals(&price_spec, None));
        assert!(!wants_fundamentals(&price_spec, Some(&syms(&["close"]))));
        // `require` names a fundamental.
        assert!(wants_fundamentals(
            &price_spec,
            Some(&syms(&["close", "pe"]))
        ));
        // The strategy itself references one.
        let pe_spec = serde_json::json!({"op":"IsSmallest","of":{"op":"Data","name":"pe"},"n":1});
        assert!(wants_fundamentals(&pe_spec, None));
    }

    #[test]
    fn syncs_only_the_missing_symbols_into_the_tree() {
        let dir = tmp("fetch");
        let http = MockHttp {
            routes: vec![("symbol=BBB".into(), BBB_PRICES.into())],
            hits: RefCell::new(0),
        };
        let req = SyncRequest {
            source: DataSource::Fmp,
            missing: &syms(&["BBB"]),
            from: 20240102,
            to: 99991231, // clamped internally to a real calendar date
            include_fundamentals: false,
            root: &dir,
        };
        let note = sync_missing(&req, &http, "KEY").unwrap();
        assert!(note.contains("1 symbols"), "{note}");
        assert!(dir.join("prices/BBB.csv.gz").exists());
        assert!(*http.hits.borrow() > 0);
    }

    #[test]
    fn a_failed_symbol_is_an_error_not_a_partial_silent_success() {
        let dir = tmp("fail");
        let http = MockHttp {
            routes: vec![], // every URL 404s
            hits: RefCell::new(0),
        };
        let req = SyncRequest {
            source: DataSource::Fmp,
            missing: &syms(&["ZZZ"]),
            from: 20240102,
            to: 20240104,
            include_fundamentals: false,
            root: &dir,
        };
        let err = sync_missing(&req, &http, "KEY").unwrap_err();
        assert!(err.contains("ZZZ"), "{err}");
    }
}
