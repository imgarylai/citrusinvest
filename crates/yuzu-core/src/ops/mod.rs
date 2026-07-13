//! DSL operations, grouped by family. Each submodule adds `impl Panel` methods.
//!
//! # NaN / non-finite policy (engine-wide for panel ops)
//!
//! - **Missing data** is `f64::NAN`. Boolean panels use `1.0` / `0.0` with `NaN`
//!   for unknown.
//! - **Arithmetic** (`add`/`sub`/`mul`/`div`, scalar forms) propagates `NaN` like
//!   IEEE; comparisons with `NaN` yield false (`0.0`).
//! - **Cross-section** ranks / top-n / preprocess ignore `NaN` cells; empty rows
//!   stay all-`NaN` (or zeros for boolean selectors).
//! - **Rolling TA / indicators**: warm-up rows are `NaN` until the window is full
//!   (per-op `min_periods`); windows that contain `NaN` usually yield `NaN`
//!   (see each method). Listing leading-`NaN`s are skipped where noted (RSI/EMA).
//! - **Sorts / ranks** assume finite inputs at call sites that filter first;
//!   kernels should not panic on edge inputs (see `stat` property tests).

pub mod arith;
pub mod cross_section;
pub mod indicators;
pub mod linalg;
pub mod neutralize;
pub mod rebalance;
pub mod rotation;
pub mod signals;
pub mod stat;
pub mod ta;
