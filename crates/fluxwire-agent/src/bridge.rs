//! The seam between the tool layer and the live app (ADR-009). `fluxwire-ui` implements it
//! behind `dev-mcp`; the headless server has none, so the `ui` class is never exposed there.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One request from a `ui.*` tool to the running app.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum UiRequest {
    /// Snapshot of the running app.
    State,
    /// Open a Workspace directory.
    OpenWorkspace { path: String },
    /// Open a Request by its Workspace-relative key.
    OpenRequest { key: String, permanent: bool },
    /// Prototype of window capture (M9.6 replaces it): write a PNG of the window to `path`.
    CapturePrototype { path: String },
    /// Every registered action with its binding, context and whether it is enabled now.
    ListActions,
    /// Dispatch an action by name; errors when unknown or disabled.
    InvokeAction { name: String },
    /// A typed operation that needs arguments.
    Command { command: UiCommand },
    /// Dispatch a keystroke such as `cmd-enter`.
    Key { keys: String },
    /// Type text into the focused input.
    TypeText { text: String },
    /// Click at window coordinates.
    Click { x: f32, y: f32, count: u32 },
    /// Scroll the wheel at window coordinates.
    Scroll { x: f32, y: f32, dx: f32, dy: f32 },
    /// `{idle, generation}`; the tool layer polls it for `ui.wait_for_idle`.
    Idle,
}

/// Operations that need arguments; they call the same Workspace methods the UI uses.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum UiCommand {
    OpenFile {
        key: String,
        permanent: bool,
    },
    SetUrl {
        value: String,
    },
    SelectTab {
        key: String,
    },
    /// `None` clears the active environment.
    ChooseEnvironment {
        name: Option<String>,
    },
}

/// Calls cross to the GPUI thread inside the implementation; the tool layer never touches
/// GPUI entities.
pub trait UiBridge: Send + Sync {
    /// A short name of the host window, for `server_info`.
    fn describe(&self) -> String;

    /// Runs `req` on the UI thread and returns its JSON result, or an error message (including
    /// a timeout when the UI thread does not answer).
    fn call(&self, req: UiRequest) -> Result<serde_json::Value, String>;
}
