//! Protocol-neutral execution types (SPEC §5.5). The runner, history, CLI and UI
//! see only these; nothing here names HTTP.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use bytes::Bytes;
use fluxwire_model::{Diagnostic, Edit, ExpectBlock, FluxDoc};
use fluxwire_vars::VarScope;

use crate::{EnvState, TokenInfo};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
pub use tokio_util::sync::CancellationToken;

use crate::{Interaction, RoleSpec};

/// Responses up to this size stay in memory; larger ones stream to a temp file.
pub const MEMORY_BODY_LIMIT: u64 = 16 * 1024 * 1024;
/// Default hard cap per response (SPEC §9.2).
pub const DEFAULT_MAX_BODY: u64 = 2 * 1024 * 1024 * 1024;

/// A protocol's prepared request. The payload is private to the protocol.
#[derive(Clone)]
pub struct Plan {
    pub protocol: &'static str,
    /// Display label, e.g. `GET https://api.example.com/v1/payments`.
    pub label: String,
    /// Non-fatal findings from planning.
    pub warnings: Vec<Diagnostic>,
    payload: Arc<dyn Any + Send + Sync>,
}

impl Plan {
    pub fn new<T: Any + Send + Sync>(
        protocol: &'static str,
        label: impl Into<String>,
        payload: T,
    ) -> Self {
        Self {
            protocol,
            label: label.into(),
            warnings: Vec::new(),
            payload: Arc::new(payload),
        }
    }

    pub fn payload<T: Any>(&self) -> Option<&T> {
        self.payload.downcast_ref()
    }
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("protocol", &self.protocol)
            .field("label", &self.label)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ClientCert {
    pub host: String,
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Network settings that come from `fluxwire.local.toml` / `settings.toml`.
#[derive(Debug, Clone, Default)]
pub struct NetConfig {
    pub ca_files: Vec<PathBuf>,
    pub client_certs: Vec<ClientCert>,
    /// Explicit proxy URL; `None` falls back to the proxy environment variables.
    pub proxy: Option<String>,
    pub no_proxy: Option<String>,
}

/// Settings the collection defaults and the request's `settings` map resolve to.
#[derive(Debug, Clone)]
pub struct ExecSettings {
    pub timeout: Option<Duration>,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub verify_tls: bool,
    pub max_body: u64,
    pub memory_body_limit: u64,
}

impl Default for ExecSettings {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            follow_redirects: true,
            max_redirects: 10,
            verify_tls: true,
            max_body: DEFAULT_MAX_BODY,
            memory_body_limit: MEMORY_BODY_LIMIT,
        }
    }
}

#[derive(Clone)]
pub struct ExecCtx {
    pub cancel: CancellationToken,
    pub settings: ExecSettings,
    pub net: NetConfig,
    /// Where oversized bodies spill to; `None` uses the OS temp directory.
    pub temp_dir: Option<PathBuf>,
    /// Cookie jar and token cache of the active environment.
    pub env: Option<crate::EnvState>,
    /// Commands from the caller into a live session (SPEC §10); `None` for exchange protocols.
    pub session: Option<SessionInbox>,
}

impl Default for ExecCtx {
    fn default() -> Self {
        Self {
            cancel: CancellationToken::new(),
            settings: ExecSettings::default(),
            net: NetConfig::default(),
            temp_dir: None,
            env: None,
            session: None,
        }
    }
}

/// A command the caller sends into a live session.
#[derive(Debug, Clone)]
pub enum SessionCmd {
    /// Send a frame; `kind` is `text` or `binary`.
    Send { kind: String, data: Bytes },
    /// Close gracefully (normal closure) and finish the run.
    Close,
}

/// Caller side of a session: send frames or close. Cheap to clone.
#[derive(Debug, Clone)]
pub struct SessionHandle(tokio::sync::mpsc::UnboundedSender<SessionCmd>);

impl SessionHandle {
    /// `false` if the session has already ended.
    pub fn send(&self, kind: &str, data: impl Into<Bytes>) -> bool {
        self.0
            .send(SessionCmd::Send {
                kind: kind.into(),
                data: data.into(),
            })
            .is_ok()
    }

    pub fn close(&self) -> bool {
        self.0.send(SessionCmd::Close).is_ok()
    }
}

/// Protocol side of a session: take the command receiver once, at the start of `execute`.
#[derive(Debug, Clone)]
pub struct SessionInbox(
    Arc<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<SessionCmd>>>>,
);

