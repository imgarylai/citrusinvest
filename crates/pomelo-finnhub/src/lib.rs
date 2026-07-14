//! Bring-your-own-key [Finnhub](https://finnhub.io/) data sync for citrusquant
//! (epic [#210](https://github.com/citrusquant/citrusquant/issues/210)).
//!
//! Direct HTTP, **no third-party Finnhub SDK**. Given the user's own API key,
//! fetch market data and write a
//! [`docs/data-layout.md`](../../../docs/data-layout.md) tree — the same
//! contract as `pomelo-fmp` / `pomelo-eodhd` / `pomelo-alpha-vantage`:
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (#226)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (later #228)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (#227)
//! <out>/panels/{name}.csv.gz       membership / snapshot panels   (later #229–#230)
//! ```
//!
//! ## Reuse across CLI and service
//!
//! [`sync`] writes to a local path; [`sync_into`] is the storage-agnostic core
//! over any `ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`).
//!
//! The key never leaves the machine; we neither host nor redistribute Finnhub
//! data. Finnhub stays **out** of `yuzu-core` / `pomelo-data` / WASM.
//!
//! ## Status
//!
//! - **Skeleton (#225):** crate + `HttpClient` + CLI `finnhub-sync`.
//! - **Prices (#226):** `/stock/candle` (`resolution=D`, `adjusted=true`) →
//!   `prices/{SYM}.csv.gz`; resume/append modes. Adjusted OHLC map straight
//!   through (no `adj_close/close` rescale). Unadjusted risk when a plan
//!   ignores `adjusted`, plus per-request range caps, are documented honestly.
//! - **Industry (#227):** `/stock/profile2` `finnhubIndustry` + market cap →
//!   `tracked/universe.csv.gz`. **No delisted feed:** Finnhub has no clean
//!   `LISTING_STATUS`-style dead-name list, so a Finnhub-only universe is
//!   survivor-biased — documented, not faked (see `industry` module docs).
//!
//! Coverage / accepted gaps: spike
//! [#208](https://github.com/citrusquant/citrusquant/issues/208) and
//! [`docs/data-sources.md`](../../../docs/data-sources.md) § Finnhub.

mod config;
mod http;
mod industry;
mod price;
mod symbol;
mod sync;
mod util;

pub use config::{SyncConfig, SyncSummary, WriteMode};
/// The real ureq-backed client — only with the `finnhub-sync` feature.
#[cfg(feature = "finnhub-sync")]
pub use http::UreqClient;
pub use http::{HttpClient, HttpError};
pub use industry::INDUSTRY_KEY;
pub use symbol::{layout_symbol, parse_symbols_list, split_symbol};
pub use sync::{sync, sync_into, FINNHUB_BASE};

/// Default market hint (US bare tickers).
pub const DEFAULT_EXCHANGE: &str = "US";
