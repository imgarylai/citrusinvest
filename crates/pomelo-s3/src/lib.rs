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
    /// `session_token` is `Some` for **temporary** credentials (AWS STS / IAM
    /// role); it's signed into requests as `X-Amz-Security-Token`. R2 and other
    /// static-key stores pass `None`.
    pub fn new(
        endpoint: &str,
        bucket: &str,
        access_key: &str,
        secret_key: &str,
        session_token: Option<&str>,
        region: &str,
    ) -> Result<Self, DataError> {
        let url = endpoint
            .parse()
            .map_err(|e| DataError::Io(format!("bad S3 endpoint {endpoint:?}: {e}")))?;
        let bucket = Bucket::new(url, UrlStyle::Path, bucket.to_string(), region.to_string())
            .map_err(|e| DataError::Io(format!("bad S3 bucket: {e}")))?;
        let creds = match session_token {
            Some(t) => Credentials::new_with_token(access_key, secret_key, t),
            None => Credentials::new(access_key, secret_key),
        };
        Ok(Self { bucket, creds })
    }

    /// Build from `S3_ENDPOINT` / `S3_BUCKET` / `S3_ACCESS_KEY_ID` /
    /// `S3_SECRET_ACCESS_KEY` (+ optional `S3_SESSION_TOKEN` for temporary
    /// credentials, and `S3_REGION`, default `auto`).
    pub fn from_env() -> Result<Self, DataError> {
        let var = |k: &str| std::env::var(k).map_err(|_| DataError::Io(format!("missing env {k}")));
        let region = std::env::var("S3_REGION").unwrap_or_else(|_| "auto".to_string());
        let token = std::env::var("S3_SESSION_TOKEN").ok();
        Self::new(
            &var("S3_ENDPOINT")?,
            &var("S3_BUCKET")?,
            &var("S3_ACCESS_KEY_ID")?,
            &var("S3_SECRET_ACCESS_KEY")?,
            token.as_deref(),
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

/// Resolved S3/R2 connection details from the environment.
pub struct S3Conn {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    /// `Some` for temporary (AWS STS / IAM role) credentials.
    pub session_token: Option<String>,
    pub region: String,
}

/// Resolve S3/R2 credentials + endpoint from the environment, trying the `S3_`
/// prefix first (Cloudflare R2 / static keys) then `AWS_` (AWS S3, incl. IAM-role
/// temporary credentials via `AWS_SESSION_TOKEN`). Per prefix `P` it reads
/// `{P}ACCESS_KEY_ID`, `{P}SECRET_ACCESS_KEY`, optional `{P}SESSION_TOKEN`,
/// `{P}ENDPOINT`/`{P}ENDPOINT_URL` (AWS: derived from `{P}REGION` if unset), and
/// `{P}REGION` (default `auto`). `get` is injected so the chain is unit-testable.
/// A different deployment prefix should be mapped onto `S3_*`/`AWS_*` in the env.
pub fn resolve_s3_conn(get: impl Fn(&str) -> Option<String>) -> Result<S3Conn, String> {
    for p in ["S3_", "AWS_"] {
        let Some(access_key) = get(&format!("{p}ACCESS_KEY_ID")) else {
            continue;
        };
        let secret_key = get(&format!("{p}SECRET_ACCESS_KEY")).ok_or_else(|| {
            format!("{p}ACCESS_KEY_ID is set but {p}SECRET_ACCESS_KEY is missing")
        })?;
        let session_token = get(&format!("{p}SESSION_TOKEN"));
        let region = get(&format!("{p}REGION")).unwrap_or_else(|| "auto".to_string());
        let endpoint = get(&format!("{p}ENDPOINT"))
            .or_else(|| get(&format!("{p}ENDPOINT_URL")))
            .or_else(|| {
                // AWS with a real region but no explicit endpoint → derive it.
                (p == "AWS_" && region != "auto")
                    .then(|| format!("https://s3.{region}.amazonaws.com"))
            })
            .ok_or_else(|| {
                format!("set {p}ENDPOINT (R2: https://<acct>.r2.cloudflarestorage.com) or {p}REGION (AWS)")
            })?;
        return Ok(S3Conn {
            endpoint,
            access_key,
            secret_key,
            session_token,
            region,
        });
    }
    Err("no S3 credentials in env: set S3_ACCESS_KEY_ID + S3_SECRET_ACCESS_KEY (+ S3_ENDPOINT) for R2, or AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY (+ AWS_SESSION_TOKEN) for an AWS IAM role".into())
}

/// The destination for an `fmp-sync` run: a local path or an S3/R2 bucket. Both
/// implement [`ObjectSink`]/[`ObjectSource`], so `sync_into` writes a
/// byte-identical `data-layout.md` tree either way (the S3 variant prepends an
/// optional key prefix from the URL path).
pub enum OutStore {
    Local(pomelo_data::LocalSource),
    // Box the S3 client: it's much larger than the Local variant (clippy::large_enum_variant).
    S3 { src: Box<S3Source>, prefix: String },
}

impl OutStore {
    /// Parse `--out`: an `s3://bucket[/prefix]` URL builds an [`S3Source`]
    /// from the standard `$S3_*` env credentials; anything else is a local path.
    pub fn parse(out: &str) -> Result<Self, String> {
        let Some(rest) = out.strip_prefix("s3://") else {
            return Ok(OutStore::Local(pomelo_data::LocalSource::new(out)));
        };
        let (bucket, prefix) = match rest.split_once('/') {
            Some((b, p)) => (b, p.trim_matches('/')),
            None => (rest, ""),
        };
        if bucket.is_empty() {
            return Err("s3:// URL needs a bucket: s3://bucket[/prefix]".into());
        }
        let conn = resolve_s3_conn(|k| std::env::var(k).ok())?;
        let src = S3Source::new(
            &conn.endpoint,
            bucket,
            &conn.access_key,
            &conn.secret_key,
            conn.session_token.as_deref(),
            &conn.region,
        )
        .map_err(|e| e.to_string())?;
        Ok(OutStore::S3 {
            src: Box::new(src),
            prefix: prefix.to_string(),
        })
    }

    pub fn is_s3(&self) -> bool {
        matches!(self, OutStore::S3 { .. })
    }

    /// Prepend the S3 key prefix (if any) to a data-layout key.
    fn prefixed(prefix: &str, key: &str) -> String {
        if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}/{key}")
        }
    }
}

