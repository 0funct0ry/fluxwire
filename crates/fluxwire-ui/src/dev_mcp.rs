//! The in-app MCP host (`dev-mcp`, ADR-009). The server runs on its own Tokio runtime; `ui.*`
//! tools reach the GPUI thread through [`GpuiBridge`], which sends a request over a channel and
//! waits on a reply with a timeout. Entities are touched only in the drain loop on the UI thread.

use crate::workspace::WorkspaceView;
use fluxwire_agent::bridge::UiRequest;
use fluxwire_agent::{AllowSet, FluxwireServer, Roots, UiBridge, tools};
use gpui::{App, Context, Window, WindowHandle};
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
            UiRequest::CapturePrototype { path } => {
                let b = window.bounds();
                let region = crate::capture::Region {
                    x: f32::from(b.origin.x),
                    y: f32::from(b.origin.y),
                    w: f32::from(b.size.width),
                    h: f32::from(b.size.height),
                    scale: window.scale_factor(),
                };
                window.activate_window();
                let out = crate::capture::capture(region, Path::new(&path))?;
                Ok(serde_json::json!({"path": out, "region": region}))
            }
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
            _ => Err("unsupported request".into()),
        }
    }
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
