//! Data-loading glue: turn a synced `prices/` tree into an [`EvalContext`] the
//! engine can evaluate. Shared by every run_* entry point in this crate.

use std::collections::HashMap;
use std::path::Path;

use pomelo_data::{
    list_symbols, load_combined_panel, load_panel, Field, LocalSource, PANELS_DIR, PRICES_DIR,
};
use yuzu_core::backtest::BacktestConfig;
use yuzu_core::EvalContext;

/// OHLCV `Field` backing a price-series name usable as an execution/return
/// series (`run_backtest`'s `price_key`). Only OHLC prices qualify — `volume`
/// isn't a price you can fill at.
pub(crate) fn field_for_price_key(key: &str) -> Result<Field, String> {
    match key {
        "close" => Ok(Field::AdjClose),
        "open" => Ok(Field::AdjOpen),
        "high" => Ok(Field::AdjHigh),
        "low" => Ok(Field::AdjLow),
        other => Err(format!(
            "price-key must be one of open/high/low/close (got '{other}')"
        )),
    }
}

/// Load the close panel into an `EvalContext` — for every symbol under `root`,
/// or only for `symbols` when given (the explicit-universe path; cross-sectional
/// ops then see exactly that universe). Also loads the execution/return panel
/// named by `price_key` when it isn't `close` (so e.g. a close-signal strategy
/// can fill at the open — `--price-key open`), the volume panel when the
/// config's liquidity cap is active, and a "benchmark" panel (that symbol's
/// closes) when `cfg.benchmark_key` names a symbol that isn't already a loaded
/// panel (the benchmark does not need to be in `symbols`).
pub(crate) fn load_ctx(
    root: &Path,
    from: i32,
    to: i32,
    cfg: &BacktestConfig,
    price_key: &str,
    symbols: Option<&[String]>,
) -> Result<EvalContext, String> {
    let syms = scoped_symbols(root, symbols)?;
    let src = LocalSource::new(root);
    let mut panels = HashMap::new();
    let close = load_panel(&src, &syms, Field::AdjClose, from, to, PRICES_DIR)
        .map_err(|e| e.to_string())?;
    panels.insert("close".to_string(), close);
    // Load the execution/return price panel too when it isn't the close we
    // already have (validating the key up front for a clear error).
    let price_field = field_for_price_key(price_key)?;
    if price_key != "close" {
        let px = load_panel(&src, &syms, price_field, from, to, PRICES_DIR)
            .map_err(|e| e.to_string())?;
        panels.insert(price_key.to_string(), px);
    }
    // Execution-layer stops need OHLC (open for gap fills, high/low for the
    // touched trigger); load them when any stop is set.
    if cfg.stops.is_active() {
        for (name, field) in [
            ("open", Field::AdjOpen),
            ("high", Field::AdjHigh),
            ("low", Field::AdjLow),
        ] {
            if !panels.contains_key(name) {
                let p = load_panel(&src, &syms, field, from, to, PRICES_DIR)
                    .map_err(|e| e.to_string())?;
                panels.insert(name.to_string(), p);
            }
        }
    }
    if (cfg.max_participation > 0.0 || cfg.impact_coef > 0.0) && cfg.initial_capital > 0.0 {
        let volume = load_panel(&src, &syms, Field::Volume, from, to, PRICES_DIR)
            .map_err(|e| e.to_string())?;
        panels.insert("volume".to_string(), volume);
    }
    // Auto-load index membership panels (in_sp500, …) from panels/ when present,
    // so a strategy can `mask(signal, in_sp500)` on the CLI path. Columns absent
    // from the file become NaN (i.e. "not a member"). Missing file → skipped.
    for name in pomelo_fmp::MEMBERSHIP_SERIES {
        if panels.contains_key(*name) {
            continue;
        }
        if let Some(p) = load_combined_panel(&src, name, &syms, from, to, PANELS_DIR)
            .map_err(|e| e.to_string())?
        {
            panels.insert((*name).to_string(), p);
        }
    }
    // Auto-load snapshot-factor panels (piotroski_score, altman_z, …) from panels/
    // when present, so a factor strategy resolves them on the CLI path (the server
    // already loads these). Missing file → skipped (factor stays NaN).
    for name in pomelo_data::fundamentals::FACTOR_PANEL_FIELDS {
        if panels.contains_key(*name) {
            continue;
        }
        if let Some(p) = load_combined_panel(&src, name, &syms, from, to, PANELS_DIR)
            .map_err(|e| e.to_string())?
        {
            panels.insert((*name).to_string(), p);
        }
    }
    // The CLI treats benchmark_key as a SYMBOL: its closes are loaded as a
    // one-column panel under that key (e.g. --benchmark SPY).
    if let Some(sym) = &cfg.benchmark_key {
        if !panels.contains_key(sym) {
            let bench = load_panel(
                &src,
                std::slice::from_ref(sym),
                Field::AdjClose,
                from,
                to,
                PRICES_DIR,
            )
            .map_err(|e| e.to_string())?;
            panels.insert(sym.clone(), bench);
        }
    }
    Ok(EvalContext {
        panels,
        industry: HashMap::new(),
    })
}

/// Resolve the run's symbol universe: everything under `prices/`, or the
/// explicit `symbols` list validated against it. A requested symbol with no
/// price file is an error, never a silent drop — a quietly shrunken universe
/// changes every cross-sectional op. The list is deduplicated and sorted so a
/// reordered request loads the same panels.
fn scoped_symbols(root: &Path, symbols: Option<&[String]>) -> Result<Vec<String>, String> {
    let available = list_symbols(root).map_err(|e| e.to_string())?;
    let Some(requested) = symbols else {
        return Ok(available);
    };
    if requested.is_empty() {
        return Err("the symbols list is empty — omit it to run the full universe".into());
    }
    let have: std::collections::HashSet<&str> = available.iter().map(String::as_str).collect();
    let missing: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|s| !have.contains(s))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "symbols not in the data tree (no prices/<sym>.csv.gz): {}",
            missing.join(", ")
        ));
    }
    let mut syms: Vec<String> = requested.to_vec();
    syms.sort();
    syms.dedup();
    Ok(syms)
}
