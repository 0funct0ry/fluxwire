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
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub summary: Summary,
    pub truncated: bool,
    pub payload: Value,
}

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
    /// Only entries with an id below this (paging).
    pub before_id: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum Scope {
    Request { collection: String, path: String },
    Collection(String),
    All,
}
