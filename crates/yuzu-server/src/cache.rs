//! In-memory panel cache for the warm container. Loading a panel means hundreds
//! of R2 GETs (~seconds); when the user iterates on different specs over the
//! SAME universe + window, the panels are identical, so we cache them keyed by
//! (series name, date window, symbol set) and skip the reloads on repeat runs.
//!
//! Bounds: entries expire after `TTL` (so a long-lived warm container can't serve
//! data from before a nightly rebuild) and the cache holds at most `MAX_ENTRIES`
//! panels (oldest evicted first) to cap memory — a full-universe panel is tens of
//! MB. When the container sleeps (`sleepAfter`), the process dies and the cache is
//! gone, so the next wake reloads fresh.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use yuzu_core::panel::Panel;

/// Re-load a panel at most this often even on a warm container (guards against
/// serving pre-nightly-rebuild data if kept warm across a data refresh).
const TTL: Duration = Duration::from_secs(30 * 60);
/// Max cached panels. A 1756×20y panel is ~70 MB, so this bounds cache memory to
/// ~1 GB — comfortable headroom under the 4 GiB container alongside eval.
const MAX_ENTRIES: usize = 16;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    name: String,
    from: i32,
    to: i32,
    symbols: u64,
}

fn hash_symbols(symbols: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    symbols.hash(&mut h);
    h.finish()
}

#[allow(clippy::type_complexity)]
fn store() -> &'static Mutex<HashMap<Key, (Instant, Panel)>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, (Instant, Panel)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Return the cached panel for `(name, symbols, from, to)`, or run `load`, cache
/// the result, and return it. `load` is run OUTSIDE the lock (it's slow I/O).
pub fn get_or_load<F>(
    name: &str,
    symbols: &[String],
    from: i32,
    to: i32,
    load: F,
) -> Result<Panel, String>
where
    F: FnOnce() -> Result<Panel, String>,
{
    let key = Key {
        name: name.to_string(),
        from,
        to,
        symbols: hash_symbols(symbols),
    };

    {
        let cache = store().lock().unwrap();
        if let Some((at, panel)) = cache.get(&key) {
            if at.elapsed() < TTL {
                eprintln!("[yuzu] cache hit: {name}");
                return Ok(panel.clone());
            }
        }
    }

    let panel = load()?;
    let mut cache = store().lock().unwrap();
    if cache.len() >= MAX_ENTRIES && !cache.contains_key(&key) {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (Instant::now(), panel.clone()));
    Ok(panel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tiny() -> Panel {
        Panel::new(
            vec![20240102],
            vec!["X".into()],
            Array2::from_elem((1, 1), 1.0),
        )
        .unwrap()
    }

    #[test]
    fn second_call_for_same_key_hits_cache() {
        let calls = AtomicUsize::new(0);
        let syms = vec!["CACHE_TEST_UNIQUE".to_string()];
        let load = |c: &AtomicUsize| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(tiny())
        };
        let a = get_or_load("close", &syms, 1, 9, || load(&calls)).unwrap();
        let b = get_or_load("close", &syms, 1, 9, || load(&calls)).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second call must hit the cache"
        );
        assert_eq!(a.dates, b.dates);

        // A different window is a different key → miss → load runs again.
        let _ = get_or_load("close", &syms, 1, 10, || load(&calls)).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
