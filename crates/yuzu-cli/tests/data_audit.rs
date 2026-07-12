//! CLI-wiring tests for `yuzu-cli data-audit` (#133). The audit *logic* is tested
//! in `pomelo-audit`; here we only exercise the binary: the human table + FAIL
//! exit code on an empty tree, and `--json` machine output on a seeded tree.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pomelo_data::csv_io::{write_series, OhlcvRow};
use pomelo_data::fundamentals::{write_fundamentals, FundamentalRow};
use pomelo_data::FUNDAMENTAL_FIELDS;

fn write_prices(dir: &Path, sym: &str, bars: &[(i32, f64)]) {
    let p = dir.join("prices");
    fs::create_dir_all(&p).unwrap();
    let rows: Vec<OhlcvRow> = bars
        .iter()
        .map(|&(day, close)| OhlcvRow {
            day,
            adj_open: close,
            adj_high: close,
            adj_low: close,
            adj_close: close,
            volume: 0.0,
        })
        .collect();
    fs::write(
        p.join(format!("{sym}.csv.gz")),
        write_series(&rows).unwrap(),
    )
    .unwrap();
}

/// A tree with a warning in several checks (gap, split, lookahead, all-NaN
/// panel, coverage) — enough to exercise the binary's `--json` path end-to-end.
fn build_tree(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yuzu_cli_audit_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    let d = [20240102, 20240103, 20240104, 20240105];

    write_prices(
        &dir,
        "GOOD",
        &[(d[0], 100.0), (d[1], 101.0), (d[2], 102.0), (d[3], 103.0)],
    );
    write_prices(&dir, "GAP", &[(d[0], 50.0), (d[1], 51.0), (d[3], 52.0)]);
    write_prices(
        &dir,
        "SPLIT",
        &[(d[0], 100.0), (d[1], 100.0), (d[2], 200.0), (d[3], 201.0)],
    );
    write_prices(&dir, "DEAD", &[(d[0], 100.0), (d[1], 101.0)]);

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

    let pdir = dir.join("panels");
    fs::create_dir_all(&pdir).unwrap();
    fs::write(
        pdir.join("piotroski_score.csv"),
        "day,GOOD\n2024-01-02,\n2024-01-03,\n",
    )
    .unwrap();

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
fn audit_empty_tree_fails_and_exits_nonzero() {
    let dir = std::env::temp_dir().join("yuzu_cli_audit_empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_yuzu-cli"))
        .args(["data-audit", "--data", dir.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(2), "empty tree must FAIL → exit 2");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("FAIL"),
        "table should show a FAIL: {stdout}"
    );
}

#[test]
fn audit_json_flag_emits_machine_report() {
    let dir = build_tree("json");
    let out = Command::new(env!("CARGO_BIN_EXE_yuzu-cli"))
        .args(["data-audit", "--data", dir.to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    assert!(out.status.success(), "warnings must still exit 0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["overall"], "WARN");
    assert_eq!(v["symbol_count"], 4);
    assert!(v["checks"].as_array().unwrap().len() >= 7);
}

/// #149: `--data s3://…` reaches an S3/R2 tree via the same `S3_*` credential
/// chain as `fmp-sync --out`. A hand-rolled stub server stands in for R2:
/// `ListObjectsV2` for discovery, `GET` for the price files, `404` for
/// everything else (a bare tree, same shape as `audit_empty_tree_...`'s local
/// sibling but priced).
#[test]
fn audit_over_s3_url_uses_the_s3_credential_chain() {
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

    let aapl = write_series(&[
        OhlcvRow {
            day: 20240102,
            adj_open: 10.0,
            adj_high: 10.0,
            adj_low: 10.0,
            adj_close: 10.0,
            volume: 0.0,
        },
        OhlcvRow {
            day: 20240103,
            adj_open: 11.0,
            adj_high: 11.0,
            adj_low: 11.0,
            adj_close: 11.0,
            volume: 0.0,
        },
    ])
    .unwrap();

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
                http_ok_bytes(list_xml(&["prices/AAPL.csv.gz"]).as_bytes())
            } else if first_line.contains("prices/AAPL.csv.gz") {
                http_ok_bytes(&aapl)
            } else if first_line.contains("prefix=fundamentals")
                || first_line.contains("prefix=panels")
            {
                http_ok_bytes(list_xml(&[]).as_bytes())
            } else {
                NOT_FOUND.to_vec()
            };
            let _ = sock.write_all(&resp);
        }
    });

    let out = Command::new(env!("CARGO_BIN_EXE_yuzu-cli"))
        .args(["data-audit", "--data", "s3://bucket", "--json"])
        .env("S3_ENDPOINT", format!("http://{addr}"))
        .env("S3_ACCESS_KEY_ID", "ak")
        .env("S3_SECRET_ACCESS_KEY", "sk")
        .env_remove("S3_SESSION_TOKEN")
        .env_remove("S3_REGION")
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["data_dir"], "s3://bucket");
    assert_eq!(v["symbol_count"], 1);
    assert_eq!(v["overall"], "OK");
}
