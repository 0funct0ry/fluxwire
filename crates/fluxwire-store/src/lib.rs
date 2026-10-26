//! History store (SPEC §13): SQLite persistence, redaction, retention, diffing.
//!
//! The store is protocol-agnostic: it holds indexed columns plus the protocol's JSON payload.

mod db;
mod diff;
mod model;
mod redact;
mod settings;

pub use db::{Store, StoreError, data_dir, workspace_key};
pub use diff::{DiffKind, DiffRow, diff_headers, diff_json_or_text};
pub use model::{Entry, Filter, NewEntry, Scope, Summary};
pub use redact::{BODY_CAP, Redactor};
pub use settings::HistorySettings;
