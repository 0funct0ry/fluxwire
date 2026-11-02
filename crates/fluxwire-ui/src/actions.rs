//! Every visible action, registered with GPUI and listed in `ACTIONS` so the command palette,
//! keymap and tests read from one table (SPEC §17.5, §17.6). Most are no-ops in M1.

use crate::input_keys::*;
use gpui::{Action, App, KeyBinding, actions};

actions!(
    fluxwire,
    [
        Send,
        EditorToggleComment,
        RequestFormatBody,
        EditorMoveLineUp,
        EditorMoveLineDown,
        EditorDuplicateLine,
        EditorDeleteLine,
        EditorGoToLine,
        EditorJumpToBracket,
        EditorShowCompletions,
        EditorDismissCompletions,
        EditorToggleSoftWrap,
        CancelRequest,
        CopyAsCurl,
        CopyAsCurlWithSecrets,
        FindInResponse,
        CopyResponseBody,
        SaveResponseBody,
        QuickSearch,
        CommandPalette,
        NewRequest,
        NewFolder,
        NewWebSocketRequest,
        QuickRequest,
        RenameItem,
        DuplicateItem,
        DeleteItem,
        RevealInFileManager,
        ReloadFromDisk,
        KeepMine,
        NewCollection,
        AddCollection,
        OpenWorkspace,
        NewWorkspace,
        SwitchWorkspace,
        Import,
        Save,
        RunFolder,
        SwitchEnvironment,
        ToggleLeftDock,
        ToggleBottomDock,
        FocusUrlBar,
        CloseTab,
        PinTab,
        NextTab,
        PrevTab,
        ReopenClosedTab,
        GroupTabsByKind,
        ToggleSplitOrientation,
        ToggleTheme,
        SelectTheme,
        ShowWorkspacePanel,
        ShowHistoryPanel,
        HistoryOpen,
        HistoryFocusSearch,
        HistoryPin,
        HistoryExport,
        ClearHistoryAll,
        ClearHistoryCollection,
        ClearHistoryRequest,
        ShowEnvironmentsPanel,
        ShowSyncPanel,
        ShowConsole,
        ShowProblems,
        OpenAsText,
        CloseWorkspace,
        OpenDocs,
        EditActiveEnvironment,
        ManageEnvironments,
        OpenSync,
        Connect,
        Dismiss,
        Quit,
        CloseOtherTabs,
        CloseTabsToRight,
        CloseCleanTabs,
        CloseAllTabs,
        CopyTabPath,
        ShowParams,
        ShowHeaders,
        ShowBody,
        ShowAuth,
        ShowScripts,
        ShowTests,
        ShowDocsTab,
        ShowSettings,
        ShowResponseBody,
        ShowResponseHeaders,
        ShowResponseCookies,
        ShowResponseTimeline,
        ShowResponseTests,
        ShowResponseConsole,
        ToggleMethodMenu,
        NewEnvironment,
        RenameEnvironment,
        DuplicateEnvironment,
        DeleteEnvironment,
        NewWindow,
        OpenKeybindings,
        ReloadKeymap,
        OpenKeymapFile,
        OpenSettingsFile,
        TrustCollection,
        RevokeCollectionTrust,
        RegenerateFwTypes,
        OpenResultBeside,
        NewWorkspaceCollection,
        CopyWorkspacePath,
        EditKeymap,
        InstallCliShim,
        UninstallCliShim,
    ]
);

