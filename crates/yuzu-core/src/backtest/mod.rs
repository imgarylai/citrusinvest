//! Daily-equity NAV loop: turns a position-weight matrix + price panel into an
//! equity curve and a trade list. See `docs/backtest-engine.md` for the model.

// Numeric kernels index matrices directly; index loops read clearer than
// iterator chains here. (Also allowed crate-wide in lib.rs.)

mod config;
mod cost;
mod delist;
mod nav;
mod stops;
mod trade;
mod weights;

#[cfg(test)]
mod tests;

pub use config::{BacktestConfig, StopConfig, StopFill};
pub use nav::{run, run_nav, run_with_initial, BacktestRun, NavInputs};
pub use trade::{Trade, TradeSide};

#[cfg(test)]
pub(crate) use weights::{cap_weights_by_liquidity, cap_weights_row, normalize_weights_row};
