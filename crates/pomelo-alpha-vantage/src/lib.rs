//! Bring-your-own-key [Alpha Vantage](https://www.alphavantage.co/) data sync
//! for citrusquant (epic [#209](https://github.com/citrusquant/citrusquant/issues/209)).
//!
//! Direct HTTP, **no third-party Alpha Vantage SDK**. Given the user's own API
//! key, fetch market data and write a
//! [`docs/data-layout.md`](../../../docs/data-layout.md) tree — the same
//! contract as `pomelo-fmp` / `pomelo-eodhd`:
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (#214)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (later #216)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (#215)
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
//! ## Status
//!
//! - **Skeleton (#213):** crate + `HttpClient` + CLI `av-sync`.
//! - **Prices (#214):** `TIME_SERIES_DAILY_ADJUSTED` → `prices/` with adj OHLC scale.
//! - **Industry + delisted (#215):** OVERVIEW sector map; `LISTING_STATUS` delisted union.
//! - Fundies / snapshot: later phases under epic #209.
//!
//! Coverage / accepted gaps: spike
//! [#207](https://github.com/citrusquant/citrusquant/issues/207) and
//! [`docs/data-sources.md`](../../../docs/data-sources.md) § Alpha Vantage.

mod config;
mod delisted;
mod http;
mod industry;
mod price;
mod symbol;
mod sync;
mod util;

pub use config::{SyncConfig, SyncSummary, WriteMode};
pub use delisted::{fetch_delisted, DelistedSymbol};
/// The real ureq-backed client — only with the `alpha-vantage-sync` feature.
#[cfg(feature = "alpha-vantage-sync")]
pub use http::UreqClient;
pub use http::{HttpClient, HttpError};
pub use industry::INDUSTRY_KEY;
pub use symbol::{layout_symbol, parse_symbols_list, split_symbol};
pub use sync::{sync, sync_into, ALPHA_VANTAGE_BASE};

/// Default market hint when bare tickers are given (AV US equities use bare codes).
pub const DEFAULT_EXCHANGE: &str = "US";
