//! Request lifecycle (SPEC §5.4): check `requires`, pick the protocol from the
//! registry, plan, execute, check `expect`, produce a `RunResult`.
//!
//! This crate contains no protocol-specific code; protocols are registered as
//! `Arc<dyn Protocol>` and spoken to only through `Plan`/`Event`/`Outcome`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

mod folder;
mod report;
pub use folder::*;
pub use report::{to_json, to_junit};

use fluxwire_model::{BlockData, BlockKind, Diagnostic, Edit, FluxDoc, FmValue};
pub use fluxwire_protocol::{
    CancellationToken, EnvState, ExecCtx, ExecError, ExecSettings, Frame, NetConfig, Outcome,
    PhaseTiming, RedirectHop, SessionHandle, TestResult,
};
use fluxwire_protocol::{CheckCtx, Event, HistoryRecord, Plan, Protocol};
pub use fluxwire_script::{
    ConsoleLine, ErrorKind as ScriptErrorKind, Limits, LogLevel, ScriptError, fw_dts,
};
use fluxwire_script::{Role, RunEnv, ScriptBlock, ScriptHost, ScriptState, VarEntry};
pub use fluxwire_vars::VarScope;
use fluxwire_vars::{Layer, Source, Var};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Why a run produced no outcome.
#[derive(Debug, Clone)]
pub enum RunError {
    /// The file has no runnable request block.
    NoRequest,
    /// The file needs features this version lacks (F401); it can be edited but not sent.
    Unsupported(Vec<String>),
    /// No registered protocol handles the request language.
    NoProtocol(String),
    /// Planning failed (bad URL, missing file, unresolved variable…).
    Plan(Vec<Diagnostic>),
    Exec(ExecError),
    /// The collection has scripts and has not been trusted (SPEC §21).
    Untrusted,
    /// A `pre` script failed, so nothing was sent.
    Script(Vec<ScriptError>),
}

