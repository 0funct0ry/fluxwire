//! The built-in tools, plus the `ui.*` tools a GUI host adds with [`ui_tools`].

use crate::ToolClass;
use crate::bridge::{UiCommand, UiRequest};
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

/// Flat input of `ui.command`: `command` selects the operation, the rest are its arguments.
#[derive(Deserialize, JsonSchema)]
pub struct CommandIn {
    /// `open_file`, `set_url`, `select_tab` or `choose_environment`.
    pub command: String,
    /// `open_file`, `select_tab`: Workspace-relative key.
    pub key: Option<String>,
    /// `open_file`: open as a permanent tab.
    #[serde(default)]
    pub permanent: bool,
    /// `set_url`: the new request target.
    pub value: Option<String>,
    /// `choose_environment`: environment name; omit to clear.
    pub name: Option<String>,
}

impl CommandIn {
    fn into_command(self) -> Result<UiCommand, String> {
        let need =
            |v: Option<String>, f: &str| v.ok_or(format!("`{f}` is required for {}", self.command));
        Ok(match self.command.as_str() {
            "open_file" => UiCommand::OpenFile {
                key: need(self.key.clone(), "key")?,
                permanent: self.permanent,
            },
            "set_url" => UiCommand::SetUrl {
                value: need(self.value.clone(), "value")?,
            },
            "select_tab" => UiCommand::SelectTab {
                key: need(self.key.clone(), "key")?,
            },
            "choose_environment" => UiCommand::ChooseEnvironment {
                name: self.name.clone(),
            },
            other => return Err(format!("unknown command: {other}")),
        })
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct InvokeActionIn {
    /// Action name such as `fluxwire::Send`, or the bare `Send`.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct KeyIn {
    /// Keystroke string such as `cmd-enter` or `secondary-shift-p`.
    pub keys: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct TypeTextIn {
    pub text: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClickIn {
    pub x: f32,
    pub y: f32,
    /// 1 = click, 2 = double click.
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, JsonSchema)]
pub struct ScrollIn {
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
}

#[derive(Deserialize, JsonSchema)]
pub struct TreeIn {
    /// Only the subtree rooted at this element id.
    pub subtree: Option<String>,
    /// Keep only nodes of this role (and their ancestors), such as `button` or `tab`.
    pub role: Option<String>,
    /// Drop elements that are fully clipped or off screen.
    #[serde(default)]
    pub visible_only: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindIn {
    /// Case-insensitive substring of the element label.
    pub label: Option<String>,
    pub role: Option<String>,
    /// Case-insensitive substring of the label or the id.
    pub text: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClickElementIn {
    /// An element id from `ui.tree` or `ui.find`.
    pub id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitIdleIn {
    /// Default 5000.
    pub timeout_ms: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitForIn {
    /// JSON pointer into `ui.state`, such as `/response/status`.
    pub path: String,
    /// Satisfied when the value equals this. Omit to wait for a non-null value.
    pub equals: Option<serde_json::Value>,
    /// Default 5000.
    pub timeout_ms: Option<u64>,
}

/// Polls `check` every 25 ms until it returns `Some`, or errors at the timeout.
fn poll(
    ctx: &ToolCtx,
    timeout_ms: Option<u64>,
    what: &str,
    check: impl Fn(&serde_json::Value) -> bool,
    req: UiRequest,
) -> Result<serde_json::Value, String> {
    let end =
        std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));
    loop {
        let v = forward(ctx, req.clone())?;
        if check(&v) {
            return Ok(v);
        }
        if std::time::Instant::now() >= end {
            return Err(format!("timed out waiting for {what}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
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
    r.register(def_in::<NoParams, UiOut>(
        ToolClass::Ui,
        "ui.list_actions",
        "Every action: name, label, binding, context, palette flag and whether it is enabled now.",
        |ctx, _| forward(ctx, UiRequest::ListActions),
    ));
    r.register(def_in::<InvokeActionIn, UiOut>(
        ToolClass::Ui,
        "ui.invoke_action",
        "Invoke an action by name. Errors when the action is unknown or disabled. Returns the new generation and any notice.",
        |ctx, args| {
            let a: InvokeActionIn = parse(args)?;
            forward(ctx, UiRequest::InvokeAction { name: a.name })
        },
    ));
    r.register(def_in::<CommandIn, UiOut>(
        ToolClass::Ui,
        "ui.command",
        "Typed operation with arguments: open_file, set_url, select_tab, choose_environment (field `command`).",
        |ctx, args| {
            let c = parse::<CommandIn>(args)?.into_command()?;
            forward(ctx, UiRequest::Command { command: c })
        },
    ));
    r.register(def_in::<KeyIn, UiOut>(
        ToolClass::Ui,
        "ui.key",
        "Send a keystroke such as `cmd-enter`.",
        |ctx, args| {
            let a: KeyIn = parse(args)?;
            forward(ctx, UiRequest::Key { keys: a.keys })
        },
    ));
    r.register(def_in::<TypeTextIn, UiOut>(
        ToolClass::Ui,
        "ui.type_text",
        "Type text into the focused input.",
        |ctx, args| {
            let a: TypeTextIn = parse(args)?;
            forward(ctx, UiRequest::TypeText { text: a.text })
        },
    ));
    r.register(def_in::<ClickIn, UiOut>(
        ToolClass::Ui,
        "ui.click",
        "Click at window coordinates (device-independent pixels).",
        |ctx, args| {
            let a: ClickIn = parse(args)?;
            forward(
                ctx,
                UiRequest::Click {
                    x: a.x,
                    y: a.y,
                    count: a.count,
                },
            )
        },
    ));
    r.register(def_in::<ScrollIn, UiOut>(
        ToolClass::Ui,
        "ui.scroll",
        "Scroll the wheel at window coordinates.",
        |ctx, args| {
            let a: ScrollIn = parse(args)?;
            forward(
                ctx,
                UiRequest::Scroll {
                    x: a.x,
                    y: a.y,
                    dx: a.dx,
                    dy: a.dy,
                },
            )
        },
    ));
    r.register(def_in::<TreeIn, UiOut>(
        ToolClass::Ui,
        "ui.tree",
        "The semantic UI tree: stable ids, roles, labels, enabled, bounds, visible, clipped, focused and z-order, optionally filtered by subtree, role or visibility.",
        |ctx, args| {
            let a: TreeIn = parse(args)?;
            forward(
                ctx,
                UiRequest::Tree {
                    subtree: a.subtree,
                    role: a.role,
                    visible_only: a.visible_only,
                },
            )
        },
    ));
    r.register(def_in::<FindIn, UiOut>(
        ToolClass::Ui,
        "ui.find",
        "Find elements by label, role or text; returns ids with bounds and state.",
        |ctx, args| {
            let a: FindIn = parse(args)?;
            forward(
                ctx,
                UiRequest::Find {
                    label: a.label,
                    role: a.role,
                    text: a.text,
                },
            )
        },
    ));
    r.register(def_in::<ClickElementIn, UiOut>(
        ToolClass::Ui,
        "ui.click_element",
        "Click the centre of an element's visible bounds by id. Errors when it is unknown, disabled, hidden or covered.",
        |ctx, args| {
            let a: ClickElementIn = parse(args)?;
            forward(ctx, UiRequest::ClickElement { id: a.id })
        },
    ));
    r.register(def_in::<WaitIdleIn, UiOut>(
        ToolClass::Ui,
        "ui.wait_for_idle",
        "Wait until no run is pending and the UI state has stopped changing; errors on timeout.",
        |ctx, args| {
            let a: WaitIdleIn = parse(args)?;
            poll(
                ctx,
                a.timeout_ms,
                "idle",
                |v| v["idle"] == true,
                UiRequest::Idle,
            )
        },
    ));
    r.register(def_in::<WaitForIn, UiOut>(
        ToolClass::Ui,
        "ui.wait_for",
        "Wait until the JSON pointer `path` in ui.state equals `equals` (or is non-null); errors on timeout.",
        |ctx, args| {
            let a: WaitForIn = parse(args)?;
            let (path, want) = (a.path.clone(), a.equals.clone());
            poll(
                ctx,
                a.timeout_ms,
                &format!("{} to match", a.path),
                move |v| match (v.pointer(&path), &want) {
                    (Some(got), Some(w)) => got == w,
                    (Some(got), None) => !got.is_null(),
                    _ => false,
                },
                UiRequest::State,
            )
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
