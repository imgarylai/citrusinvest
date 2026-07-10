//! Bring-your-own-key FMP ([Financial Modeling Prep](https://site.financialmodelingprep.com/))
//! data sync for `yuzu-cli` (issue #52).
//!
//! Direct HTTP, **no third-party FMP SDK**. Given the user's own API key, fetch
//! adjusted daily bars (and optionally annual fundamentals + a `symbol → sector`
//! industry map) and write a local tree that matches
//! [`docs/data-layout.md`](../../../docs/data-layout.md):
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (always)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (--include-fundamentals)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (--include-industry)
//! ```
//!
//! The key never leaves the machine; we neither host nor redistribute FMP data.
//! FMP lives **only** here in the CLI — never in `yuzu-core`, `yuzu-data`, or WASM
//! (the [`HttpClient`] indirection keeps the networking optional and testable).
//!
//! ## MVP scope
//!
//! Enough to backtest **price-based** strategies over a short US window: close /
//! OHLC TA and cross-section ops on a modest symbol list. Fundamentals are
//! best-effort from the annual ratios/key-metrics/growth endpoints; richer
//! fundamentals, full-universe, and point-in-time index membership are out of
//! scope (see #53 / #125). Delisted names can be unioned into the universe with
//! `--include-delisted` for survivorship-honest backtests (#124 / #26) — see
//! [`delisted`]. Which library features an FMP Starter key can *honestly*
//! support — and which panels are missing — is documented in
//! [`docs/fmp-data-source.md`](../../../docs/fmp-data-source.md) (#51).

mod config;
mod delisted;
mod fundamentals;
mod http;
mod index;
mod industry;
mod price;
mod sync;
mod universe;
mod util;

#[cfg(test)]
mod tests;

/// FMP API root. The stable endpoints (`/stable/...`) are the current surface.
pub(crate) const FMP_BASE: &str = "https://financialmodelingprep.com";

/// Object key the industry snapshot is written under (`tracked/{name}`).
pub(crate) const INDUSTRY_KEY: &str = "tracked/universe.csv.gz";

pub use config::{SyncConfig, SyncSummary, WriteMode};
pub use delisted::{fetch_delisted, DelistedSymbol};
pub use http::{HttpClient, HttpError, UreqClient};
pub use index::{Index, IndexMembership, MEMBERSHIP_SERIES};
pub use sync::sync;
pub use universe::{
    build_symbol_list, parse_market_cap, parse_symbols_list, SymbolFilter, US_EXCHANGES,
};
