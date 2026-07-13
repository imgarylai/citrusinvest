//! Bring-your-own-key [EODHD](https://eodhd.com/) data sync for citrusquant
//! (epic [#192](https://github.com/citrusquant/citrusquant/issues/192)).
//!
//! Direct HTTP, **no third-party EODHD SDK**. Given the user's own API token,
//! fetch market data and write a [`docs/data-layout.md`](../../../docs/data-layout.md)
//! tree — the same contract as `pomelo-fmp`:
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (#194)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (#196)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (#195)
//! <out>/panels/{name}.csv.gz       membership / snapshot panels   (later)
//! ```
//!
//! ## Reuse across CLI and service
//!
//! [`sync`] writes to a local path; [`sync_into`] is the storage-agnostic core
//! over any `ObjectSink` + `ObjectSource` (local disk or S3/R2 via `pomelo-s3`).
//!
//! The token never leaves the machine; we neither host nor redistribute EODHD
//! data. EODHD stays **out** of `yuzu-core` / `pomelo-data` / WASM.
//!
//! ## Status
//!
//! - **Prices (#194):** EOD → full adj OHLC via `adjusted_close/close` scale.
//! - **Industry + delisted (#195):** sector map; delisted list for universe union.
//! - **Fundamentals (#196):** yearly statements → dense `FUNDAMENTAL_FIELDS` + `report_event`.
//!
//! Coverage map: [`docs/data-sources.md`](../../../docs/data-sources.md).

mod config;
mod delisted;
mod fundamentals;
mod http;
mod industry;
mod price;
mod symbol;
mod sync;
mod util;

pub use config::{SyncConfig, SyncSummary, WriteMode};
pub use delisted::{fetch_delisted, DelistedSymbol};
/// The real ureq-backed client — only with the `eodhd-sync` feature.
#[cfg(feature = "eodhd-sync")]
pub use http::UreqClient;
pub use http::{HttpClient, HttpError};
pub use industry::INDUSTRY_KEY;
pub use symbol::{layout_symbol, parse_symbols_list, split_symbol};
pub use sync::{sync, sync_into, EODHD_BASE};

/// Default US exchange code for bare tickers (`AAPL` → `AAPL.US`).
pub const DEFAULT_EXCHANGE: &str = "US";
