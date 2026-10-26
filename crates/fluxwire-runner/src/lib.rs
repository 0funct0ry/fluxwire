//! Request lifecycle (SPEC §5.4): check `requires`, pick the protocol from the
//! registry, plan, execute, check `expect`, produce a `RunResult`.
//!
//! This crate contains no protocol-specific code; protocols are registered as
//! `Arc<dyn Protocol>` and spoken to only through `Plan`/`Event`/`Outcome`.

use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use fluxwire_model::{BlockData, Diagnostic, Edit, FluxDoc};
pub use fluxwire_protocol::{
    CancellationToken, ExecCtx, ExecError, ExecSettings, NetConfig, Outcome, PhaseTiming,
    RedirectHop, TestResult,
};
use fluxwire_protocol::{Event, HistoryRecord, Plan, Protocol};
pub use fluxwire_vars::VarScope;
use futures::StreamExt;
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
        }
    }

    pub fn phase(&self) -> &str {
        match self {
            RunError::Exec(e) => &e.phase,
            RunError::Plan(_) | RunError::NoRequest | RunError::NoProtocol(_) => "plan",
            RunError::Unsupported(_) => "requires",
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
}

#[derive(Debug, Clone)]
pub enum RunEvent {
    Phase(PhaseTiming),
    Redirect(RedirectHop),
    Finished(Box<RunResult>),
}

pub struct RunHandle {
    pub events: UnboundedReceiver<RunEvent>,
    cancel: CancellationToken,
}

impl RunHandle {
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
}

pub struct Runner {
    protocols: Vec<Arc<dyn Protocol>>,
    rt: tokio::runtime::Runtime,
}

/// The shared Tokio runtime and the built-in protocols (SPEC §5.3).
pub fn runner() -> &'static Runner {
    static R: OnceLock<Runner> = OnceLock::new();
    R.get_or_init(|| {
        let protocols: Vec<Arc<dyn Protocol>> = vec![
            #[cfg(feature = "http")]
            Arc::new(fluxwire_http::HttpProtocol),
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
        Self { protocols, rt }
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
        let ctx = ExecCtx {
            cancel: cancel.clone(),
            settings: opts.settings.clone(),
            net: opts.net.clone(),
            temp_dir: None,
        };
        let proto = self.protocol_for(doc).cloned();
        let doc = doc.clone();
        let started_at = SystemTime::now();
        self.rt.spawn(async move {
            let result = run(proto, doc, opts.scope, ctx, &tx, started_at).await;
            let _ = tx.send(RunEvent::Finished(Box::new(result)));
        });
        RunHandle { events: rx, cancel }
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

    /// Edits that fill `doc` from pasted text, if some protocol recognises it.
    pub fn edits_from_paste(&self, doc: &FluxDoc, text: &str) -> Option<Vec<Edit>> {
        let proto = self.protocol_for(doc).ok()?;
        proto.edits_from_paste(text, doc)
    }
}

async fn run(
    proto: Result<Arc<dyn Protocol>, RunError>,
    doc: FluxDoc,
    scope: VarScope,
    ctx: ExecCtx,
    tx: &UnboundedSender<RunEvent>,
    started_at: SystemTime,
) -> RunResult {
    let mut result = RunResult {
        label: String::new(),
        started_at,
        outcome: None,
        error: None,
        tests: Vec::new(),
        history: None,
        phases: Vec::new(),
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
    let plan: Plan = match proto.plan(&doc, &scope) {
        Ok(p) => p,
        Err(d) => {
            result.error = Some(RunError::Plan(d));
            return result;
        }
    };
    result.label = plan.label.clone();
    let mut stream = proto.execute(plan, ctx);
    while let Some(ev) = stream.next().await {
        match ev {
            Event::Phase(p) => {
                result.phases.push(p.clone());
                let _ = tx.send(RunEvent::Phase(p));
            }
            Event::Redirect(r) => {
                let _ = tx.send(RunEvent::Redirect(r));
            }
            Event::Frame(_) => {}
            Event::Done(o) => {
                if let Some(i) = doc
                    .blocks
                    .iter()
                    .position(|b| b.kind == fluxwire_model::BlockKind::Expect)
                    && let Some(BlockData::Expect(e)) = doc.data.get(i)
                {
                    result.tests = proto.check(&o, e);
                }
                result.history = Some(proto.history_record(&o));
                result.outcome = Some(o);
                break;
            }
            Event::Failed(e) => {
                result.error = Some(RunError::Exec(e));
                break;
            }
        }
    }
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