impl SessionInbox {
    pub fn take(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<SessionCmd>> {
        self.0.lock().ok()?.take()
    }
}

pub fn session_channel() -> (SessionHandle, SessionInbox) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (
        SessionHandle(tx),
        SessionInbox(Arc::new(std::sync::Mutex::new(Some(rx)))),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseTiming {
    pub name: String,
    pub start_ms: f64,
    pub duration_ms: f64,
    /// A phase that did not run because a pooled connection was reused.
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RedirectHop {
    pub status: u16,
    pub from: String,
    pub to: String,
}

/// A response body: in memory, or in a temp file that is deleted when the last
/// clone is dropped.
#[derive(Debug, Clone)]
pub enum Body {
    Memory(Bytes),
    File {
        path: Arc<tempfile::TempPath>,
        len: u64,
    },
}

impl Body {
    pub fn len(&self) -> u64 {
        match self {
            Body::Memory(b) => b.len() as u64,
            Body::File { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_memory(&self) -> Option<&Bytes> {
        match self {
            Body::Memory(b) => Some(b),
            Body::File { .. } => None,
        }
    }

    /// Read at most `limit` bytes from the start.
    pub fn read_prefix(&self, limit: usize) -> std::io::Result<Vec<u8>> {
        match self {
            Body::Memory(b) => Ok(b[..b.len().min(limit)].to_vec()),
            Body::File { path, .. } => {
                use std::io::Read;
                let mut out = Vec::new();
                std::fs::File::open(&**path)?
                    .take(limit as u64)
                    .read_to_end(&mut out)?;
                Ok(out)
            }
        }
    }

    pub fn read_all(&self) -> std::io::Result<Vec<u8>> {
        match self {
            Body::Memory(b) => Ok(b.to_vec()),
            Body::File { path, .. } => std::fs::read(&**path),
        }
    }

    pub fn save_to(&self, dest: &std::path::Path) -> std::io::Result<()> {
        match self {
            Body::Memory(b) => std::fs::write(dest, b),
            Body::File { path, .. } => std::fs::copy(&**path, dest).map(|_| ()),
        }
    }
}

/// The request/response shape every exchange protocol fills in.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub status: u16,
    pub status_text: String,
    /// Protocol label shown in the status line, e.g. `HTTP/2`.
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub content_type: Option<String>,
    pub wire_size: u64,
    pub decoded_size: u64,
}

impl Exchange {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub protocol: &'static str,
    pub interaction: Interaction,
    pub final_url: String,
    pub exchange: Option<Exchange>,
    pub timings: Vec<PhaseTiming>,
    pub total_ms: f64,
    pub redirects: Vec<RedirectHop>,
    pub connection_reused: bool,
    /// Transcript of a session protocol; `None` for exchange protocols.
    pub session: Option<SessionSummary>,
}

/// Frames kept in an `Outcome` and in history (SPEC §10).
pub const TRANSCRIPT_FRAME_CAP: usize = 1000;

#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    /// The first [`TRANSCRIPT_FRAME_CAP`] frames, in order.
    pub frames: Vec<Frame>,
    /// Every frame seen, including those beyond the cap.
    pub total_frames: u64,
    pub close_code: Option<u16>,
    pub close_reason: String,
    /// Set when the session ended abnormally after it was open.
    pub error: Option<String>,
}

/// A failure that names the phase it happened in and, where known, a fix.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecError {
    pub phase: String,
    pub message: String,
    pub fix: Option<String>,
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.phase, self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " {fix}")?;
        }
        Ok(())
    }
}

/// Session-protocol frame (WebSocket and later); unused by exchange protocols.
#[derive(Debug, Clone)]
pub struct Frame {
    pub outbound: bool,
    /// `text`, `binary`, `ping`, `pong` or `close`.
    pub kind: String,
    pub data: Bytes,
    /// Milliseconds since the run started.
    pub ts_ms: f64,
    /// Close code, for `close` frames.
    pub close_code: Option<u16>,
}

impl Frame {
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A phase finished (live waterfall).
    Phase(PhaseTiming),
    Redirect(RedirectHop),
    Frame(Frame),
    /// Always the last event on success.
    Done(Box<Outcome>),
    /// Always the last event on failure or cancellation.
    Failed(ExecError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub message: Option<String>,
}

/// What `Protocol::check` needs beyond the outcome.
pub struct CheckCtx<'a> {
    /// The document the expect block was parsed from; body spans index its source.
    pub doc: Option<&'a FluxDoc>,
    /// Scope for `{{var}}` interpolation inside the block.
    pub scope: &'a VarScope,
}

/// Names a protocol exposes to scripts.
#[derive(Debug, Clone, Default)]
pub struct ScriptBindings {
    pub names: Vec<&'static str>,
}

/// What history stores for a run; secrets are redacted by the store (M5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryRecord {
    pub protocol: String,
    pub summary: String,
    pub payload: serde_json::Value,
}

