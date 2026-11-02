//! The in-app MCP host (`dev-mcp`, ADR-009). The server runs on its own Tokio runtime; `ui.*`
//! tools reach the GPUI thread through [`GpuiBridge`], which sends a request over a channel and
//! waits on a reply with a timeout. Entities are touched only in the drain loop on the UI thread.

use crate::workspace::WorkspaceView;
use fluxwire_agent::bridge::UiRequest;
use fluxwire_agent::{AllowSet, FluxwireServer, Roots, UiBridge, tools};
use gpui::{App, AppContext as _, Context, Window, WindowHandle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

/// How long a `ui.*` call waits for the UI thread.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// What the host serves, from `--mcp-socket [PATH]` and `--mcp-stdio`.
#[derive(Clone, Debug, Default)]
pub struct HostOpts {
    pub stdio: bool,
    /// Serve a unix socket at this path (the default path when `Some(None)`).
    pub socket: Option<Option<PathBuf>>,
}

impl HostOpts {
    pub fn enabled(&self) -> bool {
        self.stdio || self.socket.is_some()
    }
}

type Reply = mpsc::Sender<Result<serde_json::Value, String>>;
type Call = (UiRequest, Reply);

/// Sends `ui.*` requests to the UI thread and waits for the answer.
pub struct GpuiBridge {
    tx: smol::channel::Sender<Call>,
    timeout: Duration,
}

impl GpuiBridge {
    pub fn new(timeout: Duration) -> (Self, smol::channel::Receiver<Call>) {
        let (tx, rx) = smol::channel::unbounded();
        (Self { tx, timeout }, rx)
    }
}

impl UiBridge for GpuiBridge {
    fn describe(&self) -> String {
        "fluxwire window".into()
    }

    fn call(&self, req: UiRequest) -> Result<serde_json::Value, String> {
        let (reply, wait) = mpsc::channel();
        self.tx
            .send_blocking((req, reply))
            .map_err(|_| "the app is shutting down".to_string())?;
        wait.recv_timeout(self.timeout)
            .map_err(|_| "ui thread busy: no answer within the timeout".to_string())?
    }
}

impl WorkspaceView {
    /// Runs one `ui.*` request on the UI thread.
    pub fn handle_ui_request(
        &mut self,
        req: UiRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        match self.handle_input_request(req.clone(), window, cx) {
            Some(out) => out,
            None => self.handle_core_request(req, window, cx),
        }
    }

    fn handle_core_request(
        &mut self,
        req: UiRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        match req {
            UiRequest::State => {
                serde_json::to_value(self.ui_state(window, cx)).map_err(|e| e.to_string())
            }
            UiRequest::OpenWorkspace { path } => {
                self.open_workspace(Path::new(&path), window, cx);
                match &self.open_error {
                    Some(e) => Err(e.clone()),
                    None => {
                        Ok(serde_json::json!({"opened": true, "workspace": self.workspace_label()}))
                    }
                }
            }
            UiRequest::Activate => Ok(self.activate_for_capture(window)),
            UiRequest::Capture { element } => self.capture_request(element, window, cx),
            UiRequest::SetTheme { name } => self.set_theme_request(name, cx),
            UiRequest::Resize { width, height } => self.resize_request(width, height, window),
            UiRequest::OpenRequest { key, permanent } => {
                self.notice = None;
                self.open_request(&key, permanent, window, cx);
                let active = self
                    .scope
                    .tabs
                    .active()
                    .map(|t| t.key.clone())
                    .unwrap_or_default();
                if active == key {
                    Ok(serde_json::json!({"opened": true, "key": key}))
                } else {
                    Err(self
                        .notice
                        .clone()
                        .unwrap_or_else(|| format!("could not open {key}")))
                }
            }
            UiRequest::Tree {
                subtree,
                role,
                visible_only,
            } => {
                let f = crate::dev_tree::TreeFilter {
                    subtree,
                    role: parse_role(role)?,
                    visible_only,
                };
                let roots = crate::dev_tree::filter(&crate::dev_id::last_frame(cx).tree(), &f)?;
                Ok(serde_json::json!({"tree": roots, "generation": self.generation}))
            }
            UiRequest::Find { label, role, text } => {
                let q = crate::dev_tree::FindQuery {
                    label,
                    role: parse_role(role)?,
                    text,
                };
                let found = crate::dev_tree::find(&crate::dev_id::last_frame(cx).tree(), &q);
                Ok(serde_json::json!({"elements": found}))
            }
            UiRequest::Audit {
                rules,
                subtree,
                theme,
                fixtures,
            } => self.audit_request(rules, subtree, theme, fixtures, window, cx),
            UiRequest::ClickElement { id } => {
                // Runs outside the entity lease in the drain loop (`click_element`); reaching
                // this arm means the request skipped it.
                Err(format!("click of {id:?} must go through the drain loop"))
            }
            _ => Err("unsupported request".into()),
        }
    }
}

impl WorkspaceView {
    fn audit_request(
        &mut self,
        rules: Vec<String>,
        subtree: Option<String>,
        theme: Option<String>,
        fixtures: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        use crate::dev_audit::{ActionRow, Inputs, audit};
        let reg = crate::theme::registry();
        let tokens = match &theme {
            Some(n) => reg
                .get(n)
                .or_else(|| reg.get(&n.replace(['-', '_'], " ")))
                .cloned()
                .ok_or_else(|| format!("unknown theme {n:?}"))?,
            None => reg
                .get(&crate::theme::theme(cx).name)
                .cloned()
                .ok_or("the active theme is not a bundled theme")?,
        };
        let f = crate::dev_tree::TreeFilter {
            subtree,
            ..Default::default()
        };
        let roots = crate::dev_tree::filter(&crate::dev_id::last_frame(cx).tree(), &f)?;
        let actions: Vec<ActionRow> = serde_json::from_value(
            serde_json::to_value(self.list_actions(window, cx)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let sources = scan_sources();
        let findings = audit(
            &roots,
            &tokens,
            &Inputs {
                actions: &actions,
                fixtures: &fixtures,
                latency: crate::dev_id::latency(cx),
                sources: &sources,
            },
            &rules,
        );
        Ok(serde_json::json!({
            "findings": findings,
            "theme": tokens.name,
            "generation": self.generation,
        }))
    }
}

/// The view sources the hard-coded colour rule scans, read from the source tree this build came
/// from. Empty when it is not on disk (an installed binary), which skips the rule.
fn scan_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let skip = ["theme.rs", "dev_audit.rs", "tests.rs", "assets.rs"];
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_string();
            (name.ends_with(".rs") && !skip.contains(&name.as_str()))
                .then(|| std::fs::read_to_string(&p).ok().map(|s| (name, s)))?
        })
        .collect();
    out.sort();
    out
}

