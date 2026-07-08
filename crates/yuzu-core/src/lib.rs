//! Pure, I/O-free evaluator for strategy specs over (dates × symbols)
//! data panels. A [`spec::Expr`] tree plus a data context evaluates to a boolean
//! position matrix ([`panel::Panel`]) — the input the backtest loop consumes.
//!
//! No network, no platform deps: f64 matrices in, `Panel` out. Compiles to both
//! native and WASM. See `docs/backtest-engine.md` for the full overview.

pub mod align;
pub mod backtest;
pub mod error;
pub mod eval;
pub mod metrics;
pub mod ops;
pub mod panel;
pub mod report;

pub use lemon::spec;

pub use eval::run_strategy;
pub use eval::run_backtest;
pub use eval::EvalContext;
