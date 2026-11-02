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
    /// Bring the window to the front (the tool layer then waits for it to settle).
    Activate,
    /// Capture the window, or the visible bounds of `element`, as a PNG. The answer carries the
    /// base64 PNG, region, scale, tier, `ui.state` and the rectangles of volatile elements.
    Capture { element: Option<String> },
    /// Set the theme by name without saving it to settings; `None` restores the configured one.
    SetTheme { name: Option<String> },
    /// Resize the window to `width` x `height` points.
    Resize { width: f32, height: f32 },
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
    /// The semantic UI tree, optionally filtered (M9.5).
    Tree {
        subtree: Option<String>,
        role: Option<String>,
        visible_only: bool,
    },
    /// Elements matching a label, role or text.
    Find {
        label: Option<String>,
        role: Option<String>,
        text: Option<String>,
    },
    /// Click the centre of an element's visible bounds.
    ClickElement { id: String },
    /// Run the UI audit rules over the current tree (M9.7). `theme` audits against that theme's
    /// tokens (the app's own when `None`); `fixtures` are the state fixture names that exist.
    Audit {
        rules: Vec<String>,
        subtree: Option<String>,
        theme: Option<String>,
        fixtures: Vec<String>,
    },
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
