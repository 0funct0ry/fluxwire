//! The built-in tools, plus the `ui.*` tools a GUI host adds with [`ui_tools`].

use crate::ToolClass;
use crate::bridge::UiRequest;
use crate::registry::{Registry, ToolCtx, ToolDef};
use rmcp::model::{JsonObject, Tool};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, JsonSchema)]
pub struct PingOut {
    /// Always `pong`.
    pub reply: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ServerInfoOut {
    pub version: String,
    /// The `fluxwire.toml` format version this build reads.
    pub format_version: u32,
    pub transports: Vec<String>,
    pub allowed_classes: Vec<String>,
    pub roots: Vec<String>,
    pub features: Vec<String>,
}

/// An input type with no fields.
#[derive(JsonSchema)]
pub struct NoParams {}

pub fn builtin() -> Registry {
    builtin_with(crate::engine::Engine::from_environment())
}

/// The built-in tools with engine tools bound to `engine` (tests pass temp directories).
pub fn builtin_with(engine: Arc<crate::engine::Engine>) -> Registry {
    let mut r = Registry::default();
    crate::engine::register(&mut r, engine);
    r.register(def::<NoParams, PingOut>(
        "ping",
        "Check the server is alive. Returns `pong`.",
        |_, _| {
            Ok(serde_json::to_value(PingOut {
                reply: "pong".into(),
            })
            .unwrap())
        },
    ));
    r.register(def::<NoParams, ServerInfoOut>(
        "server_info",
        "Version, format version, transports, allowed tool classes, roots and build features.",
        |ctx, _| {
            let mut features = vec![];
            if cfg!(feature = "dev-mcp") {
                features.push("dev-mcp".to_string());
            }
            if cfg!(unix) {
                features.push("unix-socket".to_string());
            }
            if ctx.ui.is_some() {
                features.push("ui-bridge".to_string());
            }
            let info = ServerInfoOut {
                version: env!("CARGO_PKG_VERSION").into(),
                format_version: fluxwire_format::SUPPORTED_FORMAT,
                transports: ctx.transports.iter().map(|s| s.to_string()).collect(),
                allowed_classes: ctx.allow.names().iter().map(|s| s.to_string()).collect(),
                roots: ctx
                    .roots
                    .list()
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect(),
                features,
            };
            Ok(serde_json::to_value(info).unwrap())
        },
    ));
    r
}

#[derive(Deserialize, JsonSchema)]
pub struct OpenWorkspaceIn {
    /// Absolute path of the Workspace directory.
    pub path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct OpenRequestIn {
    /// Workspace-relative key, such as `payments/create-payment.flux.md`.
    pub key: String,
    /// Open as a permanent tab instead of a preview.
    #[serde(default)]
    pub permanent: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct CaptureIn {
    /// Where to write the PNG.
    pub path: String,
}

/// Free-form JSON result of a `ui.*` tool.
#[derive(Serialize, JsonSchema)]
pub struct UiOut {}

fn forward(ctx: &ToolCtx, req: UiRequest) -> Result<serde_json::Value, String> {
    ctx.ui
        .as_ref()
        .ok_or_else(|| "no UI bridge in this host".to_string())?
        .call(req)
}

fn parse<T: serde::de::DeserializeOwned>(args: Option<JsonObject>) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::Object(args.unwrap_or_default()))
        .map_err(|e| format!("invalid arguments: {e}"))
}

/// The `ui.*` tools; a GUI host registers them next to [`builtin`] and supplies a `UiBridge`.
pub fn ui_tools(r: &mut Registry) {
    r.register(def_in::<NoParams, UiOut>(
        ToolClass::Ui,
        "ui.state",
        "Serializable snapshot of the running app: Workspace, tabs, docks, Problems, Console, active response, runs, modal, notice, focus, window, theme and a `generation` that increases on every change.",
        |ctx, _| forward(ctx, UiRequest::State),
    ));
    r.register(def_in::<OpenWorkspaceIn, UiOut>(
        ToolClass::Ui,
        "ui.open_workspace",
        "Open a Workspace directory in the running app.",
        |ctx, args| {
            let a: OpenWorkspaceIn = parse(args)?;
            forward(ctx, UiRequest::OpenWorkspace { path: a.path })
        },
    ));
    r.register(def_in::<OpenRequestIn, UiOut>(
        ToolClass::Ui,
        "ui.open_request",
        "Open a Request tab by key; `permanent` pins it instead of previewing.",
        |ctx, args| {
            let a: OpenRequestIn = parse(args)?;
            forward(
                ctx,
                UiRequest::OpenRequest {
                    key: a.key,
                    permanent: a.permanent,
                },
            )
        },
    ));
    r.register(def_in::<CaptureIn, UiOut>(
        ToolClass::Ui,
        "ui.capture_prototype",
        "Prototype: write a PNG of the app window with `screencapture` (macOS; needs Screen Recording permission). Superseded by `ui.screenshot`.",
        |ctx, args| {
            let a: CaptureIn = parse(args)?;
            forward(ctx, UiRequest::CapturePrototype { path: a.path })
        },
    ));
}

/// Builds a read-class definition whose input and output schemas come from the types.
pub fn def<I: JsonSchema + 'static, O: JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
    f: impl Fn(&ToolCtx, Option<JsonObject>) -> Result<serde_json::Value, String>
    + Send
    + Sync
    + 'static,
) -> ToolDef {
    def_in::<I, O>(ToolClass::Read, name, description, f)
}

pub fn def_in<I: JsonSchema + 'static, O: JsonSchema + 'static>(
    class: ToolClass,
    name: &'static str,
    description: &'static str,
    f: impl Fn(&ToolCtx, Option<JsonObject>) -> Result<serde_json::Value, String>
    + Send
    + Sync
    + 'static,
) -> ToolDef {
    let tool = Tool::new(name, description, JsonObject::new())
        .with_input_schema::<I>()
        .with_output_schema::<O>();
    ToolDef {
        tool,
        class,
        handler: Arc::new(f),
    }
}
