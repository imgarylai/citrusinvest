//! Read-only data-quality audit of a pomelo data-layout tree (#133).
//!
//! Given a synced `prices/` / `fundamentals/` / `panels/` / `tracked/` tree,
//! [`run_data_audit`] answers *"is this clean enough to trust a backtest?"* —
//! turning "high-quality data" from a claim into a measurement. It also
//! doubles as the verification tool for #131 (filing-date lag) and #132
//! (snapshot-factor coverage).
//!
//! Storage-agnostic (#149): `run_data_audit` takes any [`ObjectSource`] +
//! [`ObjectLister`], so it audits a local tree or an S3/R2 tree identically —
//! `yuzu-cli data-audit --data s3://…` builds a `pomelo_s3::OutStore` and
//! passes it straight in. Discovery (which symbols / fundamentals files /
//! membership panels exist) goes through `ObjectLister::list`; reads go
//! through `ObjectSource::get`. Over S3 this means shallow checks (coverage,
//! membership) cost a handful of `ListObjectsV2` calls, while deep checks
//! (gaps / jumps / NaN / lookahead) GET every object — auditing a full R2 tree
//! deeply is therefore comparable in cost to downloading it (see
//! `docs/fmp-data-source.md`).
//!
//! No network beyond the source's own reads, no engine run. It reuses the
//! [`pomelo_data`] loaders and returns a serializable [`DataAuditReport`] of
//! per-check `OK` / `WARN` / `FAIL` verdicts, so any front end —
//! `yuzu-cli data-audit`, a nightly job, or a backend service — can call the
//! same logic. The CLI is a thin shim that renders/emits the report and maps a
//! `FAIL` to a non-zero exit.
//!
//! ## Module layout
//!
//! - [`report`] — the [`Status`] / [`Check`] / [`DataAuditReport`] shapes,
//!   [`render_table`], and the shared formatting/date helpers.
//! - [`checks`] — the individual `check_*` data-quality checks.
//! - [`scan`] — tree discovery and the single-pass fundamentals scan.
//! - this module — `run_data_audit` orchestration + the public re-exports.

mod checks;
mod report;
mod scan;

pub use report::{render_table, Check, DataAuditReport, Status};

use pomelo_data::{load_panel, Field, ObjectLister, ObjectSource, PRICES_DIR};

use checks::{
    check_adjustment, check_calendar_gaps, check_coverage, check_index_membership,
    check_nan_density, check_pit_lag, check_survivorship,
};
use scan::{list_price_symbols, scan_fundamentals};

