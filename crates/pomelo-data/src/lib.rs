//! pomelo-data — native I/O layer. Reads per-symbol price files into yuzu-core
//! `Panel` matrices. Formats are detected by content: gzip CSV (`.csv.gz`),
//! plain CSV (`.csv`), and — with the `parquet` feature — Apache Parquet
//! (`.parquet`). NOT a dependency of yuzu-core (which stays WASM-pure);
//! pomelo-data depends on yuzu-core, never the reverse.

pub mod combined;
pub mod csv_io;
mod date;
pub mod error;
pub mod format;
pub mod fundamentals;
pub mod industry;
pub mod loader;
mod parallel;
#[cfg(feature = "parquet")]
mod parquet_io;
pub mod source;

pub use combined::{
    assemble, load_combined_panel, rebuild_combined_panels, write_combined_panel, RebuildSummary,
    PANELS_DIR,
};
pub use csv_io::{Field, OhlcvRow};
pub use format::Format;
pub use fundamentals::{
    is_fundamental_series, load_fundamental_panel, parse_fundamentals, write_fundamentals,
    FundamentalRow, FACTOR_PANEL_FIELDS, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS, REPORT_EVENT_FIELD,
};
pub use loader::{load_panel, PRICES_DIR};
pub use source::{list_symbols, LocalSource, ObjectSink, ObjectSource};