impl RunError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, RunError::Exec(e) if e.phase == "cancelled")
    }

    /// One-line text plus an optional fix, for the response pane and status bar.
    pub fn describe(&self) -> (String, Option<String>) {
        match self {
            RunError::NoRequest => ("This file has no request to send.".into(), None),
            RunError::Unsupported(f) => (
                format!(
                    "This file requires features this version does not support: {}.",
                    f.join(", ")
                ),
                Some("It can be edited, but not sent (F401).".into()),
            ),
            RunError::NoProtocol(l) => (
                format!("No protocol is available for `{l}` requests."),
                None,
            ),
            RunError::Plan(d) => (
                d.iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None,
            ),
            RunError::Exec(e) => (e.message.clone(), e.fix.clone()),
            RunError::Untrusted => (
                "This collection contains scripts, and it is not trusted yet.".into(),
                Some("Trust the collection to run its scripts.".into()),
            ),
            RunError::Script(errs) => (
                errs.iter()
                    .map(|e| format!("{} {} ({})", e.kind.code(), e.message, e.location()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                None,
            ),
        }
    }

    pub fn phase(&self) -> &str {
        match self {
            RunError::Exec(e) => &e.phase,
            RunError::Plan(_) | RunError::NoRequest | RunError::NoProtocol(_) => "plan",
            RunError::Unsupported(_) => "requires",
            RunError::Untrusted => "trust",
            RunError::Script(_) => "pre script",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub label: String,
    pub started_at: SystemTime,
    pub outcome: Option<Box<Outcome>>,
    pub error: Option<RunError>,
    pub tests: Vec<TestResult>,
    pub history: Option<HistoryRecord>,
    /// Phases seen while running, in order (kept for failed runs too).
    pub phases: Vec<PhaseTiming>,
    /// Non-fatal planning diagnostics (e.g. unresolved variables left as literal text).
    pub warnings: Vec<Diagnostic>,
    /// `(name, value)` of secret variables in scope when the run started.
    pub secrets: Vec<(String, String)>,
    /// `console.*` output of every script phase, secrets redacted.
    pub console: Vec<ConsoleLine>,
    /// Net runtime-variable changes made by scripts and `capture`, in order.
    pub var_changes: Vec<VarChange>,
    /// `fw.env.persist(name)` requests, with the value to write.
    pub env_persist: Vec<(String, String)>,
    /// Failures of `post`/`test` scripts (a failing `pre` script is `RunError::Script`).
    pub script_errors: Vec<ScriptError>,
    /// `fw.skip(reason)` from a `pre` script: nothing was sent.
    pub skipped: Option<String>,
}

/// A runtime variable a script or `capture` set (`value: None` = unset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarChange {
    pub name: String,
    pub value: Option<String>,
    pub secret: bool,
}

impl RunResult {
    /// A result for a request that was not run (`fw.skip`, bail, cancel).
    pub(crate) fn skipped(label: String, reason: &str) -> Self {
        Self {
            label,
            started_at: SystemTime::now(),
            outcome: None,
            error: None,
            tests: Vec::new(),
            history: None,
            phases: Vec::new(),
            warnings: Vec::new(),
            secrets: Vec::new(),
            console: Vec::new(),
            var_changes: Vec::new(),
            env_persist: Vec::new(),
            script_errors: Vec::new(),
            skipped: Some(reason.to_string()),
        }
    }

    /// True when nothing went wrong: no error, no failed test, no script failure.
    /// A skipped request passes.
    pub fn passed(&self) -> bool {
        self.error.is_none() && self.script_errors.is_empty() && self.tests.iter().all(|t| t.passed)
    }

    /// Replace secret values with `‹secret:name›` (SPEC §8.3). Use for logs and the Console.
    pub fn redact(&self, text: &str) -> String {
        redact_with(&self.secrets, text)
    }
}

pub fn redact_with(secrets: &[(String, String)], text: &str) -> String {
    let mut s: Vec<&(String, String)> = secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
    s.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_string();
    for (n, v) in s {
        out = out.replace(v.as_str(), &format!("‹secret:{n}›"));
    }
    out
}

#[derive(Debug, Clone)]
pub enum RunEvent {
    Phase(PhaseTiming),
    Redirect(RedirectHop),
    /// A frame of a session protocol (WebSocket and later), in either direction.
    Frame(Frame),
    Finished(Box<RunResult>),
}

pub struct RunHandle {
    pub events: UnboundedReceiver<RunEvent>,
    cancel: CancellationToken,
    session: SessionHandle,
}

impl RunHandle {
    /// A clonable handle to send frames into, or close, the live session.
    pub fn session(&self) -> SessionHandle {
        self.session.clone()
    }

    /// Send a frame into a live session (`kind` is `text` or `binary`). `false` once the
    /// session has ended or if the protocol is not a session protocol.
    pub fn send_frame(&self, kind: &str, data: impl Into<fluxwire_protocol::Bytes>) -> bool {
        self.session.send(kind, data)
    }

    /// Close a live session gracefully; the run then finishes normally.
    pub fn close_session(&self) -> bool {
        self.session.close()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn canceller(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

#[derive(Clone, Default)]
pub struct SendOptions {
    pub scope: VarScope,
    pub settings: ExecSettings,
    pub net: NetConfig,
    /// Cookie jar and token cache of the active environment.
    pub env: Option<EnvState>,
    pub scripts: ScriptOptions,
}

/// Everything the script phases need beyond the document (SPEC §11, §21).
#[derive(Clone, Default)]
pub struct ScriptOptions {
    /// Scripts run only in trusted collections; `fluxwire run` trusts by definition.
    pub trusted: bool,
    pub limits: Limits,
    /// Collection root: enables folder/collection scripts, `lib/` imports and `fw.sendRequest`.
    pub collection_root: Option<PathBuf>,
    /// Absolute path of the request file (need not exist on disk).
    pub file: Option<PathBuf>,
    /// `[globals]` from settings.toml, for `fw.globals` and nested requests.
    pub globals: Vec<(String, String)>,
    pub allow_process_env: bool,
}

pub struct Runner {
    protocols: Arc<Vec<Arc<dyn Protocol>>>,
    rt: tokio::runtime::Runtime,
}

/// The shared Tokio runtime and the built-in protocols (SPEC §5.3).
pub fn runner() -> &'static Runner {
    static R: OnceLock<Runner> = OnceLock::new();
    R.get_or_init(|| {
        let protocols: Vec<Arc<dyn Protocol>> = vec![
            #[cfg(feature = "http")]
            Arc::new(fluxwire_http::HttpProtocol),
            #[cfg(feature = "ws")]
            Arc::new(fluxwire_ws::WsProtocol),
        ];
        Runner::new(protocols)
    })
}

impl Runner {
    pub fn new(protocols: Vec<Arc<dyn Protocol>>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("fluxwire-io")
            .enable_all()
            .build()
            .expect("tokio runtime");
        Self {
            protocols: Arc::new(protocols),
            rt,
        }
    }

    pub fn handle(&self) -> &tokio::runtime::Handle {
        self.rt.handle()
    }

    fn protocol_for(&self, doc: &FluxDoc) -> Result<&Arc<dyn Protocol>, RunError> {
        let i = doc.request_index().ok_or(RunError::NoRequest)?;
        let lang = &doc.blocks[i].info.language;
        self.protocols
            .iter()
            .find(|p| p.request_languages().contains(&lang.as_str()))
            .ok_or_else(|| RunError::NoProtocol(lang.clone()))
    }

    /// Start a send from the in-memory document (saving is never required).
    pub fn send(&self, doc: &FluxDoc, opts: SendOptions) -> RunHandle {
        let (tx, rx) = unbounded_channel();
        let cancel = CancellationToken::new();
        let (session, inbox) = fluxwire_protocol::session_channel();
        let ctx = ExecCtx {
            cancel: cancel.clone(),
            settings: opts.settings.clone(),
            net: opts.net.clone(),
            temp_dir: None,
            env: opts.env.clone(),
            session: Some(inbox),
        };
        let proto = self.protocol_for(doc).cloned();
        let doc = doc.clone();
        let started_at = SystemTime::now();
        let shared = self.protocols.clone();
        self.rt.spawn(async move {
            let job = Job {
                shared,
                proto,
                doc,
                scope: opts.scope,
                ctx,
                scripts: opts.scripts,
                depth: 0,
                started_at,
                ancestors: None,
            };
            let result = exec(job, Some(&tx)).await;
            let _ = tx.send(RunEvent::Finished(Box::new(result)));
        });
        RunHandle {
            events: rx,
            cancel,
            session,
        }
    }

    /// `Copy as cURL` (or any shell export the protocol offers).
    pub fn export_curl(
        &self,
        doc: &FluxDoc,
        scope: &VarScope,
        mask_secrets: bool,
    ) -> Result<String, RunError> {
        let proto = self.protocol_for(doc)?;
        let plan = proto.plan(doc, scope).map_err(RunError::Plan)?;
        proto
            .to_curl(&plan, mask_secrets)
            .ok_or_else(|| RunError::NoProtocol("curl export".into()))
    }

    /// OAuth2 token status for `doc` in `env`, if its auth uses tokens.
    pub fn token_info(
        &self,
        doc: &FluxDoc,
        scope: &VarScope,
        env: &EnvState,
    ) -> Option<fluxwire_protocol::TokenInfo> {
        let proto = self.protocol_for(doc).ok()?;
        let plan = proto.plan(doc, scope).ok()?;
        proto.token_info(&plan, env)
    }

    /// Fetch (or with `force`, replace) the token behind `doc`'s auth. The receiver yields
    /// once; `None` when the request does not use token auth or cannot be planned.
    pub fn fetch_token(
        &self,
        doc: &FluxDoc,
        opts: SendOptions,
        force: bool,
    ) -> Option<(
        tokio::sync::oneshot::Receiver<Result<fluxwire_protocol::TokenInfo, ExecError>>,
        CancellationToken,
    )> {
        let proto = self.protocol_for(doc).ok()?.clone();
        let plan = proto.plan(doc, &opts.scope).ok()?;
        let cancel = CancellationToken::new();
        let ctx = ExecCtx {
            cancel: cancel.clone(),
            settings: opts.settings,
            net: opts.net,
            temp_dir: None,
            env: opts.env,
            session: None,
        };
        let fut = proto.fetch_token(plan, ctx, force)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.rt.spawn(async move {
            let _ = tx.send(fut.await);
        });
        Some((rx, cancel))
    }

    fn protocol_by_id(&self, id: &str) -> Option<&Arc<dyn Protocol>> {
        self.protocols.iter().find(|p| p.id() == id)
    }

    /// `Copy as cURL` for a stored history record (already redacted).
    pub fn history_curl(&self, rec: &HistoryRecord) -> Option<String> {
        self.protocol_by_id(&rec.protocol)?.history_to_curl(rec)
    }

    /// Rebuild the outcome of a stored record, to show an earlier response.
    pub fn history_outcome(&self, rec: &HistoryRecord) -> Option<Outcome> {
        self.protocol_by_id(&rec.protocol)?.history_to_outcome(rec)
    }

    /// Edits that fill `doc` from pasted text, if some protocol recognises it.
    pub fn edits_from_paste(&self, doc: &FluxDoc, text: &str) -> Option<Vec<Edit>> {
        let proto = self.protocol_for(doc).ok()?;
        proto.edits_from_paste(text, doc)
    }
}

/// One run of one request (the unit `fw.sendRequest` nests).
struct Job {
    shared: Arc<Vec<Arc<dyn Protocol>>>,
    proto: Result<Arc<dyn Protocol>, RunError>,
    doc: FluxDoc,
    scope: VarScope,
    ctx: ExecCtx,
    scripts: ScriptOptions,
    depth: u32,
    started_at: SystemTime,
    /// Collection and folder docs above the request, when the caller has them cached
    /// (folder runs); `None` reads them from disk.
    ancestors: Option<Arc<Vec<(String, FluxDoc)>>>,
}

const MAX_NESTING: u32 = 3;

fn rel_label(root: Option<&Path>, file: Option<&Path>, doc: &FluxDoc) -> String {
    match (root, file) {
        (Some(r), Some(f)) => f
            .strip_prefix(r)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/"),
        (_, Some(f)) => f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => doc.name().unwrap_or("request").to_string(),
    }
}

/// The three script lists for one request, in SPEC §11.2 order.
struct Scripts {
    pre: Vec<ScriptBlock>,
    post: Vec<ScriptBlock>,
    tests: Vec<ScriptBlock>,
}

fn collect_scripts(doc: &FluxDoc, ancestors: &[(String, FluxDoc)], own_label: &str) -> Scripts {
    let blocks = |d: &FluxDoc, label: &str, role| fluxwire_script::blocks_from_doc(d, label, role);
    let mut pre = Vec::new();
    let mut post = Vec::new();
    let mut tests = Vec::new();
    // collection → folders (outer → inner) → request
    for (label, d) in ancestors {
        pre.extend(blocks(d, label, Role::Pre));
        tests.extend(blocks(d, label, Role::Test));
    }
    pre.extend(blocks(doc, own_label, Role::Pre));
    tests.extend(blocks(doc, own_label, Role::Test));
    // request → folders (inner → outer) → collection
    post.extend(blocks(doc, own_label, Role::Post));
    for (label, d) in ancestors.iter().rev() {
        post.extend(blocks(d, label, Role::Post));
    }
    Scripts { pre, post, tests }
}

fn script_state(
    scope: &VarScope,
    opts: &ScriptOptions,
    request: Option<Value>,
    response: Option<Value>,
) -> ScriptState {
    ScriptState {
        request,
        response,
        vars: scope
            .vars
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    VarEntry {
                        value: v.value.clone(),
                        secret: v.secret,
                    },
                )
            })
            .collect(),
        env_vars: scope
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect(),
        env_name: scope.env_name.clone().unwrap_or_default(),
        globals: opts.globals.iter().cloned().collect(),
        ..Default::default()
    }
}

fn runtime_var(value: String, secret: bool) -> Var {
    Var {
        value,
        secret,
        source: Source::new("runtime", None),
    }
}

/// Fold what a script phase changed into the scope and the run result.
fn absorb(scope: &mut VarScope, st: &mut ScriptState, result: &mut RunResult) {
    for (name, change) in std::mem::take(&mut st.vars_changed) {
        match change {
            Some(e) => {
                if e.secret {
                    result.secrets.push((name.clone(), e.value.clone()));
                }
                scope.set(
                    Layer::Runtime,
                    name.clone(),
                    runtime_var(e.value.clone(), e.secret),
                );
                result.var_changes.push(VarChange {
                    name,
                    value: Some(e.value),
                    secret: e.secret,
                });
            }
            None => {
                scope.remove(&name);
                result.var_changes.push(VarChange {
                    name,
                    value: None,
                    secret: false,
                });
            }
        }
    }
    for (name, value) in std::mem::take(&mut st.env_set) {
        let secret = scope.var(&name).is_some_and(|v| v.secret);
        scope.set(
            Layer::Runtime,
            name.clone(),
            runtime_var(value.clone(), secret),
        );
        result.var_changes.push(VarChange {
            name,
            value: Some(value),
            secret,
        });
    }
    for name in std::mem::take(&mut st.persist) {
        if let Some(v) = scope.get(&name) {
            result.env_persist.push((name, v.to_string()));
        }
    }
    result.console.append(&mut st.console);
}

fn to_test_result(t: fluxwire_script::TestOutcome) -> TestResult {
    TestResult {
        name: t.name,
        passed: t.passed,
        expected: t.expected,
        actual: t.actual,
        message: t.message,
    }
}

/// Runs one phase's blocks on the blocking pool.
async fn run_phase(
    blocks: Vec<ScriptBlock>,
    state: ScriptState,
    env: RunEnv,
) -> (ScriptState, Vec<ScriptError>) {
    if blocks.is_empty() {
        return (state, Vec::new());
    }
    let fallback = blocks[0].clone();
    tokio::task::spawn_blocking(move || fluxwire_script::run_blocks(&blocks, state, &env))
        .await
        .unwrap_or_else(|e| {
            (
                ScriptState::default(),
                vec![ScriptError {
                    kind: fluxwire_script::ErrorKind::Throw,
                    message: format!("the script runtime crashed: {e}"),
                    file: fallback.file,
                    line: fallback.first_line,
                    col: 0,
                    stack: String::new(),
                }],
            )
        })
}

/// `fw.sendRequest` and `pm.sendRequest`: sends another request through the same engine.
struct NestedHost {
    shared: Arc<Vec<Arc<dyn Protocol>>>,
    handle: tokio::runtime::Handle,
    scope: VarScope,
    ctx: ExecCtx,
    opts: ScriptOptions,
    depth: u32,
}

impl NestedHost {
    fn load(&self, target: &Value) -> Result<(FluxDoc, Option<PathBuf>), String> {
        match target {
            Value::String(rel) => {
                let root = self
                    .opts
                    .collection_root
                    .as_ref()
                    .ok_or("fw.sendRequest needs a collection")?;
                let joined = root.join(rel);
                let (canon, canon_root) = (
                    joined.canonicalize().map_err(|e| format!("{rel}: {e}"))?,
                    root.canonicalize().map_err(|e| e.to_string())?,
                );
                if !canon.starts_with(&canon_root) || !rel.ends_with(".flux.md") {
                    return Err(format!(
                        "{rel}: only .flux.md files inside the collection can be sent"
                    ));
                }
                let text = std::fs::read_to_string(&canon).map_err(|e| format!("{rel}: {e}"))?;
                Ok((fluxwire_format::parse(&text), Some(joined)))
            }
            Value::Object(o) => {
                let get = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default();
                let method = if get("method").is_empty() {
                    "GET"
                } else {
                    get("method")
                };
                let mut text = format!("```http\n{method} {}\n", get("url"));
                if let Some(hs) = o.get("headers").and_then(Value::as_array) {
                    for h in hs {
                        if let (Some(k), Some(v)) = (
                            h.get(0).and_then(Value::as_str),
                            h.get(1).and_then(Value::as_str),
                        ) {
                            text.push_str(&format!("{k}: {v}\n"));
                        }
                    }
                }
                if let Some(b) = o.get("body").and_then(Value::as_str) {
                    text.push_str(&format!("\n{b}\n"));
                }
                text.push_str("```\n");
                Ok((fluxwire_format::parse(&text), None))
            }
            _ => Err("fw.sendRequest expects a .flux.md path".into()),
        }
    }
}

impl ScriptHost for NestedHost {
    fn send_request(
        &self,
        target: &Value,
        vars: &BTreeMap<String, String>,
    ) -> Result<Value, String> {
        if self.depth >= MAX_NESTING {
            return Err("fw.sendRequest is nested too deeply".into());
        }
        let (doc, file) = self.load(target)?;
        let mut scope = match (&self.opts.collection_root, &file) {
            (Some(root), Some(file)) => {
                let runtime: BTreeMap<String, (String, bool)> = self
                    .scope
                    .vars
                    .iter()
                    .filter(|(_, v)| v.source.label == "runtime")
                    .map(|(k, v)| (k.clone(), (v.value.clone(), v.secret)))
                    .collect();
                fluxwire_format::build_scope(&fluxwire_format::ScopeInputs {
                    collection_root: root,
                    file,
                    doc: &doc,
                    env: self.scope.env_name.as_deref(),
                    runtime: &runtime,
                    globals: &self.opts.globals,
                    allow_process_env: self.opts.allow_process_env,
                })
            }
            _ => self.scope.clone(),
        };
        for (k, v) in vars {
            scope.set(Layer::Runtime, k.clone(), runtime_var(v.clone(), false));
        }
        let proto = pick_protocol(&self.shared, &doc);
        let mut scripts = self.opts.clone();
        scripts.file = file;
        let job = Job {
            shared: self.shared.clone(),
            proto: proto.clone(),
            doc,
            scope,
            // A nested send never takes over the caller's live session.
            ctx: ExecCtx {
                session: None,
                ..self.ctx.clone()
            },
            scripts,
            depth: self.depth + 1,
            started_at: SystemTime::now(),
            ancestors: None,
        };
        let result = self.handle.block_on(exec(job, None));
        if let Some(err) = result.error {
            let (msg, _) = err.describe();
            return Err(msg);
        }
        let outcome = result.outcome.ok_or("the request produced no response")?;
        proto
            .ok()
            .and_then(|p| p.script_response(&outcome))
            .ok_or_else(|| "the response is not available to scripts".to_string())
    }
}

fn pick_protocol(
    protocols: &[Arc<dyn Protocol>],
    doc: &FluxDoc,
) -> Result<Arc<dyn Protocol>, RunError> {
    let i = doc.request_index().ok_or(RunError::NoRequest)?;
    let lang = &doc.blocks[i].info.language;
    protocols
        .iter()
        .find(|p| p.request_languages().contains(&lang.as_str()))
        .cloned()
        .ok_or_else(|| RunError::NoProtocol(lang.clone()))
}

fn capture_exprs(doc: &FluxDoc) -> Vec<(String, String)> {
    match doc.front.get("capture").map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn redact_console(result: &mut RunResult) {
    let secrets = result.secrets.clone();
    for line in &mut result.console {
        line.text = redact_with(&secrets, &line.text);
    }
}

async fn exec(job: Job, tx: Option<&UnboundedSender<RunEvent>>) -> RunResult {
    let Job {
        shared,
        proto,
        doc,
        mut scope,
        ctx,
        scripts: sopts,
        depth,
        started_at,
        ancestors,
    } = job;
    let mut result = RunResult {
        label: String::new(),
        started_at,
        outcome: None,
        error: None,
        tests: Vec::new(),
        history: None,
        phases: Vec::new(),
        warnings: Vec::new(),
        secrets: scope
            .vars
            .iter()
            .filter(|(_, v)| v.secret)
            .map(|(n, v)| (n.clone(), v.value.clone()))
            .collect(),
        console: Vec::new(),
        var_changes: Vec::new(),
        env_persist: Vec::new(),
        script_errors: Vec::new(),
        skipped: None,
    };
    if !doc.unsupported.is_empty() {
        result.error = Some(RunError::Unsupported(doc.unsupported.clone()));
        return result;
    }
    let proto = match proto {
        Ok(p) => p,
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };

    // ---- script setup ---------------------------------------------------------------
    let ancestors = match (ancestors, &sopts.collection_root, &sopts.file) {
        (Some(a), _, _) => a,
        (None, Some(root), Some(file)) => Arc::new(fluxwire_format::ancestor_docs(root, file)),
        _ => Arc::new(Vec::new()),
    };
    let own_label = rel_label(
        sopts.collection_root.as_deref(),
        sopts.file.as_deref(),
        &doc,
    );
    let scripts = collect_scripts(&doc, &ancestors, &own_label);
    let has_scripts =
        !(scripts.pre.is_empty() && scripts.post.is_empty() && scripts.tests.is_empty());
    if has_scripts && !sopts.trusted {
        result.error = Some(RunError::Untrusted);
        return result;
    }
    let script_env = |scope: &VarScope| RunEnv {
        limits: sopts.limits,
        lib_dir: sopts.collection_root.as_ref().map(|r| r.join("lib")),
        host: Some(Arc::new(NestedHost {
            shared: shared.clone(),
            handle: tokio::runtime::Handle::current(),
            scope: scope.clone(),
            ctx: ctx.clone(),
            opts: sopts.clone(),
            depth,
        })),
    };

    // ---- pre ------------------------------------------------------------------------
    let mut request_ops: Vec<Value> = Vec::new();
    if !scripts.pre.is_empty() {
        let plan = proto.plan(&doc, &scope).ok();
        let view = proto.script_request(&doc, &scope, plan.as_ref());
        let state = script_state(&scope, &sopts, view, None);
        let (mut st, errors) = run_phase(scripts.pre, state, script_env(&scope)).await;
        request_ops = std::mem::take(&mut st.request_ops);
        result.tests.extend(st.tests.drain(..).map(to_test_result));
        let skip = st.skip.take();
        absorb(&mut scope, &mut st, &mut result);
        redact_console(&mut result);
        if !errors.is_empty() {
            result.error = Some(RunError::Script(errors));
            return result;
        }
        if let Some(reason) = skip {
            result.skipped = Some(reason);
            return result;
        }
    }

    let mut plan: Plan = match proto.plan(&doc, &scope) {
        Ok(p) => p,
        Err(d) => {
            result.error = Some(RunError::Plan(d));
            return result;
        }
    };
    if !request_ops.is_empty() {
        plan = match proto.apply_script_ops(&plan, &request_ops) {
            Ok(p) => p,
            Err(msg) => {
                result.error = Some(RunError::Exec(ExecError {
                    phase: "pre script".into(),
                    message: msg,
                    fix: None,
                }));
                return result;
            }
        };
    }
    result.label = result.redact(&plan.label);
    result.warnings = plan.warnings.clone();
    let request = proto.history_request(&plan);
    let protocol = plan.protocol;
    result.secrets.extend(proto.history_secrets(&plan));
    let request_view = if scripts.post.is_empty() && scripts.tests.is_empty() {
        None
    } else {
        proto.script_request(&doc, &scope, Some(&plan))
    };
    let mut stream = proto.execute(plan, ctx.clone());
    let mut done: Option<Box<Outcome>> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            Event::Phase(p) => {
                result.phases.push(p.clone());
                if let Some(tx) = tx {
                    let _ = tx.send(RunEvent::Phase(p));
                }
            }
            Event::Redirect(r) => {
                if let Some(tx) = tx {
                    let _ = tx.send(RunEvent::Redirect(r));
                }
            }
            Event::Frame(f) => {
                if let Some(tx) = tx {
                    let _ = tx.send(RunEvent::Frame(f));
                }
            }
            Event::Done(o) => {
                done = Some(o);
                break;
            }
            Event::Failed(e) => {
                result.history = Some(HistoryRecord {
                    protocol: protocol.into(),
                    summary: format!("{} ✗ {}", result.label, e.phase),
                    payload: serde_json::json!({
                        "request": request,
                        "timings": result.phases,
                        "error": {"phase": e.phase, "message": e.message, "fix": e.fix},
                    }),
                });
                result.error = Some(RunError::Exec(e));
                break;
            }
        }
    }
    drop(stream);
    let Some(o) = done else {
        return result;
    };

    // ---- capture --------------------------------------------------------------------
    for (name, expr) in capture_exprs(&doc) {
        match proto.capture(&o, &expr) {
            Ok(Some(v)) => {
                let secret = scope.var(&name).is_some_and(|x| x.secret);
                scope.set(Layer::Runtime, name.clone(), runtime_var(v.clone(), secret));
                result.var_changes.push(VarChange {
                    name,
                    value: Some(v),
                    secret,
                });
            }
            Ok(None) => result.warnings.push(Diagnostic::new(
                "P110",
                fluxwire_model::Severity::Warning,
                fluxwire_model::Span::at(0),
                format!("capture `{name}`: `{expr}` matched nothing in the response."),
            )),
            Err(msg) => result.warnings.push(Diagnostic::new(
                "P111",
                fluxwire_model::Severity::Warning,
                fluxwire_model::Span::at(0),
                format!("capture `{name}`: {msg}"),
            )),
        }
    }

    // ---- post -----------------------------------------------------------------------
    let response_view = proto.script_response(&o);
    if !scripts.post.is_empty() {
        let state = script_state(&scope, &sopts, request_view.clone(), response_view.clone());
        let (mut st, errors) = run_phase(scripts.post, state, script_env(&scope)).await;
        result.tests.extend(st.tests.drain(..).map(to_test_result));
        absorb(&mut scope, &mut st, &mut result);
        result.script_errors.extend(errors);
    }

    // ---- expect, then test blocks -----------------------------------------------------
    if let Some(i) = doc.blocks.iter().position(|b| b.kind == BlockKind::Expect)
        && let Some(BlockData::Expect(e)) = doc.data.get(i)
    {
        let cx = CheckCtx {
            doc: Some(&doc),
            scope: &scope,
        };
        result.tests.extend(proto.check(&o, e, &cx));
    }
    if !scripts.tests.is_empty() {
        let state = script_state(&scope, &sopts, request_view, response_view);
        let (mut st, errors) = run_phase(scripts.tests, state, script_env(&scope)).await;
        result.tests.extend(st.tests.drain(..).map(to_test_result));
        absorb(&mut scope, &mut st, &mut result);
        result.script_errors.extend(errors);
    }
    redact_console(&mut result);

    let mut rec = proto.history_record(&o);
    if let Some(m) = rec.payload.as_object_mut() {
        m.insert("request".into(), request.clone());
    }
    result.history = Some(rec);
    result.outcome = Some(o);
    result
}

