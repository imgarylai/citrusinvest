//! Pure, I/O-free backtest engine core for US equity strategies.
//!
//! # Pipeline
//!
//! 1. Build [`panel::Panel`]s of market / factor data (`dates × symbols`, `f64`).
//! 2. Put them in an [`EvalContext`] (optionally with a sector map).
//! 3. Evaluate a lemon JSON [`spec::Expr`] with [`run_strategy`] → position panel.
//! 4. Or run end-to-end with [`run_backtest`] → [`report::Report`] (equity, trades, metrics).
//!
//! No network and no platform deps: matrices in, report out. Compiles to native
//! and WASM. Conceptual overview: `docs/backtest-engine.md`. Strategy language:
//! `docs/lemon.md`.
//!
//! # Example
//!
//! See the crate example `basic_backtest`:
//!
//! ```text
//! cargo run -p yuzu-core --example basic_backtest
//! ```

// Numeric kernels index matrices directly; index loops read clearer than
// iterator chains here.
#![allow(clippy::needless_range_loop)]

pub mod align;
pub mod backtest;
pub mod bootstrap;
pub mod error;
pub mod eval;
pub mod metrics;
pub mod ops;
pub mod panel;
pub mod report;
pub mod research;

pub use lemon::spec;

pub use error::EngineError;
pub use eval::run_backtest;
pub use eval::run_strategy;
pub use eval::EvalContext;