pub struct ActionInfo {
    /// Palette label.
    pub label: &'static str,
    /// GPUI action name, e.g. `fluxwire::Send`.
    pub name: &'static str,
    /// Default binding (`secondary` = Cmd on macOS, Ctrl elsewhere), if any.
    pub binding: Option<&'static str>,
    /// Key context the binding applies to; `None` = global.
    pub context: Option<&'static str>,
    /// Listed in the command palette; hidden actions stay bindable and in the Keybindings tab.
    pub palette: bool,
    pub build: fn() -> Box<dyn Action>,
    bind: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! action_row {
    ($palette:literal, $label:literal, $ty:ident, $binding:expr, $ctx:expr) => {
        ActionInfo {
            label: $label,
            name: concat!("fluxwire::", stringify!($ty)),
            binding: $binding,
            context: $ctx,
            palette: $palette,
            build: || Box::new($ty),
            bind: |keys, ctx| KeyBinding::new(keys, $ty, ctx),
        }
    };
}

/// A palette-visible action.
macro_rules! a {
    ($($t:tt)*) => { action_row!(true, $($t)*) };
}

/// An action kept out of the palette (key-bound or sub-view actions, ADR-014).
macro_rules! h {
    ($($t:tt)*) => { action_row!(false, $($t)*) };
}

pub static ACTIONS: &[ActionInfo] = &[
    h!("request: send", Send, Some("secondary-enter"), None),
    h!(
        "request: cancel",
        CancelRequest,
        Some("escape"),
        Some("RequestEditor")
    ),
    h!(
        "request: copy as cURL",
        CopyAsCurl,
        Some("secondary-shift-c"),
        Some("RequestEditor")
    ),
    h!(
        "request: copy as cURL (with secrets)",
        CopyAsCurlWithSecrets,
        None,
        Some("RequestEditor")
    ),
    a!("response: find", FindInResponse, Some("secondary-f"), None),
    a!("response: copy body", CopyResponseBody, None, None),
    a!("response: save body to file", SaveResponseBody, None, None),
    a!("quick search: open", QuickSearch, Some("secondary-k"), None),
    a!(
        "command palette: toggle",
        CommandPalette,
        Some("secondary-shift-p"),
        None
    ),
    h!(
        "workspace: new request",
        NewRequest,
        Some("secondary-n"),
        None
    ),
    a!("workspace: quick request", QuickRequest, None, None),
    h!("workspace: new folder", NewFolder, None, None),
    h!(
        "workspace: new WebSocket request",
        NewWebSocketRequest,
        None,
        None
    ),
    a!("workspace: rename", RenameItem, None, None),
    a!("workspace: duplicate", DuplicateItem, None, None),
    a!("workspace: delete (move to trash)", DeleteItem, None, None),
    a!(
        "workspace: reveal in file manager",
        RevealInFileManager,
        None,
        None
    ),
    h!("file: reload from disk", ReloadFromDisk, None, None),
    h!("file: keep my changes", KeepMine, None, None),
    a!("collection: new…", NewCollection, None, None),
    a!("collection: add existing…", AddCollection, None, None),
    a!("workspace: open…", OpenWorkspace, Some("secondary-o"), None),
    a!("workspace: switch…", SwitchWorkspace, None, None),
    a!(
        "workspace: new…",
        NewWorkspace,
        Some("secondary-shift-n"),
        None
    ),
    a!("collection: import…", Import, None, None),
    h!("file: save", Save, Some("secondary-s"), None),
    a!(
        "run: run folder",
        RunFolder,
        Some("secondary-shift-r"),
        None
    ),
    a!(
        "environment: switch",
        SwitchEnvironment,
        Some("secondary-e"),
        None
    ),
    a!(
        "view: toggle sidebar",
        ToggleLeftDock,
        Some("secondary-b"),
        None
    ),
    a!(
        "view: toggle console",
        ToggleBottomDock,
        Some("secondary-j"),
        None
    ),
    h!(
        "request: focus URL bar",
        FocusUrlBar,
        Some("secondary-l"),
        None
    ),
    a!("tab: close", CloseTab, Some("secondary-w"), None),
    a!("tab: pin or unpin", PinTab, None, None),
    h!("tab: next", NextTab, Some("ctrl-tab"), None),
    h!("tab: previous", PrevTab, Some("ctrl-shift-tab"), None),
    h!(
        "tab: reopen closed",
        ReopenClosedTab,
        Some("secondary-shift-t"),
        None
    ),
    h!("tab: group by kind", GroupTabsByKind, None, None),
    a!(
        "view: toggle split orientation",
        ToggleSplitOrientation,
        Some("secondary-alt-l"),
        None
    ),
    a!("theme: toggle", ToggleTheme, None, None),
    a!("theme: select…", SelectTheme, None, None),
    a!("panel: workspace", ShowWorkspacePanel, None, None),
    a!("panel: history", ShowHistoryPanel, None, None),
    a!(
        "history: open",
        HistoryOpen,
        Some("secondary-shift-h"),
        None
    ),
    h!("history: focus search", HistoryFocusSearch, None, None),
    a!("history: pin", HistoryPin, None, None),
    a!("history: export", HistoryExport, None, None),
    a!("history: clear all", ClearHistoryAll, None, None),
    a!(
        "history: clear collection",
        ClearHistoryCollection,
        None,
        None
    ),
    a!("history: clear request", ClearHistoryRequest, None, None),
    a!("panel: environments", ShowEnvironmentsPanel, None, None),
    a!("panel: sync", ShowSyncPanel, None, None),
    a!("panel: console", ShowConsole, None, None),
    a!("panel: problems", ShowProblems, None, None),
    a!("request: open as text", OpenAsText, None, None),
    a!(
        "request: format body",
        RequestFormatBody,
        Some("alt-shift-f"),
        None
    ),
    a!(
        "workspace: close",
        CloseWorkspace,
        Some("secondary-shift-w"),
        None
    ),
    a!("docs: open for request", OpenDocs, None, None),
    h!(
        "environment: edit active",
        EditActiveEnvironment,
        None,
        None
    ),
    a!("environment: manage", ManageEnvironments, None, None),
    a!("sync: open", OpenSync, None, None),
    h!("websocket: connect or disconnect", Connect, None, None),
    h!("modal: dismiss", Dismiss, Some("escape"), None),
    a!("app: quit", Quit, Some("secondary-q"), None),
    a!("tab: close others", CloseOtherTabs, None, None),
    a!("tab: close to the right", CloseTabsToRight, None, None),
    h!("tab: close clean tabs", CloseCleanTabs, None, None),
    a!("tab: close all", CloseAllTabs, None, None),
    a!("tab: copy path", CopyTabPath, None, None),
    a!("request: show params", ShowParams, None, None),
    a!("request: show headers", ShowHeaders, None, None),
    a!("request: show body", ShowBody, None, None),
    a!("request: show auth", ShowAuth, None, None),
    a!("request: show scripts", ShowScripts, None, None),
    a!("request: show tests", ShowTests, None, None),
    a!("request: show docs", ShowDocsTab, None, None),
    a!("request: show settings", ShowSettings, None, None),
    h!("response: show body", ShowResponseBody, None, None),
    h!("response: show headers", ShowResponseHeaders, None, None),
    h!("response: show cookies", ShowResponseCookies, None, None),
    h!("response: show timeline", ShowResponseTimeline, None, None),
    h!("response: show tests", ShowResponseTests, None, None),
    h!("response: show console", ShowResponseConsole, None, None),
    h!("request: choose method", ToggleMethodMenu, None, None),
    a!("environment: new…", NewEnvironment, None, None),
    a!("environment: rename…", RenameEnvironment, None, None),
    a!("environment: duplicate", DuplicateEnvironment, None, None),
    a!("environment: delete…", DeleteEnvironment, None, None),
    a!("workspace: new window", NewWindow, None, None),
    a!("keymap: show", OpenKeybindings, None, None),
    a!("keymap: edit", EditKeymap, None, None),
    a!(
        "workspace: new collection",
        NewWorkspaceCollection,
        None,
        None
    ),
    a!("workspace: copy path", CopyWorkspacePath, None, None),
    a!("cli: install shell command", InstallCliShim, None, None),
    a!("cli: uninstall shell command", UninstallCliShim, None, None),
    h!("keymap: reload", ReloadKeymap, None, None),
    h!("keymap: open keymap.toml", OpenKeymapFile, None, None),
    a!("settings: show", OpenSettingsFile, None, None),
    a!("scripts: trust collection", TrustCollection, None, None),
    a!("scripts: revoke trust", RevokeCollectionTrust, None, None),
    a!("scripts: regenerate fw.d.ts", RegenerateFwTypes, None, None),
    a!(
        "quick search: open result beside",
        OpenResultBeside,
        Some("secondary-enter"),
        Some("Modal")
    ),
    h!(
        "input: move left",
        InputLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move right",
        InputRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move word left",
        InputWordLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move word right",
        InputWordRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move to line start",
        InputLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move to line end",
        InputLineEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move up",
        InputUp,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move down",
        InputDown,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move to start",
        InputDocStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: move to end",
        InputDocEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select left",
        InputSelectLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select right",
        InputSelectRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select word left",
        InputSelectWordLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select word right",
        InputSelectWordRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select to line start",
        InputSelectLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select to line end",
        InputSelectLineEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select up",
        InputSelectUp,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select down",
        InputSelectDown,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select to start",
        InputSelectDocStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select to end",
        InputSelectDocEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: select all",
        InputSelectAll,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: copy",
        InputCopy,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: cut",
        InputCut,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: paste",
        InputPaste,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: undo",
        InputUndo,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: redo",
        InputRedo,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: delete backward",
        InputBackspace,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: delete forward",
        InputDelete,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: delete word backward",
        InputDeleteWordBack,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: delete word forward",
        InputDeleteWordForward,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: delete to line start",
        InputDeleteToLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "input: context menu",
        InputContextMenu,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    h!(
        "editor: toggle comment",
        EditorToggleComment,
        Some("secondary-/"),
        Some("CodeEditor")
    ),
    h!(
        "editor: move line up",
        EditorMoveLineUp,
        Some("alt-up"),
        Some("CodeEditor")
    ),
    h!(
        "editor: move line down",
        EditorMoveLineDown,
        Some("alt-down"),
        Some("CodeEditor")
    ),
    h!(
        "editor: duplicate line",
        EditorDuplicateLine,
        Some("secondary-shift-d"),
        Some("CodeEditor")
    ),
    h!(
        "editor: delete line",
        EditorDeleteLine,
        Some("secondary-shift-k"),
        Some("CodeEditor")
    ),
    a!(
        "editor: go to line…",
        EditorGoToLine,
        Some("ctrl-g"),
        Some("CodeEditor")
    ),
    h!(
        "editor: jump to matching bracket",
        EditorJumpToBracket,
        Some("ctrl-m"),
        Some("CodeEditor")
    ),
    h!(
        "editor: show completions",
        EditorShowCompletions,
        Some("ctrl-space"),
        Some("CodeEditor")
    ),
    h!(
        "editor: dismiss completions",
        EditorDismissCompletions,
        Some("escape"),
        Some("CodeEditor")
    ),
    a!(
        "editor: toggle soft wrap",
        EditorToggleSoftWrap,
        Some("alt-z"),
        Some("CodeEditor")
    ),
];

/// Old action names (Rust names and palette labels) that a user `keymap.toml` may still use;
/// they map to the current action without a diagnostic. Kept for v1.0.
const ALIASES: &[(&str, &str)] = &[
    ("OpenEnvironment", "EditActiveEnvironment"),
    ("environment: open editor", "EditActiveEnvironment"),
    ("panel: project", "ShowWorkspacePanel"), // legacy-project alias
    ("ShowProjectPanel", "ShowWorkspacePanel"), // legacy-project alias
    ("OpenFolder", "OpenWorkspace"),
    ("workspace: open folder", "OpenWorkspace"),
    ("workspace: new collection", "NewWorkspaceCollection"),
    ("command palette: open", "CommandPalette"),
    ("view: toggle left dock", "ToggleLeftDock"),
    ("view: toggle bottom dock", "ToggleBottomDock"),
    ("keymap: open keybindings", "OpenKeybindings"),
    ("settings: open settings.toml", "OpenSettingsFile"),
    ("scripts: trust this collection", "TrustCollection"),
    (
        "scripts: revoke trust for this collection",
        "RevokeCollectionTrust",
    ),
    ("workspace: import", "Import"),
    ("workspace: close (show welcome)", "CloseWorkspace"),
];

/// The level an action belongs to (SPEC §17.9). An action is available only when its level is
/// open: App always, Workspace with an open Workspace, Collection with at least one Collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    App,
    Workspace,
    Collection,
}

const APP_ACTIONS: &[&str] = &[
    "OpenWorkspace",
    "NewWorkspace",
    "SwitchWorkspace",
    "NewWindow",
    "CommandPalette",
    "ToggleTheme",
    "SelectTheme",
    "OpenSettingsFile",
    "OpenKeymapFile",
    "ReloadKeymap",
    "Dismiss",
    "Quit",
];

const WORKSPACE_ACTIONS: &[&str] = &[
    "SwitchEnvironment",
    "NewEnvironment",
    "EditActiveEnvironment",
    "ManageEnvironments",
    "RenameEnvironment",
    "DuplicateEnvironment",
    "DeleteEnvironment",
    "CloseWorkspace",
    "NewCollection",
    "NewRequest",
    "NewFolder",
    "NewWebSocketRequest",
    "QuickRequest",
    "AddCollection",
    "Import",
    "QuickSearch",
    "OpenKeybindings",
    "EditKeymap",
    "NewWorkspaceCollection",
    "CopyWorkspacePath",
    "InstallCliShim",
    "UninstallCliShim",
    "RevealInFileManager",
    "ToggleLeftDock",
    "ToggleBottomDock",
    "ToggleSplitOrientation",
    "ShowWorkspacePanel",
    "ShowHistoryPanel",
    "HistoryOpen",
    "HistoryFocusSearch",
    "HistoryPin",
    "HistoryExport",
    "ClearHistoryAll",
    "ClearHistoryCollection",
    "ClearHistoryRequest",
    "ShowSyncPanel",
    "ShowConsole",
    "ShowProblems",
    "CloseTab",
    "PinTab",
    "NextTab",
    "PrevTab",
    "ReopenClosedTab",
    "GroupTabsByKind",
    "CloseOtherTabs",
    "CloseTabsToRight",
    "CloseCleanTabs",
    "CloseAllTabs",
    "CopyTabPath",
];

pub fn scope_of(name: &str) -> Scope {
    let short = name.rsplit("::").next().unwrap_or(name);
    if APP_ACTIONS.contains(&short) || input_action_names().any(|n| n == short) {
        Scope::App
    } else if WORKSPACE_ACTIONS.contains(&short) {
        Scope::Workspace
    } else {
        Scope::Collection
    }
}

/// Finds an action by `fluxwire::Send`, bare `Send`, or a namespaced alias such as
/// `request::Send` (SPEC §17.6): only the last path segment has to match.
pub fn find_action(name: &str) -> Option<&'static ActionInfo> {
    let name = ALIASES
        .iter()
        .find(|(old, _)| *old == name.trim() || name.trim().rsplit("::").next() == Some(*old))
        .map_or(name, |(_, new)| new);
    let short = name.trim().rsplit("::").next().unwrap_or_default();
    if short.is_empty() {
        return None;
    }
    ACTIONS
        .iter()
        .find(|a| a.name.rsplit("::").next() == Some(short))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingSource {
    Default,
    User,
}

/// One effective binding, for the Keybindings tab and the palette.
#[derive(Clone, Debug)]
pub struct BindingRow {
    pub action: &'static str,
    pub label: &'static str,
    pub keys: String,
    pub context: Option<String>,
    pub source: BindingSource,
}

/// A warning about `keymap.toml`, shown in Problems.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapIssue {
    pub line: usize,
    pub code: &'static str,
    pub message: String,
}

