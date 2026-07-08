//! Concurrent per-symbol fetch for the panel loaders. The slow part of loading a
//! panel is the per-symbol object-store GET (an HTTPS round-trip for `S3Source`);
//! done serially over ~300 symbols that's tens of seconds. We overlap the GETs on
//! a fixed-size pool so the wall-clock is the slowest few waves, not the sum.

use std::collections::HashMap;
use std::sync::OnceLock;

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::error::DataError;
use crate::source::ObjectSource;

/// Shared pool for overlapping object-store GETs. Sized for I/O concurrency (many
/// in-flight network reads), NOT CPU count — the fetches are network-bound, so a
/// fixed width stays effective even on a fractional-vCPU container.
fn io_pool() -> &'static ThreadPool {
    static POOL: OnceLock<ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        ThreadPoolBuilder::new()
            .num_threads(16)
            .thread_name(|i| format!("yuzu-io-{i}"))
            .build()
            .expect("build io pool")
    })
}

/// Fetch `{dir}/{sym}.csv.gz` for every symbol concurrently, parse each with
/// `parse`, and keep `(day, value)` rows within `[from, to]`. Returns one map per
/// symbol **in input order** (so column index matches `symbols`). A missing file
/// yields an empty map; a parse failure is logged and yields an empty map (NaN
/// column) rather than sinking the batch; a fetch error aborts the whole load.
pub(crate) fn fetch_series<S, F>(
    source: &S,
    symbols: &[String],
    dir: &str,
    from: i32,
    to: i32,
    parse: F,
) -> Result<Vec<HashMap<i32, f64>>, DataError>
where
    S: ObjectSource + Sync,
    F: Fn(&[u8]) -> Result<Vec<(i32, f64)>, DataError> + Sync,
{
    io_pool().install(|| {
        symbols
            .par_iter()
            .map(|sym| {
                let key = format!("{dir}/{sym}.csv.gz");
                let mut map = HashMap::new();
                if let Some(bytes) = source.get(&key)? {
                    match parse(&bytes) {
                        Ok(rows) => {
                            for (d, v) in rows {
                                if d >= from && d <= to {
                                    map.insert(d, v);
                                }
                            }
                        }
                        Err(e) => eprintln!("[yuzu-data] {key} parse failed: {e}"),
                    }
                }
                Ok(map)
            })
            .collect()
    })
}

/// Fetch every key concurrently, returning raw bytes in input order (`None` for an
/// absent key). Lets a caller read each object once and parse it many times.
pub(crate) fn fetch_raw<S: ObjectSource + Sync>(
    source: &S,
    keys: &[String],
) -> Result<Vec<Option<Vec<u8>>>, DataError> {
    io_pool().install(|| keys.par_iter().map(|k| source.get(k)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::LocalSource;
    use std::fs;

    #[test]
    fn fetch_raw_returns_bytes_in_order_with_none_for_missing() {
        let dir = std::env::temp_dir().join("yuzu_data_fetch_raw");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.bin"), b"AA").unwrap();
        fs::write(dir.join("b.bin"), b"BB").unwrap();
        let src = LocalSource::new(&dir);
        let keys = vec![
            "a.bin".to_string(),
            "missing.bin".to_string(),
            "b.bin".to_string(),
        ];
        let got = fetch_raw(&src, &keys).unwrap();
        assert_eq!(got, vec![Some(b"AA".to_vec()), None, Some(b"BB".to_vec())]);
    }
}
