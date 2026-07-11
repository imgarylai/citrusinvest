use crate::error::DataError;
use std::fs;
use std::path::PathBuf;

/// Read-only byte store. `Ok(None)` means the key is absent (fail-soft).
pub trait ObjectSource {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError>;
}

/// Files under a root directory; `key` is a relative path (e.g. "prices/AAPL.csv.gz").
pub struct LocalSource {
    root: PathBuf,
}

impl LocalSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        LocalSource { root: root.into() }
    }
}

impl ObjectSource for LocalSource {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, DataError> {
        match fs::read(self.root.join(key)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(DataError::Io(e.to_string())),
        }
    }
}

/// Write-side counterpart to [`ObjectSource`]. Kept separate so the read path
/// (and OSS consumers) stay write-free; only the panel rebuild needs this.
pub trait ObjectSink {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), DataError>;
}

impl ObjectSink for LocalSource {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), DataError> {
        let path = self.root.join(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| DataError::Io(e.to_string()))?;
        }
        fs::write(path, bytes).map_err(|e| DataError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn local_source_reads_present_and_missing() {
        let dir = std::env::temp_dir().join("pomelo_data_source_test");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hello.bin"), b"hi").unwrap();
        let src = LocalSource::new(&dir);
        assert_eq!(src.get("hello.bin").unwrap(), Some(b"hi".to_vec()));
        assert_eq!(src.get("nope.bin").unwrap(), None);
    }

    #[test]
    fn local_source_put_writes_and_creates_parents() {
        let dir = std::env::temp_dir().join("pomelo_data_sink_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = LocalSource::new(&dir);
        // a key with a missing parent dir ("panels/") must still write
        src.put("panels/close.csv.gz", b"data").unwrap();
        assert_eq!(
            src.get("panels/close.csv.gz").unwrap(),
            Some(b"data".to_vec())
        );
    }
}