fn parse_role(role: Option<String>) -> Result<Option<crate::dev_tree::Role>, String> {
    role.map(|r| {
        serde_json::from_value(serde_json::Value::String(r.clone()))
            .map_err(|_| format!("unknown role {r:?}"))
    })
    .transpose()
}

/// Runs the click handler a tagged element registered. It runs with the window but without the
/// root view leased, because the handlers are entity listeners that update the view themselves.
fn click_element(
    cx: &mut gpui::AsyncApp,
    handle: WindowHandle<WorkspaceView>,
    id: &str,
) -> Result<serde_json::Value, String> {
    let (point, click) = cx
        .update(|cx| {
            let frame = crate::dev_id::last_frame(cx);
            frame.click_point(id).and_then(|p| {
                crate::dev_id::click_handler(cx, id)
                    .map(|h| (p, h))
                    .ok_or_else(|| {
                        format!(
                            "element {id:?} has no click handler; use ui.invoke_action or ui.key"
                        )
                    })
            })
        })
        .map_err(|e| e.to_string())??;
    handle
        .update(cx, |ws, _, _| ws.notice = None)
        .map_err(|e| e.to_string())?;
    cx.update_window(handle.into(), |_, window, cx| {
        click(&gpui::ClickEvent::default(), window, cx)
    })
    .map_err(|e| e.to_string())?;
    handle
        .update(cx, |ws, window, cx| {
            ws.track_generation(window, cx);
            serde_json::json!({
                "clicked": id,
                "at": [point.0, point.1],
                "generation": ws.generation,
                "notice": ws.notice,
            })
        })
        .map_err(|e| e.to_string())
}

