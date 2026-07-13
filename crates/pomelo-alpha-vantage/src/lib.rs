//! Bring-your-own-key [Alpha Vantage](https://www.alphavantage.co/) data sync
//! for citrusquant (epic [#209](https://github.com/citrusquant/citrusquant/issues/209)).
//!
//! Direct HTTP, **no third-party Alpha Vantage SDK**. Given the user's own API
//! key, fetch market data and write a
//! [`docs/data-layout.md`](../../../docs/data-layout.md) tree — the same
//! contract as `pomelo-fmp` / `pomelo-eodhd`:
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (later #214)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (later #216)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (later #215)
//! <out>/panels/{name}.csv.gz       membership / snapshot panels   (later)
//! ```
//!
//! ## Reuse across CLI and service
//!
//! [`sync`] writes to a local path; [`sync_into`] is the storage-agnostic core
//! over any `ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`).
//!
//! The key never leaves the machine; we neither host nor redistribute Alpha
//! Vantage data. AV stays **out** of `yuzu-core` / `pomelo-data` / WASM.
//!
//! ## Status (skeleton #213)
//!
//! Crate shape + CLI stub only: validates config/symbols; does **not** fetch
//! prices yet. Coverage / accepted gaps: spike
//! [#207](https://github.com/citrusquant/citrusquant/issues/207) and
//! [`docs/data-sources.md`](../../../docs/data-sources.md) § Alpha Vantage.

mod config;
mod http;
mod symbol;
mod sync;

pub use config::{SyncConfig, SyncSummary, WriteMode};
/// The real ureq-backed client — only with the `alpha-vantage-sync` feature.
#[cfg(feature = "alpha-vantage-sync")]
pub use http::UreqClient;
pub use http::{HttpClient, HttpError};
pub use symbol::{layout_symbol, parse_symbols_list, split_symbol};
pub use sync::{sync, sync_into, ALPHA_VANTAGE_BASE};

/// Default market hint when bare tickers are given (AV US equities use bare codes).
pub const DEFAULT_EXCHANGE: &str = "US";
