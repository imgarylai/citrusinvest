//! `symbols_hint` / `#! index:` resolution — a named index into a
//! point-in-time universe over the local membership panel.
//!
//! `symbols_hint: "sp500"` means "run this strategy on the S&P 500":
//!
//! - **Cross-section = ever-members in the window.** The run is scoped to the
//!   symbols that were in the index at any point over `[from, to]` (the columns
//!   of `panels/in_sp500` with at least one membership day). Cross-sectional
//!   ops (`rank`, `is_largest`, `zscore`) therefore rank across those names.
//! - **Holdings = day-members, flattened on exit.** The strategy `S` is wrapped
//!   as `S * (in_sp500 >= 0.5)` so a name is held only on days it was a member
//!   and its position drops to an explicit `0.0` the day it leaves.
//!
//! Why multiply, not `mask`: the NAV loop forward-fills a NaN position (it
//! means "no new instruction, hold"), so `mask(S, in_sp500)` — which NaNs out
//! non-members — would keep *holding* a name after it leaves the index. `S *
//! (in_sp500 >= 0.5)` resolves a non-member to `0.0` instead, which the NAV
//! loop flattens. The `>= 0.5` also coerces any panel gap (NaN) to `0.0`.
//!
//! For a per-day-exact cross-section (rank only among that day's members, not
//! all ever-members), mask the ranking input yourself:
//! `is_largest(rank(mask(-pe, in_sp500)), 30)` — an escape hatch this keeps open.

use std::path::Path;

use pomelo_data::{list_symbols, load_combined_panel, LocalSource, PANELS_DIR};
use pomelo_fmp::Index;

/// A resolved index universe: the ever-in-window member symbols (the run's
/// cross-section) and the membership series name (`in_sp500`) used to mask
/// holdings down to the day's members.
pub(crate) struct IndexUniverse {
    pub symbols: Vec<String>,
    pub series: &'static str,
}

/// Resolve a `symbols_hint` (`"sp500"` / `"nasdaq"` / `"dowjones"`) against the
/// data tree's membership panel. Errors if the hint is unknown or the panel is
/// absent — never silently falls back to "all symbols" (a wrong universe is a
/// correctness bug, not a convenience).
pub(crate) fn resolve(
    root: &Path,
    hint: &str,
    from: i32,
    to: i32,
) -> Result<IndexUniverse, String> {
    let index = Index::parse(hint)
        .ok_or_else(|| format!("unknown index `{hint}` (supported: sp500, nasdaq, dowjones)"))?;
    let series = index.series_name();
    let src = LocalSource::new(root);
    let all = list_symbols(root).map_err(|e| e.to_string())?;
    let panel = load_combined_panel(&src, series, &all, from, to, PANELS_DIR)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "no `panels/{series}.csv.gz` in the data tree — build it with `yuzu-cli fmp-sync --index {hint} --out <dir>` (see docs/data-layout.md §8 point-in-time membership)"
            )
        })?;
    // Ever-member in window: a column with at least one membership day (1.0).
    let members: Vec<String> = panel
        .symbols
        .iter()
        .enumerate()
        .filter(|(c, _)| (0..panel.dates.len()).any(|r| panel.data[[r, *c]] == 1.0))
        .map(|(_, s)| s.clone())
        .collect();
    if members.is_empty() {
        return Err(format!(
            "`panels/{series}.csv.gz` has no members in {from}..{to} — check the window or rebuild the panel"
        ));
    }
    Ok(IndexUniverse {
        symbols: members,
        series,
    })
}

/// Wrap a strategy spec to hold only day-members and flatten on exit:
/// `S * (in_<index> >= 0.5)`. See the module docs for why multiply (not
/// `mask`). Field names (`l`/`r`, `Const{value}`) match `lemon-lang`'s `Expr`.
pub(crate) fn hold_mask(spec: serde_json::Value, series: &str) -> serde_json::Value {
    serde_json::json!({
        "op": "Mul",
        "l": spec,
        "r": {
            "op": "Ge",
            "l": { "op": "Data", "name": series },
            "r": { "op": "Const", "value": 0.5 }
        }
    })
}
