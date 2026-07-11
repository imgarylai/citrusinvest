//! Generic S3 `ObjectSource` for `pomelo-data` — reads objects from any
//! S3-compatible store (Cloudflare R2, AWS S3, MinIO, GCS-S3) over HTTPS with
//! SigV4-presigned GETs. Endpoint is configurable, so nothing here is
//! Cloudflare-specific; the container points it at R2, OSS users at anything.
//!
//! `pomelo-data` stays pure (trait + `LocalSource`); cloud access lives here.

use std::time::Duration;

use pomelo_data::error::DataError;
use pomelo_data::{ObjectSink, ObjectSource};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

/// How long a presigned GET URL stays valid — generous, single-shot use.
const SIGN_TTL: Duration = Duration::from_secs(300);

/// An `ObjectSource` backed by an S3-compatible bucket.
pub struct S3Source {
    bucket: Bucket,
    creds: Credentials,
}

impl S3Source {
    /// `endpoint` is the host root (no bucket), e.g.
    /// `https://<acct>.r2.cloudflarestorage.com`. Path-style addressing, so the
    /// bucket is appended to the path (works with custom endpoints).
    pub fn new(
        endpoint: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        region: &str,
    ) -> Result<Self, DataError> {
        let url = endpoint
            .parse()
            .map_err(|e| DataError::Io(format!("bad S3 endpoint {endpoint:?}: {e}")))?;
        let bucket = Bucket::new(url, UrlStyle::Path, bucket.to_string(), region.to_string())
            .map_err(|e| DataError::Io(format!("bad S3 bucket: {e}")))?;
        Ok(Self {
            bucket,
            creds: Credentials::new(access_key, secret_key),
        })
    }

    /// Build from `S3_ENDPOINT` / `S3_BUCKET` / `S3_ACCESS_KEY_ID` /
    /// `S3_SECRET_ACCESS_KEY` (+ optional `S3_REGION`, default `auto`).
    pub fn from_env() -> Result<Self, DataError> {
        let var = |k: &str| std::env::var(k).map_err(|_| DataError::Io(format!("missing env {k}")));
        let region = std::env::var("S3_REGION").unwrap_or_else(|_| "auto".to_string());
        Self::new(
            &var("S3_ENDPOINT")?,
            &var("S3_BUCKET")?,
            &var("S3_ACCESS_KEY_ID")?,
            &var("S3_SECRET_ACCESS_KEY")?,
            &region,
        )
    }
}

impl ObjectSource for S3Source {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError> {
        let url = self
            .bucket
            .get_object(Some(&self.creds), key)
            .sign(SIGN_TTL);
        match ureq::get(url.as_str()).call() {
            Ok(resp) => {
                // Per-symbol gzip files are small, but lift the default body cap
                // so a large universe file can't get rejected mid-load.
                let bytes = resp
                    .into_body()
                    .with_config()
                    .limit(256 * 1024 * 1024)
                    .read_to_vec()
                    .map_err(|e| DataError::Io(format!("read {key}: {e}")))?;
                Ok(Some(bytes))
            }
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(DataError::Io(format!("GET {key}: {e}"))),
        }
    }
}

impl ObjectSink for S3Source {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), DataError> {
        let url = self
            .bucket
            .put_object(Some(&self.creds), key)
            .sign(SIGN_TTL);
        match ureq::put(url.as_str()).send(bytes) {
            Ok(_) => Ok(()),
            Err(e) => Err(DataError::Io(format!("PUT {key}: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Minimal stub S3: serves "hi" for any key, 404 for keys containing
    /// "missing". Handles exactly `n` connections then exits.
    fn spawn_stub(n: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for _ in 0..n {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let read = sock.read(&mut buf).unwrap();
                let line = String::from_utf8_lossy(&buf[..read]);
                let first = line.lines().next().unwrap_or("");
                let resp = if first.contains("missing") {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi"
                        .to_string()
                };
                sock.write_all(resp.as_bytes()).unwrap();
            }
        });
        format!("http://{addr}")
    }

    /// Serves `body` once with a `Content-Encoding: gzip` header — to prove the
    /// client does NOT transparently decompress (objects are `.csv.gz` and the
    /// loaders gunzip them; auto-decompress would corrupt that).
    fn spawn_encoded_stub(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&body).unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn get_returns_bytes_and_none_on_404() {
        let endpoint = spawn_stub(2);
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", "auto").unwrap();
        assert_eq!(src.get("prices/AAPL.csv.gz").unwrap(), Some(b"hi".to_vec()));
        assert_eq!(src.get("prices/missing.csv.gz").unwrap(), None);
    }

    #[test]
    fn get_does_not_decompress_content_encoding_gzip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"hello,world\n").unwrap();
        let gz = enc.finish().unwrap();

        let endpoint = spawn_encoded_stub(gz.clone());
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", "auto").unwrap();
        let got = src.get("fundamentals/AAA.csv.gz").unwrap().unwrap();
        // Raw gzip bytes must come back verbatim (magic 0x1f 0x8b intact), NOT decompressed.
        assert_eq!(got, gz, "S3Source must return raw stored bytes");
        assert_eq!(&got[..2], &[0x1f, 0x8b]);
    }

    #[test]
    fn put_returns_ok_on_2xx() {
        let endpoint = spawn_stub(1); // existing stub: 200 for non-"missing" keys
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", "auto").unwrap();
        src.put("panels/close.csv.gz", b"gzip-bytes").unwrap();
    }

    #[test]
    fn new_rejects_a_malformed_endpoint() {
        let err = match S3Source::new("not a url", "bucket", "ak", "sk", "auto") {
            Err(e) => e,
            Ok(_) => panic!("expected a malformed-endpoint error"),
        };
        assert!(matches!(err, DataError::Io(_)));
    }

    /// A bound-then-closed port: connections are refused, so both GET and PUT hit
    /// the non-404 error arms (transport error, not a status code).
    fn dead_endpoint() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // free the port so connects are refused
        format!("http://{addr}")
    }

    #[test]
    fn get_and_put_surface_transport_errors() {
        let src = S3Source::new(&dead_endpoint(), "bucket", "ak", "sk", "auto").unwrap();
        assert!(matches!(
            src.get("prices/AAA.csv.gz"),
            Err(DataError::Io(_))
        ));
        assert!(matches!(
            src.put("panels/close.csv.gz", b"x"),
            Err(DataError::Io(_))
        ));
    }

    #[test]
    fn from_env_reads_vars_and_reports_missing() {
        // These S3_* vars are used by no other test in this crate, so mutating the
        // process environment here is safe within this test binary.
        for k in [
            "S3_ENDPOINT",
            "S3_BUCKET",
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
            "S3_REGION",
        ] {
            std::env::remove_var(k);
        }
        // Missing required var → Err naming the var.
        assert!(matches!(
            S3Source::from_env(),
            Err(DataError::Io(ref m)) if m.contains("S3_ENDPOINT")
        ));

        std::env::set_var("S3_ENDPOINT", "https://example.r2.cloudflarestorage.com");
        std::env::set_var("S3_BUCKET", "bucket");
        std::env::set_var("S3_ACCESS_KEY_ID", "ak");
        std::env::set_var("S3_SECRET_ACCESS_KEY", "sk");
        // S3_REGION left unset → defaults to "auto".
        S3Source::from_env().expect("from_env builds when all required vars are present");

        for k in [
            "S3_ENDPOINT",
            "S3_BUCKET",
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
        ] {
            std::env::remove_var(k);
        }
    }
}
