//! yuzu-data — native I/O layer. Reads per-symbol gzip CSV price files into
//! yuzu-core `Panel` matrices. NOT a dependency of yuzu-core (which stays
//! WASM-pure); yuzu-data depends on yuzu-core, never the reverse.

pub mod combined;
pub mod csv_io;
pub mod error;
pub mod fundamentals;
pub mod industry;
pub mod loader;
mod parallel;
pub mod source;

pub use csv_io::{Field, OhlcvRow};
pub use fundamentals::{
    is_fundamental_series, load_fundamental_panel, parse_fundamentals, write_fundamentals,
    FACTOR_PANEL_FIELDS, FundamentalRow, FUNDAMENTALS_DIR, FUNDAMENTAL_FIELDS, REPORT_EVENT_FIELD,
};
pub use combined::{
    load_combined_panel, rebuild_combined_panels, write_combined_panel, PANELS_DIR, RebuildSummary,
};
pub use loader::{load_panel, PRICES_DIR};
pub use source::{LocalSource, ObjectSink, ObjectSource};
