//! History store (SPEC §13): SQLite persistence, redaction, retention, diffing.
//!
//! The store is protocol-agnostic: it holds indexed columns plus the protocol's JSON payload.

mod db;
mod diff;
mod export;
mod model;
mod redact;
mod settings;

pub use db::{DAY_MS, Store, StoreError, data_dir, workspace_key};
pub use diff::{DiffKind, DiffRow, diff_headers, diff_json_or_text};
pub use export::{to_csv, to_json};
pub use model::{
    Counts, Entry, ExportFormat, Filter, NewEntry, SLOW_MS, Scope, Sort, SortCol, Stats, Summary,
};
pub use redact::{BODY_CAP, Redactor};
pub use settings::HistorySettings;