/// Network settings (SPEC §9.1) from `fluxwire.local.toml` in a collection directory:
///
/// ```toml
/// [tls]
/// ca = ["certs/ca.pem"]
/// [[tls.client_cert]]
/// host = "api.internal"
/// cert = "certs/client.pem"
/// key = "certs/client.key"
/// [proxy]
/// url = "http://proxy:3128"
/// no_proxy = "localhost,.internal"
/// ```
///
/// Relative paths resolve against `dir`. A missing or unreadable file yields the defaults.
pub fn net_config_from_dir(dir: &std::path::Path) -> NetConfig {
    #[derive(serde::Deserialize, Default)]
    struct Cert {
        host: String,
        cert: std::path::PathBuf,
        key: std::path::PathBuf,
    }
    #[derive(serde::Deserialize, Default)]
    struct Tls {
        #[serde(default)]
        ca: Vec<std::path::PathBuf>,
        #[serde(default)]
        client_cert: Vec<Cert>,
    }
    #[derive(serde::Deserialize, Default)]
    struct Proxy {
        url: Option<String>,
        no_proxy: Option<String>,
    }
    #[derive(serde::Deserialize, Default)]
    struct File {
        #[serde(default)]
        tls: Tls,
        #[serde(default)]
        proxy: Proxy,
    }
    let Ok(text) = std::fs::read_to_string(dir.join("fluxwire.local.toml")) else {
        return NetConfig::default();
    };
    let Ok(f) = toml::from_str::<File>(&text) else {
        return NetConfig::default();
    };
    let abs = |p: std::path::PathBuf| if p.is_relative() { dir.join(p) } else { p };
    NetConfig {
        ca_files: f.tls.ca.into_iter().map(abs).collect(),
        client_certs: f
            .tls
            .client_cert
            .into_iter()
            .map(|c| fluxwire_protocol::ClientCert {
                host: c.host,
                cert: abs(c.cert),
                key: abs(c.key),
            })
            .collect(),
        proxy: f.proxy.url,
        no_proxy: f.proxy.no_proxy,
    }
}

