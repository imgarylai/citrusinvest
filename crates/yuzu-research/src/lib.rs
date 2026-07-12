//! Multi-run backtest research over `yuzu-core`.
//!
//! `yuzu-core` runs one backtest; the *research primitives* (factor IC/ICIR,
//! event study, forward/daily returns) live in `yuzu-core::research`. This crate
//! is the layer above: it **composes** those into multi-run analyses over a
//! synced data-layout tree — parameter **sweeps**, **grids**, **walk-forward**
//! selection, and **lookahead-bias** detection — plus the data-loading glue
//! that turns a `prices/` tree into an `EvalContext`.
//!
//! It sits between the data/engine crates and the front ends: `pomelo-*` /
//! `yuzu-core` → `yuzu-research` → `yuzu-cli` (or a backend research service).
//! Everything returns serializable report structs; no CLI, no argument parsing.
//!
//! One module per analysis (plus the shared [`ctx`] loader); the public API is
//! flattened by the re-exports below.

mod ctx;
mod grid;
mod lookahead;
mod research;
mod sweep;
mod walkforward;

pub use grid::{expand_grid, GridSpec};
pub use lookahead::{
    run_lookahead, run_lookahead_profile, LookaheadLeg, LookaheadProfile, LookaheadProfilePoint,
    LookaheadReport, PROFILE_SHIFTS,
};
pub use research::{run_event, run_factor};
pub use sweep::{run_single, run_sweep, SortKey, SweepEntry};
pub use walkforward::{
    max_lookback, run_walkforward, WalkForwardParams, WalkForwardReport, WalkForwardWindow,
};
