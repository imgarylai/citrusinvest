use crate::error::DataError;
use crate::loader::PRICES_DIR;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Symbols with a per-symbol price file under `root/prices`, sorted and
/// de-duplicated. Recognizes `.csv.gz`, `.parquet`, and `.csv`; the loaders
/// detect the actual format from content.
pub fn list_symbols(root: &Path) -> std::io::Result<Vec<String>> {
    // `.csv.gz` before `.csv` so a gzip file isn't mis-stripped to "<sym>.csv".
    const EXTS: &[&str] = &[".csv.gz", ".parquet", ".csv"];
    let mut syms = std::collections::BTreeSet::new();
    let prices = root.join(PRICES_DIR);
    if !prices.exists() {
        return Ok(Vec::new());
    }
    for entry in fs::read_dir(prices)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if let Some(sym) = EXTS.iter().find_map(|ext| name.strip_suffix(ext)) {
                syms.insert(sym.to_string());
            }
        }
    }
    Ok(syms.into_iter().collect())
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

    #[test]
    fn list_symbols_finds_and_dedups_price_stems() {
        let dir = std::env::temp_dir().join("pomelo_data_list_symbols_test");
        let _ = fs::remove_dir_all(&dir);
        let prices = dir.join(PRICES_DIR);
        fs::create_dir_all(&prices).unwrap();
        fs::write(prices.join("AAPL.csv.gz"), b"x").unwrap();
        fs::write(prices.join("MSFT.csv"), b"x").unwrap();
        fs::write(prices.join("GOOG.parquet"), b"x").unwrap();
        fs::write(prices.join("notes.txt"), b"x").unwrap();
        assert_eq!(
            list_symbols(&dir).unwrap(),
            vec!["AAPL".to_string(), "GOOG".to_string(), "MSFT".to_string()]
        );
    }

    #[test]
    fn list_symbols_missing_prices_dir_is_empty() {
        let dir = std::env::temp_dir().join("pomelo_data_list_symbols_missing_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(list_symbols(&dir).unwrap(), Vec::<String>::new());
    }
}
