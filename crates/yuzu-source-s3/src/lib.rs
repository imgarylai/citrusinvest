//! Generic S3 `ObjectSource` for `yuzu-data` — reads objects from any
//! S3-compatible store (Cloudflare R2, AWS S3, MinIO, GCS-S3) over HTTPS with
//! SigV4-presigned GETs. Endpoint is configurable, so nothing here is
//! Cloudflare-specific; the container points it at R2, OSS users at anything.
//!
//! `yuzu-data` stays pure (trait + `LocalSource`); cloud access lives here.

use std::time::Duration;

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use yuzu_data::error::DataError;
use yuzu_data::{ObjectSink, ObjectSource};

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
}
