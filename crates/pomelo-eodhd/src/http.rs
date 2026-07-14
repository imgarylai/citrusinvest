//! Re-exports the shared HTTP plumbing ([`pomelo_http`]) under this crate's
//! `http` path, so vendor modules keep `use super::http::{Fetcher, HttpClient}`.
//! The [`Fetcher`] alias fixes the [`RetrySettings`](pomelo_http::RetrySettings)
//! type to this crate's [`SyncConfig`](crate::config::SyncConfig); its
//! [`get_rows`](pomelo_http::Fetcher::get_rows) covers EODHD's list-endpoint
//! error envelopes.

pub use pomelo_http::{HttpClient, HttpError};

/// The real ureq-backed client — only with the `eodhd-sync` feature.
#[cfg(feature = "eodhd-sync")]
pub use pomelo_http::UreqClient;

/// [`pomelo_http::Fetcher`] specialized to this crate's `SyncConfig`.
pub(crate) type Fetcher<'a, H> = pomelo_http::Fetcher<'a, H, crate::config::SyncConfig>;
