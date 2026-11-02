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
pub struct ScreenshotIn {
    /// A named state from `fixtures/ui-states` (such as `env-modal`) or a list of steps
    /// (`open`, `invoke`, `key`, `type`, `command`, `set_theme`, `resize`, `wait_for`,
    /// `wait_idle`) applied before the capture.
    pub state: Option<crate::state::StateSpec>,
    /// Crop to the visible bounds of this element id from `ui.tree`.
    pub element: Option<String>,
    /// Volatile text to mask: `clock`, `timings` or `{x,y,w,h}` in points. Defaults to
    /// `["clock", "timings"]`; pass `[]` to mask nothing.
    pub mask: Option<Vec<crate::state::Mask>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ScreenshotAllIn {
    /// State names from `fixtures/ui-states`; empty keeps the current state.
    #[serde(default)]
    pub states: Vec<String>,
    /// Theme names such as `one-dark`, `one-light`; empty keeps the current theme.
    #[serde(default)]
    pub themes: Vec<String>,
    /// Sizes: `default` (1280x800), `compact` (900x600), `min` or `WxH`; empty keeps the size.
    #[serde(default)]
    pub sizes: Vec<String>,
    pub mask: Option<Vec<crate::state::Mask>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetThemeIn {
    /// Theme name such as `one-dark` or `one-light`. Not saved to settings.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ResizeIn {
    /// `default` (1280x800), `compact` (900x600), `min`, or `WxH` in points.
    pub size: String,
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
pub struct AuditIn {
    /// Rule ids to run, such as `UX101`; empty runs all.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Audit only the subtree rooted at this element id.
    pub subtree: Option<String>,
    /// State names from `fixtures/ui-states` to audit; empty audits the current state.
    #[serde(default)]
    pub states: Vec<String>,
    /// Themes such as `one-dark`, `one-light`; empty keeps the current theme.
    #[serde(default)]
    pub themes: Vec<String>,
    /// Sizes: `default`, `compact`, `min` or `WxH`; empty keeps the size.
    #[serde(default)]
    pub sizes: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WalkthroughIn {
    /// Steps as for `ui.screenshot` `state`, plus `{"step": "click", "id": "..."}`.
    pub steps: Vec<crate::state::Step>,
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

/// Runs `f` with a screenshot environment over the UI bridge. State directories are searched
/// in `FLUXWIRE_UI_STATES`, the working directory and the allowed roots.
fn with_env<T>(
    ctx: &ToolCtx,
    f: impl FnOnce(&crate::screenshot::Env) -> Result<T, String>,
) -> Result<T, String> {
    let ui = ctx
        .ui
        .as_ref()
        .ok_or_else(|| "no UI bridge in this host".to_string())?;
    let mut search = vec![];
    if let Some(d) = std::env::var_os("FLUXWIRE_UI_STATES") {
        // The variable names the `ui-states` directory itself; its search root is two levels up.
        let d = std::path::PathBuf::from(d);
        if let Some(root) = d.parent().and_then(|p| p.parent()) {
            search.push(root.to_path_buf());
        }
    }
    search.extend(std::env::current_dir());
    search.extend(ctx.roots.list().iter().cloned());
    f(&crate::screenshot::Env {
        ui: ui.as_ref(),
        pace: crate::screenshot::Pace::normal(),
        search,
    })
}

fn forward(ctx: &ToolCtx, req: UiRequest) -> Result<serde_json::Value, String> {
    let out = ctx
        .ui
        .as_ref()
        .ok_or_else(|| "no UI bridge in this host".to_string())?
        .call(req.clone());
    if out.is_ok()
        && let Ok(mut r) = crate::scenario::recorder().lock()
    {
        r.note(&req);
    }
    out
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsIn {
    /// Return events with a larger `seq`; 0 replays the whole buffer. Pass the `latest` of the
    /// previous answer to continue.
    #[serde(default)]
    pub since: u64,
    /// Kinds to return: `run`, `frame`, `notice`, `console`, `generation`; empty returns all.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Wait up to this long for the first matching event (default 0, at most 30000).
    pub wait_ms: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunScenarioIn {
    /// A scenario name from `fixtures/scenarios`, such as `m03-send`.
    pub name: Option<String>,
    /// Or a scenario given inline.
    pub scenario: Option<crate::scenario::Scenario>,
    /// Take screenshot checkpoints and a failure screenshot (default true).
    pub capture: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RecordIn {
    /// `start` clears and begins recording; `stop` returns the scenario as YAML.
    pub action: String,
    /// Scenario name for `stop` (default `recorded`).
    pub name: Option<String>,
    /// For `stop`: also write `<dir>/<name>.yaml` into `fixtures/scenarios` when it exists.
    #[serde(default)]
    pub save: bool,
}

fn scenario_dirs(env: &crate::screenshot::Env) -> Option<std::path::PathBuf> {
    env.search
        .iter()
        .find_map(|d| crate::scenario::find_scenario_dir(d))
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
    r.register(def_in::<ScreenshotIn, UiOut>(
        ToolClass::Ui,
        "ui.screenshot",
        "PNG of the app window, or of one element, optionally after applying a named or explicit `state`. Waits for the UI to settle, masks volatile text, and returns the image plus the `ui.state` JSON, size, theme, scale and capture tier. Needs a visible window; on macOS, Screen Recording permission.",
        |ctx, args| {
            let a: ScreenshotIn = parse(args)?;
            with_env(ctx, |env| {
                crate::screenshot::screenshot(
                    env,
                    a.state.as_ref(),
                    a.element.as_deref(),
                    a.mask.as_deref(),
                )
            })
        },
    ));
    r.register(def_in::<ScreenshotAllIn, UiOut>(
        ToolClass::Ui,
        "ui.screenshot_all",
        "Capture every combination of `states` x `themes` x `sizes` and return a labelled image per cell (`state/theme/size`). The theme and window size are restored afterwards.",
        |ctx, args| {
            let a: ScreenshotAllIn = parse(args)?;
            with_env(ctx, |env| {
                crate::screenshot::screenshot_all(
                    env,
                    &a.states,
                    &a.themes,
                    &a.sizes,
                    a.mask.as_deref(),
                )
            })
        },
    ));
    r.register(def_in::<SetThemeIn, UiOut>(
        ToolClass::Ui,
        "ui.set_theme",
        "Switch the theme (`one-dark`, `one-light` or any bundled theme name) without saving it to settings.",
        |ctx, args| {
            let a: SetThemeIn = parse(args)?;
            forward(ctx, UiRequest::SetTheme { name: Some(a.name) })
        },
    ));
    r.register(def_in::<ResizeIn, UiOut>(
        ToolClass::Ui,
        "ui.resize",
        "Resize the window to a preset (`default` 1280x800, `compact` 900x600, `min`) or `WxH` points; waits until the window reports the size.",
        |ctx, args| {
            let a: ResizeIn = parse(args)?;
            let size = crate::state::Size::parse(&a.size)?;
            with_env(ctx, |env| {
                crate::screenshot::resize(env, size)?;
                Ok(serde_json::json!({"size": size.label()}))
            })
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
    r.register(def_in::<AuditIn, UiOut>(
        ToolClass::Ui,
        "ui.audit",
        "Lint the live UI. Rules: UX101 clipped text without tooltip, UX102 overlapping siblings, UX103 small target, UX110 contrast, UX120 hard-coded colour, UX130 unreachable action, UX131 keyboard reachability and focus, UX140 missing empty/loading/error state, UX150 latency. Each finding has a rule, severity, message, element ids, bounds and a suggested fix. `states`, `themes` and `sizes` sweep a matrix and restore the theme and size afterwards.",
        |ctx, args| {
            let a: AuditIn = parse(args)?;
            with_env(ctx, |env| {
                crate::audit::audit(
                    env,
                    &crate::audit::AuditArgs {
                        rules: a.rules,
                        subtree: a.subtree,
                        states: a.states,
                        themes: a.themes,
                        sizes: a.sizes,
                    },
                )
            })
        },
    ));
    r.register(def_in::<WalkthroughIn, UiOut>(
        ToolClass::Ui,
        "ui.walkthrough",
        "Run a task as a list of steps and report each step's time, keystrokes, clicks and notices plus totals, to compare the cost of a task over time. Stops at the first failing step.",
        |ctx, args| {
            let a: WalkthroughIn = parse(args)?;
            with_env(ctx, |env| crate::audit::walkthrough(env, &a.steps))
        },
    ));
    r.register(def_in::<EventsIn, UiOut>(
        ToolClass::Ui,
        "ui.events",
        "Replay or wait on the event feed: runs, WebSocket frames, notices, console lines and state generation changes, each with a `seq`. Returns `{events, latest, gap}`; pass `latest` as `since` next time. `gap` is true when older events were already dropped. The server also pushes each event as an MCP logging notification (logger `fluxwire.events`).",
        |ctx, args| {
            if ctx.ui.is_none() {
                return Err("no UI bridge in this host".into());
            }
            let a: EventsIn = parse(args)?;
            let wait = std::time::Duration::from_millis(a.wait_ms.unwrap_or(0).min(30_000));
            let buf = crate::events::global();
            let events = if wait.is_zero() {
                buf.since(a.since, &a.kinds)
            } else {
                buf.wait(a.since, &a.kinds, wait)
            };
            let latest = events.last().map_or(buf.latest().max(a.since), |e| e.seq);
            Ok(serde_json::json!({
                "events": events,
                "latest": latest,
                "gap": buf.has_gap(a.since),
            }))
        },
    ));
    r.register(def_in::<RunScenarioIn, UiOut>(
        ToolClass::Ui,
        "ui.run_scenario",
        "Run a scenario (YAML in fixtures/scenarios, or inline) and return a pass/fail report: per-step status and time, and on failure the step, expected vs actual, the ui.state and a screenshot. Screenshot checkpoints are attached as images. Steps: the ui.screenshot steps plus click_element, assert (JSON pointer on ui.state), assert_find, screenshot, audit and wait_event.",
        |ctx, args| {
            let a: RunScenarioIn = parse(args)?;
            with_env(ctx, |env| {
                let (sc, repo) = match (a.scenario, a.name) {
                    (Some(sc), _) => (sc, None),
                    (None, Some(n)) => {
                        let dir = scenario_dirs(env)
                            .ok_or("no fixtures/scenarios directory found")?;
                        let repo = dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf());
                        (crate::scenario::load_named(&dir, &n)?, repo)
                    }
                    _ => return Err("give `name` or `scenario`".to_string()),
                };
                let repo = repo.or_else(|| env.search.first().cloned());
                Ok(crate::scenario::run(
                    env,
                    &crate::scenario::RunOpts {
                        capture: a.capture.unwrap_or(true),
                        events: crate::events::global(),
                        repo,
                    },
                    &sc,
                ))
            })
        },
    ));
    r.register(def_in::<RecordIn, UiOut>(
        ToolClass::Ui,
        "ui.record",
        "Record the state-changing ui.* calls that follow (open, invoke, key, type, command, click_element, set_theme, resize) as a scenario. `stop` returns the YAML with assertions taken from the final ui.state.",
        |ctx, args| {
            let a: RecordIn = parse(args)?;
            let mut rec = crate::scenario::recorder()
                .lock()
                .map_err(|_| "recorder poisoned".to_string())?;
            match a.action.as_str() {
                "start" => {
                    rec.start();
                    Ok(serde_json::json!({"recording": true}))
                }
                "stop" => {
                    if !rec.active() {
                        return Err("not recording; call ui.record with action `start` first".into());
                    }
                    let st = ctx.ui.as_ref().and_then(|u| u.call(UiRequest::State).ok());
                    let ws = st
                        .as_ref()
                        .and_then(|s| s["workspace_path"].as_str())
                        .map(String::from);
                    let name = a.name.unwrap_or_else(|| "recorded".into());
                    let sc = rec.stop(&name, ws, st.as_ref());
                    let yaml = sc.to_yaml()?;
                    let mut out = serde_json::json!({"name": name, "steps": sc.steps.len(), "yaml": yaml});
                    if a.save {
                        let ok = !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                        if !ok {
                            return Err(format!("invalid scenario name `{name}`"));
                        }
                        let dir = with_env(ctx, |env| Ok(scenario_dirs(env)))?
                            .ok_or("no fixtures/scenarios directory found")?;
                        let path = dir.join(format!("{name}.yaml"));
                        std::fs::write(&path, &yaml).map_err(|e| e.to_string())?;
                        out["saved"] = path.display().to_string().into();
                    }
                    Ok(out)
                }
                other => Err(format!("unknown action `{other}`: use start or stop")),
            }
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
