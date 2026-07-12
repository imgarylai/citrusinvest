//! Bring-your-own-key FMP ([Financial Modeling Prep](https://site.financialmodelingprep.com/))
//! data sync + snapshot-factor formulas (issue #52 / #132).
//!
//! Direct HTTP, **no third-party FMP SDK**. Given the user's own API key, fetch
//! adjusted daily bars (and optionally annual fundamentals, a `symbol → sector`
//! industry map, and snapshot-factor panels) and write a [`docs/data-layout.md`](../../../docs/data-layout.md)
//! tree:
//!
//! ```text
//! <out>/prices/{SYM}.csv.gz        adjusted OHLCV                 (always)
//! <out>/fundamentals/{SYM}.csv.gz  dense forward-filled factors   (--include-fundamentals)
//! <out>/tracked/universe.csv.gz    symbol,sector,market_cap       (--include-industry)
//! <out>/panels/{name}.csv.gz       snapshot-factor panels         (--include-snapshot-factors)
//! ```
//!
//! ## Reuse across CLI and service
//!
//! [`sync`] writes to a local path; [`sync_into`] is the storage-agnostic core
//! over any `ObjectSink` + `ObjectSource`, so the CLI and a backend service
//! produce **byte-identical** trees whether the destination is local disk or an
//! S3/R2 bucket (`pomelo-s3`'s `S3Source`). The pure [`factors`] formulas
//! are the single source of truth a Rust service links directly (and wasm/PyO3
//! bindings can expose later).
//!
//! The key never leaves the machine; we neither host nor redistribute FMP data.
//! FMP stays **out** of `yuzu-core` / `pomelo-data` / WASM — the [`HttpClient`]
//! indirection keeps networking optional (build with `--no-default-features`)
//! and testable.
//!
//! ## MVP scope
//!
//! Enough to backtest **price-based** strategies over a short US window: close /
//! OHLC TA and cross-section ops on a modest symbol list. Fundamentals are
//! best-effort from the annual ratios/key-metrics/growth endpoints (plus
//! `income-statement` for filing-date visibility, #131); richer
//! fundamentals, full-universe, and point-in-time index membership are out of
//! scope (see #53 / #125). Delisted names can be unioned into the universe with
//! `--include-delisted` for survivorship-honest backtests (#124 / #26) — see
//! [`delisted`]. Which library features an FMP Starter key can *honestly*
//! support — and which panels are missing — is documented in
//! [`docs/fmp-data-source.md`](../../../docs/fmp-data-source.md) (#51).

mod config;
mod delisted;
/// Pure, I/O-free snapshot-factor formulas — the canonical implementation the
/// CLI, a Rust service, and (later) wasm/PyO3 bindings all share.
pub mod factors;
mod fundamentals;
mod http;
mod index;
mod industry;
mod price;
mod snapshot;
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
/// The real ureq-backed client — only with the `fmp-sync` feature. A dependent
/// that supplies its own [`HttpClient`] can build with the feature off.
#[cfg(feature = "fmp-sync")]
pub use http::UreqClient;
pub use http::{HttpClient, HttpError};
pub use index::{write_index_membership, Index, IndexMembership, MEMBERSHIP_SERIES};
pub use sync::{sync, sync_into};
pub use universe::{
    build_symbol_list, parse_market_cap, parse_symbols_list, SymbolFilter, US_EXCHANGES,
};
