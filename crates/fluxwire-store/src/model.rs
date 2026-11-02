use serde_json::Value;

/// One send to record. `payload` is the protocol's `HistoryRecord.payload`.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub ts_ms: i64,
    pub collection: String,
    pub request_path: String,
    pub request_name: String,
    pub protocol: String,
    pub method: String,
    pub url: String,
    pub status: Option<i64>,
    pub total_ms: f64,
    pub env: Option<String>,
    pub dirty: bool,
    pub run_id: Option<String>,
    /// Who wrote the row: `None` for the app, `agent` for MCP sends (ADR-009).
    pub origin: Option<String>,
    pub summary: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: i64,
    pub ts_ms: i64,
    pub collection: String,
    pub request_path: String,
    pub request_name: String,
    pub protocol: String,
    pub method: String,
    pub url: String,
    pub status: Option<i64>,
    pub total_ms: f64,
    pub env: Option<String>,
    pub dirty: bool,
    pub summary: String,
    /// The folder run this send belonged to (SPEC §12).
    pub run_id: Option<String>,
    /// Kept by retention and Clear (SPEC §13).
    pub pinned: bool,
    pub note: Option<String>,
    /// `agent` for rows written through MCP, `None` for the app.
    pub origin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub summary: Summary,
    pub truncated: bool,
    pub payload: Value,
}

/// Sortable columns of the History table (SPEC §13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCol {
    #[default]
    Time,
    Method,
    Name,
    Url,
    Status,
    Duration,
    Env,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sort {
    pub col: SortCol,
    pub asc: bool,
}

/// A send over this many milliseconds counts as slow.
pub const SLOW_MS: f64 = 1000.0;

/// List filters. `status_class`: 2..=5, or 0 for sends that failed before a response.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub method: Option<String>,
    pub status_class: Option<u8>,
    pub name: Option<String>,
    /// Matches the request name OR the URL (the panel's single search box).
    pub text: Option<String>,
    pub url_text: Option<String>,
    pub request_path: Option<String>,
    pub run_id: Option<String>,
    pub env: Option<String>,
    pub protocol: Option<String>,
    pub collection: Option<String>,
    pub origin: Option<String>,
    /// Only pinned entries.
    pub pinned: bool,
    /// Only sends made from an unsaved (dirty) document.
    pub dirty: bool,
    /// Only status class 0, 4 or 5 (no response, client error, server error).
    pub failed: bool,
    /// Only sends slower than [`SLOW_MS`].
    pub slow: bool,
    /// Only sends that belong to a folder run.
    pub runs_only: bool,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub sort: Sort,
    /// Only entries with an id below this (paging, newest-first order only).
    pub before_id: Option<i64>,
    /// Rows to skip (paging for any other sort).
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

/// Aggregates over a filtered set (SPEC §13.1 stats strip).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Stats {
    pub count: i64,
    /// Share of 2xx and 3xx sends over all sends, 0.0..=1.0.
    pub success_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
}

/// Navigator counts (SPEC §13.1).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub today: i64,
    pub yesterday: i64,
    pub this_week: i64,
    pub older: i64,
    pub failures: i64,
    pub slow: i64,
    pub unsaved: i64,
    pub pinned: i64,
    pub runs: i64,
    /// `(collection, request_path, request_name, count)`, ordered by collection then path.
    pub requests: Vec<(String, String, String, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Request { collection: String, path: String },
    Collection(String),
    All,
}