/// Run every check over the data-layout tree served by `src`, windowed to
/// `[from, to]`. `data_dir` is a display label only (a local path or an
/// `s3://…` URL) — it never drives I/O; all discovery goes through `src`.
/// Fail-soft: a missing directory or file downgrades a check, never panics.
pub fn run_data_audit<S: ObjectSource + ObjectLister + Sync>(
    src: &S,
    data_dir: &str,
    from: i32,
    to: i32,
) -> Result<DataAuditReport, String> {
    let symbols = list_price_symbols(src);

    // One adj-close Panel (union calendar × symbols, NaN where absent) backs the
    // coverage / gaps / delist / jump checks.
    let closes = if symbols.is_empty() {
        None
    } else {
        Some(
            load_panel(src, &symbols, Field::AdjClose, from, to, PRICES_DIR)
                .map_err(|e| format!("loading price panel: {e}"))?,
        )
    };

    // Fundamentals are parsed once (per-field coverage + filing-event days).
    let fund = scan_fundamentals(src, from, to);

    let checks = vec![
        check_coverage(src, &symbols, closes.as_ref()),
        check_calendar_gaps(&symbols, closes.as_ref()),
        check_adjustment(&symbols, closes.as_ref()),
        check_survivorship(&symbols, closes.as_ref()),
        check_nan_density(src, &symbols, &fund, from, to),
        check_pit_lag(&fund),
        check_index_membership(src, &symbols, from, to),
    ];

    let overall = checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok);
    Ok(DataAuditReport {
        data_dir: data_dir.to_string(),
        from,
        to,
        symbol_count: symbols.len(),
        overall,
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::status_str;
    use pomelo_data::csv_io::{write_series, OhlcvRow};
    use pomelo_data::fundamentals::{write_fundamentals, FundamentalRow};
    use pomelo_data::LocalSource;
    use pomelo_data::FUNDAMENTAL_FIELDS;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Run the audit over a local tree, matching the CLI's local-path path.
    fn run_local(dir: &Path, from: i32, to: i32) -> DataAuditReport {
        let src = LocalSource::new(dir);
        run_data_audit(&src, &dir.display().to_string(), from, to).unwrap()
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pomelo_audit_ut_{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn bar(day: i32, close: f64) -> OhlcvRow {
        OhlcvRow {
            day,
            adj_open: close,
            adj_high: close,
            adj_low: close,
            adj_close: close,
            volume: 0.0,
        }
    }

    fn write_prices(dir: &Path, sym: &str, bars: &[(i32, f64)]) {
        let p = dir.join("prices");
        fs::create_dir_all(&p).unwrap();
        let rows: Vec<OhlcvRow> = bars.iter().map(|&(d, c)| bar(d, c)).collect();
        fs::write(
            p.join(format!("{sym}.csv.gz")),
            write_series(&rows).unwrap(),
        )
        .unwrap();
    }

    fn find<'a>(r: &'a DataAuditReport, name: &str) -> &'a Check {
        r.checks.iter().find(|c| c.name == name).unwrap()
    }

    /// A tree that trips one specific check each — the full WARN sweep.
    fn build_rich_tree(tag: &str) -> PathBuf {
        let dir = tmp(tag);
        let d = [20240102, 20240103, 20240104, 20240105];
        write_prices(
            &dir,
            "GOOD",
            &[(d[0], 100.0), (d[1], 101.0), (d[2], 102.0), (d[3], 103.0)],
        );
        write_prices(&dir, "GAP", &[(d[0], 50.0), (d[1], 51.0), (d[3], 52.0)]); // 2024-01-04 missing
        write_prices(
            &dir,
            "SPLIT",
            &[(d[0], 100.0), (d[1], 100.0), (d[2], 200.0), (d[3], 201.0)],
        );
        write_prices(&dir, "DEAD", &[(d[0], 100.0), (d[1], 101.0)]); // ends early

        // Fundamentals for GOOD: `pe` populated; a filing on 2023-12-31 (a month-end).
        let fdir = dir.join("fundamentals");
        fs::create_dir_all(&fdir).unwrap();
        let mut vals = vec![f64::NAN; FUNDAMENTAL_FIELDS.len()];
        vals[0] = 15.0; // pe
        let rows = vec![
            FundamentalRow {
                day: 20231229,
                values: vals.clone(),
                report_event: 0.0,
            },
            FundamentalRow {
                day: 20231231,
                values: vals.clone(),
                report_event: 1.0,
            },
        ];
        fs::write(fdir.join("GOOD.csv.gz"), write_fundamentals(&rows).unwrap()).unwrap();

        // An all-NaN snapshot-factor panel (plain .csv — the loader probes .csv).
        let pdir = dir.join("panels");
        fs::create_dir_all(&pdir).unwrap();
        fs::write(
            pdir.join("piotroski_score.csv"),
            "day,GOOD\n2024-01-02,\n2024-01-03,\n",
        )
        .unwrap();

        // Universe map with an extra name that has no price file.
        let tdir = dir.join("tracked");
        fs::create_dir_all(&tdir).unwrap();
        fs::write(
            tdir.join("universe.csv"),
            "symbol,sector,market_cap\nGOOD,Tech,1e12\nGAP,Tech,1e11\nSPLIT,Tech,1e11\nDEAD,Tech,1e10\nMISSING,Tech,1e9\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn rich_tree_flags_each_check() {
        let dir = build_rich_tree("rich");
        let r = run_local(&dir, 20000101, 99991231);
        assert_eq!(r.symbol_count, 4);
        assert_eq!(r.overall, Status::Warn);

        let cov = find(&r, "coverage");
        assert_eq!(cov.status, Status::Warn);
        assert!(cov.details["in_universe_missing_prices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "MISSING"));

        assert_eq!(find(&r, "calendar_gaps").status, Status::Warn);
        assert_eq!(find(&r, "calendar_gaps").details["total_holes"], 1);

        assert_eq!(find(&r, "adjustment").status, Status::Warn);
        assert_eq!(find(&r, "adjustment").details["flagged"], 1);

        assert_eq!(find(&r, "survivorship").status, Status::Ok);
        assert_eq!(find(&r, "survivorship").details["ended_early"], 1);

        assert_eq!(find(&r, "nan_density").status, Status::Warn);
        assert!(find(&r, "nan_density").details["all_nan_factor_panels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "piotroski_score"));

        let pit = find(&r, "pit_lag");
        assert_eq!(pit.status, Status::Warn);
        assert_eq!(pit.details["report_events"], 1);
        assert_eq!(pit.details["on_month_end"], 1);

        assert_eq!(find(&r, "index_membership").status, Status::Ok);

        // The rendered table shows the overall WARN and the flagged checks.
        let table = render_table(&r);
        assert!(table.contains("WARN"));
        assert!(table.contains("adjustment"));
    }

    #[test]
    fn empty_tree_fails_and_takes_no_price_arms() {
        let d = tmp("empty");
        let r = run_local(&d, 20000101, 99991231);
        assert_eq!(r.overall, Status::Fail);
        assert_eq!(r.symbol_count, 0);
        assert_eq!(find(&r, "coverage").status, Status::Fail);
        for name in ["calendar_gaps", "adjustment", "survivorship"] {
            assert_eq!(find(&r, name).status, Status::Ok);
        }
        let table = render_table(&r);
        assert!(table.contains("FAIL"));
        assert!(table.contains("coverage"));
        assert_eq!(status_str(Status::Ok), "OK");
        assert_eq!(status_str(Status::Warn), "WARN");
        assert_eq!(status_str(Status::Fail), "FAIL");
    }

    #[test]
    fn prices_without_universe_or_fundamentals_are_ok() {
        let d = tmp("bare");
        write_prices(
            &d,
            "AAA",
            &[(20240102, 10.0), (20240103, 10.0), (20240104, 10.0)],
        );
        let r = run_local(&d, 20000101, 99991231);
        assert_eq!(r.overall, Status::Ok);
        let cov = find(&r, "coverage");
        assert_eq!(cov.status, Status::Ok);
        assert!(cov.summary.contains("no tracked/universe"));
        assert!(cov.details["date_range"]["first_day"] == 20240102);
        assert_eq!(find(&r, "survivorship").status, Status::Ok);
        assert_eq!(find(&r, "nan_density").status, Status::Ok);
        assert!(find(&r, "nan_density")
            .summary
            .contains("no fundamentals or factor panels"));
        assert_eq!(find(&r, "pit_lag").status, Status::Ok);
    }

    #[test]
    fn index_membership_panel_is_summarized() {
        let d = tmp("index");
        write_prices(&d, "AAA", &[(20240102, 10.0), (20240103, 10.0)]);
        write_prices(&d, "BBB", &[(20240102, 10.0), (20240103, 10.0)]);
        let pan = d.join("panels");
        fs::create_dir_all(&pan).unwrap();
        fs::write(
            pan.join("in_sp500.csv"),
            "day,AAA,BBB\n2024-01-02,1,\n2024-01-03,1,1\n",
        )
        .unwrap();
        let r = run_local(&d, 20000101, 99991231);
        let idx = find(&r, "index_membership");
        assert_eq!(idx.status, Status::Ok);
        let p0 = &idx.details["panels"][0];
        assert_eq!(p0["panel"], "in_sp500");
        assert_eq!(p0["min_members"], 1);
        assert_eq!(p0["max_members"], 2);
        assert_eq!(p0["last_members"], 2);
    }

    #[test]
    fn edge_arms_single_point_zero_close_and_empty_membership() {
        let d = tmp("edge");
        write_prices(&d, "ONE", &[(20240102, 10.0)]);
        write_prices(&d, "ZERO", &[(20240102, 0.0), (20240103, 10.0)]);
        let pan = d.join("panels");
        fs::create_dir_all(&pan).unwrap();
        fs::write(pan.join("in_empty.csv"), "day,ONE,ZERO\n2024-01-02,,\n").unwrap();

        let r = run_local(&d, 20000101, 99991231);
        // The 0 → 10 step is skipped by the c0 <= 0 guard, so nothing is flagged.
        assert_eq!(find(&r, "adjustment").details["flagged"], 0);
        assert_eq!(find(&r, "survivorship").status, Status::Ok);
        let idx = find(&r, "index_membership");
        assert_eq!(idx.status, Status::Warn);
        assert_eq!(idx.details["panels"][0]["max_members"], 0);
    }

    /// #149: `run_data_audit` is storage-agnostic — this drives it against a
    /// stub S3 endpoint (routing `ListObjectsV2` + `GET` requests by hand,
    /// mirroring `pomelo-s3`'s own stub-server tests) instead of `LocalSource`,
    /// proving discovery (via `ObjectLister::list`) and reads (via
    /// `ObjectSource::get`) both work over S3/R2.
    #[test]
    fn run_data_audit_over_a_stub_s3_source() {
        use pomelo_s3::S3Source;
        use std::io::{Read, Write};
        use std::net::TcpListener;

        fn list_xml(keys: &[&str]) -> String {
            let contents: String = keys
                .iter()
                .map(|k| {
                    format!(
                        "<Contents><Key>{k}</Key><LastModified>2020-01-01T00:00:00.000Z</LastModified>\
                         <ETag>\"e\"</ETag><Size>1</Size><StorageClass>STANDARD</StorageClass></Contents>"
                    )
                })
                .collect();
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
                 <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                 <Name>bucket</Name><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys>\
                 <IsTruncated>false</IsTruncated>{contents}<EncodingType>url</EncodingType>\
                 </ListBucketResult>",
                keys.len()
            )
        }
        fn http_ok_bytes(body: &[u8]) -> Vec<u8> {
            let mut resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            resp.extend_from_slice(body);
            resp
        }
        const NOT_FOUND: &[u8] =
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

        let aapl = write_series(&[bar(20240102, 10.0), bar(20240103, 11.0)]).unwrap();
        let msft = write_series(&[bar(20240102, 20.0), bar(20240103, 21.0)]).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { break };
                let mut buf = [0u8; 4096];
                let read = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..read]).to_string();
                let first_line = req.lines().next().unwrap_or("");
                let resp = if first_line.contains("prefix=prices") {
                    http_ok_bytes(
                        list_xml(&["prices/AAPL.csv.gz", "prices/MSFT.csv.gz"]).as_bytes(),
                    )
                } else if first_line.contains("prices/AAPL.csv.gz") {
                    http_ok_bytes(&aapl)
                } else if first_line.contains("prices/MSFT.csv.gz") {
                    http_ok_bytes(&msft)
                } else if first_line.contains("prefix=fundamentals")
                    || first_line.contains("prefix=panels")
                {
                    http_ok_bytes(list_xml(&[]).as_bytes())
                } else {
                    // Every other lookup (universe map, snapshot-factor panels) is
                    // "not present" — a bare synced tree, same as the local
                    // `prices_without_universe_or_fundamentals_are_ok` case.
                    NOT_FOUND.to_vec()
                };
                let _ = sock.write_all(&resp);
            }
        });

        let src = S3Source::new(
            &format!("http://{addr}"),
            "bucket",
            "ak",
            "sk",
            None,
            "auto",
        )
        .unwrap();
        let r = run_data_audit(&src, "s3://bucket", 20000101, 99991231).unwrap();
        assert_eq!(r.data_dir, "s3://bucket");
        assert_eq!(r.symbol_count, 2);
        assert_eq!(find(&r, "coverage").status, Status::Ok);
        assert_eq!(find(&r, "coverage").details["symbols_with_prices"], 2);
        assert_eq!(find(&r, "nan_density").status, Status::Ok);
        assert_eq!(find(&r, "index_membership").status, Status::Ok);
    }
}