/// Collection `[defaults]` (SPEC §6.1) applied over the built-in execution settings.
pub fn settings_from_defaults(d: &fluxwire_model::Defaults) -> ExecSettings {
    let mut s = ExecSettings::default();
    if let Some(ms) = d.timeout_ms {
        s.timeout = (ms > 0).then(|| std::time::Duration::from_millis(ms));
    }
    if let Some(f) = d.follow_redirects {
        s.follow_redirects = f;
    }
    if let Some(m) = d.max_redirects {
        s.max_redirects = m;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_toml_is_read_with_relative_paths_resolved() {
        let dir = std::env::temp_dir().join(format!("fw-net-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fluxwire.local.toml"),
            "[tls]\nca=[\"ca.pem\"]\n[[tls.client_cert]]\nhost=\"h\"\ncert=\"c.pem\"\nkey=\"/abs/k\"\n[proxy]\nurl=\"http://p:1\"\n",
        )
        .unwrap();
        let n = net_config_from_dir(&dir);
        assert_eq!(n.ca_files, [dir.join("ca.pem")]);
        assert_eq!(n.client_certs[0].key, std::path::PathBuf::from("/abs/k"));
        assert_eq!(n.proxy.as_deref(), Some("http://p:1"));
        assert!(
            net_config_from_dir(&dir.join("missing"))
                .ca_files
                .is_empty()
        );
    }

    #[test]
    fn defaults_override_builtin_settings() {
        let d = fluxwire_model::Defaults {
            timeout_ms: Some(5000),
            follow_redirects: Some(false),
            max_redirects: Some(2),
            ..Default::default()
        };
        let s = settings_from_defaults(&d);
        assert_eq!(s.timeout, Some(std::time::Duration::from_secs(5)));
        assert!(!s.follow_redirects && s.max_redirects == 2);
    }
}
