//! Thin HTTP front end for `yuzu_server::handle_backtest`. Runs in the
//! Cloudflare Container; reads panels from R2 via `S3Source::from_env()`, or
//! from a local data dir when `YUZU_DATA_DIR` is set (the OSS / smoke-test path).
//!
//!   GET  /health    → 200 "ok"
//!   POST /backtest  → body = BacktestRequest JSON → Report JSON (400 on bad
//!                     request, 500 on engine error)

use tiny_http::{Header, Method, Response, Server};
use yuzu_data::error::DataError;
use yuzu_data::{LocalSource, ObjectSink, ObjectSource};
use yuzu_server::{handle_backtest, handle_rebuild, BacktestRequest, DataDirs, RebuildRequest};
use yuzu_source_s3::S3Source;

/// The object store, chosen at startup: local disk or S3 (R2/AWS/MinIO/GCS).
enum AnySource {
    Local(LocalSource),
    S3(S3Source),
}

impl ObjectSource for AnySource {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError> {
        match self {
            AnySource::Local(s) => s.get(key),
            AnySource::S3(s) => s.get(key),
        }
    }
}

impl ObjectSink for AnySource {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), DataError> {
        match self {
            AnySource::Local(s) => s.put(key, bytes),
            AnySource::S3(s) => s.put(key, bytes),
        }
    }
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()
}

fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let dirs = DataDirs::from_env();
    let source = match std::env::var("YUZU_DATA_DIR") {
        Ok(dir) => {
            eprintln!("source: local dir {dir}");
            AnySource::Local(LocalSource::new(dir))
        }
        Err(_) => AnySource::S3(
            S3Source::from_env()
                .expect("S3 env (S3_ENDPOINT/S3_BUCKET/S3_ACCESS_KEY_ID/S3_SECRET_ACCESS_KEY)"),
        ),
    };

    let server = Server::http(("0.0.0.0", port)).expect("bind");
    eprintln!("yuzu-server listening on 0.0.0.0:{port}");

    for mut req in server.incoming_requests() {
        let is_post = *req.method() == Method::Post;
        let path = req.url().split('?').next().unwrap_or("").to_string();
        match (is_post, path.as_str()) {
            (false, "/health") => {
                let _ = req.respond(Response::from_string("ok"));
            }
            (true, "/backtest") => {
                let mut body = String::new();
                if req.as_reader().read_to_string(&mut body).is_err() {
                    let _ = req.respond(Response::from_string("bad body").with_status_code(400));
                    continue;
                }
                let resp = match serde_json::from_str::<BacktestRequest>(&body) {
                    Err(e) => Response::from_string(format!("bad request: {e}"))
                        .with_status_code(400)
                        .with_header(json_header()),
                    Ok(parsed) => match handle_backtest(&source, &parsed, &dirs) {
                        Ok(report) => Response::from_string(
                            serde_json::to_string(&report).unwrap_or_else(|e| e.to_string()),
                        )
                        .with_header(json_header()),
                        Err(e) => Response::from_string(format!("backtest error: {e}"))
                            .with_status_code(500)
                            .with_header(json_header()),
                    },
                };
                let _ = req.respond(resp);
            }
            (true, "/rebuild-panels") => {
                let mut body = String::new();
                if req.as_reader().read_to_string(&mut body).is_err() {
                    let _ = req.respond(Response::from_string("bad body").with_status_code(400));
                    continue;
                }
                let resp = match serde_json::from_str::<RebuildRequest>(&body) {
                    Err(e) => Response::from_string(format!("bad request: {e}"))
                        .with_status_code(400)
                        .with_header(json_header()),
                    Ok(parsed) => match handle_rebuild(&source, &parsed, &dirs) {
                        Ok(s) => Response::from_string(
                            serde_json::json!({ "fields": s.fields, "days": s.days }).to_string(),
                        )
                        .with_header(json_header()),
                        Err(e) => Response::from_string(format!("rebuild error: {e}"))
                            .with_status_code(500)
                            .with_header(json_header()),
                    },
                };
                let _ = req.respond(resp);
            }
            _ => {
                let _ = req.respond(Response::from_string("not found").with_status_code(404));
            }
        }
    }
}
