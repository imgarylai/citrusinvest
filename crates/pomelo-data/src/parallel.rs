//! Concurrent per-symbol fetch for the panel loaders. The slow part of loading a
//! panel is the per-symbol object-store GET (an HTTPS round-trip for `S3Source`);
//! done serially over ~300 symbols that's tens of seconds. We overlap the GETs on
//! a fixed-size pool so the wall-clock is the slowest few waves, not the sum.

use std::collections::HashMap;
use std::sync::OnceLock;

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::error::DataError;
use crate::format::CANDIDATE_EXTS;
use crate::source::ObjectSource;

/// Fetch a per-symbol object, probing the candidate extensions in priority order
/// (`.csv.gz` first for backward compatibility). Returns the first that exists,
/// or `None` if the symbol has no file in any supported format.
fn get_symbol<S: ObjectSource>(
    source: &S,
    dir: &str,
    sym: &str,
) -> Result<Option<Vec<u8>>, DataError> {
    for ext in CANDIDATE_EXTS {
        if let Some(bytes) = source.get(&format!("{dir}/{sym}{ext}"))? {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

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
                let mut map = HashMap::new();
                if let Some(bytes) = get_symbol(source, dir, sym)? {
                    match parse(&bytes) {
                        Ok(rows) => {
                            for (d, v) in rows {
                                if d >= from && d <= to {
                                    map.insert(d, v);
                                }
                            }
                        }
                        Err(e) => eprintln!("[pomelo-data] {dir}/{sym} parse failed: {e}"),
                    }
                }
                Ok(map)
            })
            .collect()
    })
}

/// Fetch one object per symbol concurrently (probing the candidate extensions,
/// `.csv.gz` first), returning raw bytes in input order (`None` if the symbol has
/// no file in any supported format). Used by the panel rebuild to read each
/// per-symbol source once regardless of its stored format.
pub(crate) fn fetch_symbols<S: ObjectSource + Sync>(
    source: &S,
    dir: &str,
    symbols: &[String],
) -> Result<Vec<Option<Vec<u8>>>, DataError> {
    io_pool().install(|| {
        symbols
            .par_iter()
            .map(|sym| get_symbol(source, dir, sym))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::LocalSource;
    use std::fs;

    #[test]
    fn fetch_symbols_probes_extensions_in_order_with_none_for_missing() {
        let dir = std::env::temp_dir().join("pomelo_data_fetch_symbols");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("prices")).unwrap();
        // gzip CSV for AAA, plain CSV for BBB, nothing for CCC
        fs::write(dir.join("prices/AAA.csv.gz"), b"GZ").unwrap();
        fs::write(dir.join("prices/BBB.csv"), b"CSV").unwrap();
        let src = LocalSource::new(&dir);
        let syms = vec!["AAA".to_string(), "CCC".to_string(), "BBB".to_string()];
        let got = fetch_symbols(&src, "prices", &syms).unwrap();
        assert_eq!(got, vec![Some(b"GZ".to_vec()), None, Some(b"CSV".to_vec())]);
    }
}