/// What is bound right now, and what is wrong with the user's file.
#[derive(Clone, Debug, Default)]
pub struct KeymapState {
    pub rows: Vec<BindingRow>,
    pub issues: Vec<KeymapIssue>,
}

impl gpui::Global for KeymapState {}

impl KeymapState {
    /// Display label of the first binding of `action` (`fluxwire::Send`), if any.
    pub fn label_for(&self, action: &str) -> Option<String> {
        self.rows
            .iter()
            .find(|r| r.action == action)
            .map(|r| crate::ui::kbd_label(&r.keys))
    }
}

pub(crate) fn norm_keys(keys: &str) -> String {
    keys.split_whitespace()
        .map(|k| {
            let mut parts: Vec<String> = k.to_lowercase().split('-').map(str::to_string).collect();
            let key = parts.pop().unwrap_or_default();
            for p in &mut parts {
                if p == "secondary" {
                    *p = if cfg!(target_os = "macos") {
                        "cmd"
                    } else {
                        "ctrl"
                    }
                    .into();
                }
            }
            parts.sort();
            parts.push(key);
            parts.join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Defaults plus the user's overrides. A user binding for an action replaces that action's
/// default bindings; an empty action (or `none`) unbinds the keystroke. Invalid entries become
/// issues and are skipped.
pub fn resolve_keymap(
    user: &fluxwire_format::Keymap,
) -> (Vec<(KeyBindingSpec, &'static ActionInfo)>, KeymapState) {
    let mut issues: Vec<KeymapIssue> = user
        .errors
        .iter()
        .map(|(line, m)| KeymapIssue {
            line: *line,
            code: "K001",
            message: m.clone(),
        })
        .collect();
    let mut valid: Vec<(&fluxwire_format::KeymapEntry, Option<&'static ActionInfo>)> = Vec::new();
    for e in &user.entries {
        if e.keystroke.split_whitespace().next().is_none()
            || e.keystroke
                .split_whitespace()
                .any(|k| gpui::Keystroke::parse(k).is_err())
        {
            issues.push(KeymapIssue {
                line: e.line,
                code: "K004",
                message: format!("`{}` is not a valid keystroke", e.keystroke),
            });
            continue;
        }
        if let Some(c) = &e.context
            && gpui::KeyBindingContextPredicate::parse(c).is_err()
        {
            issues.push(KeymapIssue {
                line: e.line,
                code: "K005",
                message: format!("`{c}` is not a valid key context"),
            });
            continue;
        }
        let a = e.action.trim();
        if a.is_empty() || a.eq_ignore_ascii_case("none") {
            valid.push((e, None));
        } else if let Some(info) = find_action(a) {
            valid.push((e, Some(info)));
        } else {
            issues.push(KeymapIssue {
                line: e.line,
                code: "K003",
                message: format!("unknown action `{a}`"),
            });
        }
    }
    let rebound: Vec<&str> = valid
        .iter()
        .filter_map(|(_, i)| i.map(|i| i.name))
        .collect();
    let mut out: Vec<(KeyBindingSpec, &'static ActionInfo)> = Vec::new();
    for (a, keys) in default_bindings() {
        if rebound.contains(&a.name) {
            continue;
        }
        let nk = norm_keys(keys);
        let unbound = valid.iter().any(|(e, i)| {
            i.is_none() && norm_keys(&e.keystroke) == nk && e.context.as_deref() == a.context
        });
        // A user binding for another action on the same key and context shadows the default.
        let shadowed = valid.iter().any(|(e, i)| {
            i.is_some_and(|i| i.name != a.name)
                && norm_keys(&e.keystroke) == nk
                && e.context.as_deref() == a.context
        });
        if !unbound && !shadowed {
            out.push((
                KeyBindingSpec {
                    keys: keys.to_string(),
                    context: a.context.map(str::to_string),
                    source: BindingSource::Default,
                    line: 0,
                },
                a,
            ));
        }
    }
    for (e, info) in &valid {
        if let Some(info) = info {
            out.push((
                KeyBindingSpec {
                    keys: e.keystroke.clone(),
                    context: e.context.clone(),
                    source: BindingSource::User,
                    line: e.line,
                },
                info,
            ));
        }
    }
    // Same keystroke and context on two actions where the user is involved: the later one wins.
    for (i, (spec, info)) in out.iter().enumerate() {
        if spec.source != BindingSource::User {
            continue;
        }
        let nk = norm_keys(&spec.keys);
        if let Some((_, other)) = out.iter().enumerate().find(|(j, (o, oi))| {
            *j != i && norm_keys(&o.keys) == nk && o.context == spec.context && oi.name != info.name
        }) {
            issues.push(KeymapIssue {
                line: spec.line,
                code: "K002",
                message: format!(
                    "`{}` is bound to both `{}` and `{}`; the later binding wins",
                    spec.keys, info.label, other.1.label
                ),
            });
        }
    }
    let rows = out
        .iter()
        .map(|(spec, info)| BindingRow {
            action: info.name,
            label: info.label,
            keys: spec.keys.clone(),
            context: spec.context.clone(),
            source: spec.source,
        })
        .collect();
    (out, KeymapState { rows, issues })
}

#[derive(Clone, Debug)]
pub struct KeyBindingSpec {
    pub keys: String,
    pub context: Option<String>,
    pub source: BindingSource,
    pub line: usize,
}

/// Rebinds every key from the defaults and `user`, live. Returns the issues found.
pub fn apply_keymap(cx: &mut App, user: &fluxwire_format::Keymap) {
    let (bindings, state) = resolve_keymap(user);
    cx.clear_key_bindings();
    cx.bind_keys(
        bindings
            .iter()
            .map(|(spec, info)| (info.bind)(&spec.keys, spec.context.as_deref())),
    );
    cx.set_global(state);
}

pub fn key_bindings() -> Vec<KeyBinding> {
    default_bindings()
        .map(|(a, keys)| (a.bind)(keys, a.context))
        .collect()
}

/// Every default (action, keystroke) pair: the table's single bindings plus the platform text
/// input keymap, which binds several keystrokes per action.
pub(crate) fn default_bindings() -> impl Iterator<Item = (&'static ActionInfo, &'static str)> {
    default_bindings_for(cfg!(target_os = "macos"))
}

fn default_bindings_for(macos: bool) -> impl Iterator<Item = (&'static ActionInfo, &'static str)> {
    let table = ACTIONS
        .iter()
        .filter_map(|a| a.binding.map(|keys| (a, keys)));
    let input = input_keymap(macos)
        .into_iter()
        .filter_map(|(keys, name)| Some((find_action(name)?, keys)));
    table.chain(input)
}

/// Registers the default bindings with GPUI (the user keymap is applied by the workspace).
pub fn install(cx: &mut App) {
    apply_keymap(cx, &fluxwire_format::Keymap::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The curated palette (M9.04): every visible label, from the KEEP rows of the entries sheet
    /// plus the new entries. Hidden actions must not appear here.
    const PALETTE: &[&str] = &[
        "request: open as text",
        "request: show params",
        "request: show headers",
        "request: show body",
        "request: show auth",
        "request: show scripts",
        "request: show tests",
        "request: show docs",
        "request: format body",
        "editor: go to line…",
        "editor: toggle soft wrap",
        "request: show settings",
        "response: find",
        "response: copy body",
        "response: save body to file",
        "quick search: open",
        "quick search: open result beside",
        "command palette: toggle",
        "workspace: quick request",
        "workspace: rename",
        "workspace: duplicate",
        "workspace: delete (move to trash)",
        "workspace: reveal in file manager",
        "workspace: open…",
        "workspace: switch…",
        "workspace: new…",
        "workspace: close",
        "workspace: new window",
        "collection: new…",
        "collection: add existing…",
        "collection: import…",
        "run: run folder",
        "environment: switch",
        "environment: manage",
        "environment: new…",
        "environment: rename…",
        "environment: duplicate",
        "environment: delete…",
        "view: toggle sidebar",
        "view: toggle console",
        "view: toggle split orientation",
        "tab: close",
        "tab: pin or unpin",
        "tab: close others",
        "tab: close to the right",
        "tab: close all",
        "tab: copy path",
        "theme: toggle",
        "theme: select…",
        "panel: workspace",
        "panel: history",
        "panel: environments",
        "panel: sync",
        "panel: console",
        "panel: problems",
        "history: open",
        "history: pin",
        "history: export",
        "history: clear all",
        "history: clear collection",
        "history: clear request",
        "docs: open for request",
        "sync: open",
        "app: quit",
        "keymap: show",
        "settings: show",
        "scripts: trust collection",
        "scripts: revoke trust",
        "scripts: regenerate fw.d.ts",
        "workspace: new collection",
        "workspace: copy path",
        "keymap: edit",
        "cli: install shell command",
        "cli: uninstall shell command",
    ];

    #[test]
    fn palette_entries_are_exactly_the_curated_list() {
        let mut shown: Vec<_> = ACTIONS
            .iter()
            .filter(|a| a.palette)
            .map(|a| a.label)
            .collect();
        let mut want = PALETTE.to_vec();
        shown.sort();
        want.sort();
        assert_eq!(shown, want);
    }

    #[test]
    fn input_actions_and_modal_dismiss_are_hidden_but_registered() {
        for a in ACTIONS
            .iter()
            .filter(|a| a.label.starts_with("input: ") || a.label == "modal: dismiss")
        {
            assert!(!a.palette, "{}", a.label);
            assert_eq!(find_action(a.name).unwrap().name, a.name);
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = ACTIONS.iter().map(|a| a.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ACTIONS.len());
    }

    #[test]
    fn spec_default_bindings_exist() {
        for (label, keys) in [
            ("request: send", "secondary-enter"),
            ("quick search: open", "secondary-k"),
            ("command palette: toggle", "secondary-shift-p"),
            ("workspace: new request", "secondary-n"),
            ("file: save", "secondary-s"),
            ("run: run folder", "secondary-shift-r"),
            ("environment: switch", "secondary-e"),
            ("view: toggle sidebar", "secondary-b"),
            ("view: toggle console", "secondary-j"),
            ("request: focus URL bar", "secondary-l"),
            ("workspace: close", "secondary-shift-w"),
            ("workspace: new…", "secondary-shift-n"),
            ("workspace: open…", "secondary-o"),
        ] {
            let a = ACTIONS.iter().find(|a| a.label == label).unwrap();
            assert_eq!(a.binding, Some(keys), "{label}");
        }
    }

    #[test]
    fn default_bindings_do_not_conflict() {
        let mut seen = std::collections::HashMap::new();
        for a in ACTIONS {
            if let Some(k) = a.binding
                && let Some(prev) = seen.insert((k, a.context), a.label)
            {
                panic!("{k} is bound to both {prev:?} and {:?}", a.label);
            }
        }
    }

    #[test]
    fn old_action_names_are_aliases_and_scopes_are_assigned() {
        for (old, label) in [
            ("OpenFolder", "workspace: open…"),
            ("workspace: open folder", "workspace: open…"),
            ("workspace: new collection", "workspace: new collection"),
            ("command palette: open", "command palette: toggle"),
            ("view: toggle left dock", "view: toggle sidebar"),
            ("workspace: import", "collection: import…"),
            ("workspace: close (show welcome)", "workspace: close"),
            ("fluxwire::OpenFolder", "workspace: open…"),
        ] {
            assert_eq!(find_action(old).unwrap().label, label, "{old}");
        }
        assert_eq!(scope_of("fluxwire::OpenWorkspace"), Scope::App);
        assert_eq!(scope_of("fluxwire::InputCopy"), Scope::App);
        assert_eq!(scope_of("fluxwire::CloseWorkspace"), Scope::Workspace);
        assert_eq!(scope_of("fluxwire::Send"), Scope::Collection);
    }
}