/// What pasted text (e.g. a cURL command) will become, shown before a request is created.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PasteSummary {
    pub method: String,
    pub url: String,
    pub header_count: usize,
    /// Body kind (`JSON`, `form`, `text`) and size in bytes.
    pub body: Option<(String, usize)>,
    pub warnings: Vec<String>,
}

pub trait Protocol: Send + Sync {
    fn id(&self) -> &'static str;
    /// Human label for the New request dialog.
    fn label(&self) -> &'static str {
        self.id()
    }
    /// URL schemes a request of this protocol accepts (empty: any).
    fn url_schemes(&self) -> &[&'static str] {
        &[]
    }
    fn feature(&self) -> Option<&'static str>;
    fn request_languages(&self) -> &[&'static str];
    fn roles(&self) -> &[RoleSpec];
    fn interaction(&self) -> Interaction;

    fn plan(&self, doc: &FluxDoc, scope: &VarScope) -> Result<Plan, Vec<Diagnostic>>;
    /// Must be called inside a Tokio runtime. The stream ends with exactly one
    /// `Done` or `Failed`, preceded only by `Phase`/`Redirect`/`Frame` events.
    fn execute(&self, plan: Plan, ctx: ExecCtx) -> BoxStream<'static, Event>;
    /// Declarative checks for the `expect` block (SPEC §7.7). `cx` carries the document the
    /// block came from (for body spans) and the scope used to interpolate its values.
    fn check(&self, result: &Outcome, expect: &ExpectBlock, cx: &CheckCtx<'_>) -> Vec<TestResult>;
    fn script_bindings(&self) -> ScriptBindings {
        ScriptBindings::default()
    }

    /// The request as scripts see it (`fw.request`), as JSON:
    /// `{method, url, headers: [[name, value]], body, name, path}`. Built from `plan` once
    /// planning succeeded, else best effort from `doc` and `scope` (unresolved variables stay
    /// as text). `None` if the protocol has no request view.
    fn script_request(
        &self,
        _doc: &FluxDoc,
        _scope: &VarScope,
        _plan: Option<&Plan>,
    ) -> Option<serde_json::Value> {
        None
    }
    /// Apply the edits a `pre` script recorded (`fw.request.*` setters) to a planned request.
    fn apply_script_ops(&self, _plan: &Plan, _ops: &[serde_json::Value]) -> Result<Plan, String> {
        Err("this protocol does not support request edits from scripts".into())
    }
    /// The response as scripts see it (`fw.response`): `{status, statusText, headers, text, time,
    /// size, timings}`.
    fn script_response(&self, _result: &Outcome) -> Option<serde_json::Value> {
        None
    }
    /// Evaluate a front matter `capture` expression (`$.json.path`, `header Name`,
    /// `cookie name`, `status`). `Ok(None)` when it matches nothing.
    fn capture(&self, _result: &Outcome, _expr: &str) -> Result<Option<String>, String> {
        Err("this protocol does not support capture".into())
    }
    fn history_record(&self, result: &Outcome) -> HistoryRecord;

    /// Resolved request snapshot stored under `payload.request` in history. Must not contain
    /// auth-derived headers or query values (only the auth type).
    fn history_request(&self, _plan: &Plan) -> serde_json::Value {
        serde_json::Value::Null
    }
    /// `(name, value)` secrets in the plan, so the store can redact them by value.
    fn history_secrets(&self, _plan: &Plan) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Shell command for a stored history record (secrets are already redacted).
    fn history_to_curl(&self, _record: &HistoryRecord) -> Option<String> {
        None
    }
    /// Rebuild an `Outcome` from a stored record, to show an earlier response.
    fn history_to_outcome(&self, _record: &HistoryRecord) -> Option<Outcome> {
        None
    }

    /// Export a plan as a shell command (`curl`). `None` if unsupported.
    fn to_curl(&self, _plan: &Plan, _mask_secrets: bool) -> Option<String> {
        None
    }
    /// Status of the token behind this plan's auth (OAuth2), for the Auth tab.
    fn token_info(&self, _plan: &Plan, _env: &EnvState) -> Option<TokenInfo> {
        None
    }
    /// Fetch, refresh, or (with `force`) replace the plan's token. `None` if the plan
    /// does not use token auth.
    fn fetch_token(
        &self,
        _plan: Plan,
        _ctx: ExecCtx,
        _force: bool,
    ) -> Option<BoxFuture<'static, Result<TokenInfo, ExecError>>> {
        None
    }
    /// Turn pasted text (e.g. a `curl` command) into edits that fill `doc`.
    fn edits_from_paste(&self, _text: &str, _doc: &FluxDoc) -> Option<Vec<Edit>> {
        None
    }
    /// Summary of pasted text for the New request dialog; `None` if not recognised.
    fn preview_paste(&self, _text: &str) -> Option<PasteSummary> {
        None
    }
}