/// Starts the server thread and the UI-side drain loop. Does nothing when `opts` serves nothing.
pub fn start(
    cx: &mut App,
    handle: WindowHandle<WorkspaceView>,
    opts: HostOpts,
    root: Option<&Path>,
) {
    if !opts.enabled() {
        return;
    }
    let (bridge, rx) = GpuiBridge::new(CALL_TIMEOUT);
    let roots = root
        .and_then(|r| Roots::new(&[r.to_path_buf()]).ok())
        .unwrap_or_default();
    std::thread::Builder::new()
        .name("fluxwire-mcp".into())
        .spawn(move || {
            if let Err(e) = serve(opts, roots, Arc::new(bridge)) {
                eprintln!("fluxwire: mcp host stopped: {e:#}");
            }
        })
        .expect("spawn mcp host thread");
    cx.spawn(async move |cx| {
        while let Ok((req, reply)) = rx.recv().await {
            if let UiRequest::ClickElement { id } = &req {
                reply.send(click_element(cx, handle, id)).ok();
                continue;
            }
            let out = handle
                .update(cx, |ws, window, cx| ws.handle_ui_request(req, window, cx))
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            reply.send(out).ok();
        }
    })
    .detach();
}

fn serve(opts: HostOpts, roots: Roots, bridge: Arc<dyn UiBridge>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let mut transports: Vec<&'static str> = vec![];
        if opts.stdio {
            transports.push("stdio");
        }
        if opts.socket.is_some() {
            transports.push("unix");
        }
        let mut reg = tools::builtin();
        tools::ui_tools(&mut reg);
        let server = FluxwireServer::with_registry(
            reg,
            AllowSet::default(),
            transports,
            roots,
            Some(bridge),
        );
        let mut tasks = tokio::task::JoinSet::new();
        #[cfg(unix)]
        let _guard = if let Some(path) = &opts.socket {
            let path = path
                .clone()
                .unwrap_or_else(fluxwire_agent::transport::default_socket_path);
            let (listener, guard) = fluxwire_agent::transport::bind_unix(&path).await?;
            eprintln!("fluxwire: mcp socket {}", path.display());
            let s = server.clone();
            tasks.spawn(async move {
                fluxwire_agent::transport::serve_unix(listener, s, std::future::pending()).await
            });
            Some(guard)
        } else {
            None
        };
        if opts.stdio {
            let s = server.clone();
            tasks.spawn(async move { fluxwire_agent::transport::serve_stdio(s).await });
        }
        while let Some(done) = tasks.join_next().await {
            done??;
        }
        anyhow::Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unanswered_call_times_out() {
        let (bridge, _rx) = GpuiBridge::new(Duration::from_millis(30));
        let err = bridge.call(UiRequest::State).unwrap_err();
        assert!(err.contains("busy"), "{err}");
    }

    #[test]
    fn a_closed_channel_reports_shutdown() {
        let (bridge, rx) = GpuiBridge::new(Duration::from_millis(30));
        drop(rx);
        assert!(
            bridge
                .call(UiRequest::State)
                .unwrap_err()
                .contains("shutting down")
        );
    }

    #[test]
    fn nothing_is_served_by_default() {
        assert!(!HostOpts::default().enabled());
        assert!(
            HostOpts {
                stdio: true,
                socket: None
            }
            .enabled()
        );
    }
}