impl ObjectSource for OutStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError> {
        match self {
            OutStore::Local(l) => l.get(key),
            OutStore::S3 { src, prefix } => src.get(&OutStore::prefixed(prefix, key)),
        }
    }
}

impl ObjectSink for OutStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), DataError> {
        match self {
            OutStore::Local(l) => l.put(key, bytes),
            OutStore::S3 { src, prefix } => src.put(&OutStore::prefixed(prefix, key), bytes),
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
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", None, "auto").unwrap();
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
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", None, "auto").unwrap();
        let got = src.get("fundamentals/AAA.csv.gz").unwrap().unwrap();
        // Raw gzip bytes must come back verbatim (magic 0x1f 0x8b intact), NOT decompressed.
        assert_eq!(got, gz, "S3Source must return raw stored bytes");
        assert_eq!(&got[..2], &[0x1f, 0x8b]);
    }

    #[test]
    fn put_returns_ok_on_2xx() {
        let endpoint = spawn_stub(1); // existing stub: 200 for non-"missing" keys
        let src = S3Source::new(&endpoint, "bucket", "ak", "sk", None, "auto").unwrap();
        src.put("panels/close.csv.gz", b"gzip-bytes").unwrap();
    }

    #[test]
    fn new_rejects_a_malformed_endpoint() {
        let err = match S3Source::new("not a url", "bucket", "ak", "sk", None, "auto") {
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
        let src = S3Source::new(&dead_endpoint(), "bucket", "ak", "sk", None, "auto").unwrap();
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

    #[test]
    fn parse_local_vs_s3_and_key_prefixing() {
        // Non-s3 → local path (no env needed).
        assert!(!OutStore::parse("./mydata").unwrap().is_s3());
        assert!(!OutStore::parse("/tmp/x").unwrap().is_s3());
        // Key prefixing: empty prefix is a passthrough; a prefix joins with '/'.
        assert_eq!(
            OutStore::prefixed("", "prices/AAPL.csv.gz"),
            "prices/AAPL.csv.gz"
        );
        assert_eq!(
            OutStore::prefixed("mirror/v1", "panels/piotroski_score.csv.gz"),
            "mirror/v1/panels/piotroski_score.csv.gz"
        );
    }

    /// Build an env lookup over a fixed table (no real env — deterministic under
    /// parallel tests).
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| owned.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn resolve_s3_conn_prefers_s3_then_falls_back_to_aws() {
        // S3_ wins when present (R2 static keys, explicit endpoint, no token).
        let c = resolve_s3_conn(env_of(&[
            ("S3_ENDPOINT", "https://acct.r2.cloudflarestorage.com"),
            ("S3_ACCESS_KEY_ID", "r2ak"),
            ("S3_SECRET_ACCESS_KEY", "r2sk"),
            ("AWS_ACCESS_KEY_ID", "awsak"),
            ("AWS_SECRET_ACCESS_KEY", "awssk"),
        ]))
        .unwrap();
        assert_eq!(c.access_key, "r2ak");
        assert_eq!(c.endpoint, "https://acct.r2.cloudflarestorage.com");
        assert_eq!(c.region, "auto");
        assert!(c.session_token.is_none());

        // AWS_ fallback: IAM temp creds (session token) + endpoint derived from region.
        let c = resolve_s3_conn(env_of(&[
            ("AWS_ACCESS_KEY_ID", "ASIAEXAMPLE"),
            ("AWS_SECRET_ACCESS_KEY", "sk"),
            ("AWS_SESSION_TOKEN", "tok"),
            ("AWS_REGION", "us-east-1"),
        ]))
        .unwrap();
        assert_eq!(c.access_key, "ASIAEXAMPLE");
        assert_eq!(c.session_token.as_deref(), Some("tok"));
        assert_eq!(c.endpoint, "https://s3.us-east-1.amazonaws.com");

        // Access key without its secret → a clear error.
        assert!(resolve_s3_conn(env_of(&[("S3_ACCESS_KEY_ID", "x")])).is_err());
        // Nothing set → error.
        assert!(resolve_s3_conn(env_of(&[])).is_err());
    }
}
