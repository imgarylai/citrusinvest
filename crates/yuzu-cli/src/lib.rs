//! Native batch backtest runner library. A thin layer over the engine, data, and
//! research crates: `main.rs` is the clap front end, and the actual logic lives
//! in the libraries — the multi-run research orchestration (sweep / grid /
//! walk-forward / lookahead) in `yuzu-research`, all re-exported here so callers
//! keep the `yuzu_cli::…` path.

/// The FMP data sync + snapshot-factor formulas, re-exported from the standalone
/// `pomelo-fmp` crate. Kept under the `yuzu_cli::fmp::…` path so existing callers
/// (and the CLI binary) are unchanged after the extraction.
pub use pomelo_fmp as fmp;

/// EODHD data sync (epic #192), re-exported as `yuzu_cli::eodhd::…`.
pub use pomelo_eodhd as eodhd;

/// Alpha Vantage data sync (epic #209), re-exported as `yuzu_cli::alpha_vantage::…`.
pub use pomelo_alpha_vantage as alpha_vantage;

/// `list_symbols` lives in `pomelo-data` (price-file discovery is a data-layout
/// concern shared with `pomelo-audit`); re-exported so callers are unchanged.
pub use pomelo_data::list_symbols;
/// `write_index_membership` lives in `pomelo-fmp`, next to `IndexMembership`;
/// re-exported so callers are unchanged.
pub use pomelo_fmp::write_index_membership;
/// Multi-run research orchestration, re-exported from `yuzu-research` so the CLI
/// binary and its tests keep calling `yuzu_cli::run_sweep` / `run_walkforward` /
/// `SortKey` / … unchanged.
pub use yuzu_research::*;
