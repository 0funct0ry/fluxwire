//! Every visible action, registered with GPUI and listed in `ACTIONS` so the command palette,
//! keymap and tests read from one table (SPEC §17.5, §17.6). Most are no-ops in M1.

use crate::input_keys::*;
use gpui::{Action, App, KeyBinding, actions};

actions!(
    fluxwire,
    [
        Send,
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
        ShowWorkspacePanel,
        ShowHistoryPanel,
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
    pub build: fn() -> Box<dyn Action>,
    bind: fn(&str, Option<&str>) -> KeyBinding,
}

macro_rules! a {
    ($label:literal, $ty:ident, $binding:expr, $ctx:expr) => {
        ActionInfo {
            label: $label,
            name: concat!("fluxwire::", stringify!($ty)),
            binding: $binding,
            context: $ctx,
            build: || Box::new($ty),
            bind: |keys, ctx| KeyBinding::new(keys, $ty, ctx),
        }
    };
}

pub static ACTIONS: &[ActionInfo] = &[
    a!("request: send", Send, Some("secondary-enter"), None),
    a!(
        "request: cancel",
        CancelRequest,
        Some("escape"),
        Some("RequestEditor")
    ),
    a!(
        "request: copy as cURL",
        CopyAsCurl,
        Some("secondary-shift-c"),
        Some("RequestEditor")
    ),
    a!(
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
        "command palette: open",
        CommandPalette,
        Some("secondary-shift-p"),
        None
    ),
    a!(
        "workspace: new request",
        NewRequest,
        Some("secondary-n"),
        None
    ),
    a!("workspace: new folder", NewFolder, None, None),
    a!(
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
    a!("file: reload from disk", ReloadFromDisk, None, None),
    a!("file: keep my changes", KeepMine, None, None),
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
    a!("file: save", Save, Some("secondary-s"), None),
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
        "view: toggle left dock",
        ToggleLeftDock,
        Some("secondary-b"),
        None
    ),
    a!(
        "view: toggle bottom dock",
        ToggleBottomDock,
        Some("secondary-j"),
        None
    ),
    a!(
        "request: focus URL bar",
        FocusUrlBar,
        Some("secondary-l"),
        None
    ),
    a!("tab: close", CloseTab, Some("secondary-w"), None),
    a!("tab: pin or unpin", PinTab, None, None),
    a!("tab: next", NextTab, Some("ctrl-tab"), None),
    a!("tab: previous", PrevTab, Some("ctrl-shift-tab"), None),
    a!(
        "tab: reopen closed",
        ReopenClosedTab,
        Some("secondary-shift-t"),
        None
    ),
    a!("tab: group by kind", GroupTabsByKind, None, None),
    a!(
        "view: toggle split orientation",
        ToggleSplitOrientation,
        Some("secondary-alt-l"),
        None
    ),
    a!("theme: toggle", ToggleTheme, None, None),
    a!("panel: workspace", ShowWorkspacePanel, None, None),
    a!("panel: history", ShowHistoryPanel, None, None),
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
        "workspace: close",
        CloseWorkspace,
        Some("secondary-shift-w"),
        None
    ),
    a!("docs: open for request", OpenDocs, None, None),
    a!(
        "environment: edit active",
        EditActiveEnvironment,
        None,
        None
    ),
    a!("environment: manage", ManageEnvironments, None, None),
    a!("sync: open", OpenSync, None, None),
    a!("websocket: connect or disconnect", Connect, None, None),
    a!("modal: dismiss", Dismiss, Some("escape"), None),
    a!("app: quit", Quit, Some("secondary-q"), None),
    a!("tab: close others", CloseOtherTabs, None, None),
    a!("tab: close to the right", CloseTabsToRight, None, None),
    a!("tab: close clean tabs", CloseCleanTabs, None, None),
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
    a!("response: show body", ShowResponseBody, None, None),
    a!("response: show headers", ShowResponseHeaders, None, None),
    a!("response: show cookies", ShowResponseCookies, None, None),
    a!("response: show timeline", ShowResponseTimeline, None, None),
    a!("response: show tests", ShowResponseTests, None, None),
    a!("response: show console", ShowResponseConsole, None, None),
    a!("request: choose method", ToggleMethodMenu, None, None),
    a!("environment: new…", NewEnvironment, None, None),
    a!("environment: rename…", RenameEnvironment, None, None),
    a!("environment: duplicate", DuplicateEnvironment, None, None),
    a!("environment: delete…", DeleteEnvironment, None, None),
    a!("workspace: new window", NewWindow, None, None),
    a!("keymap: open keybindings", OpenKeybindings, None, None),
    a!("keymap: reload", ReloadKeymap, None, None),
    a!("keymap: open keymap.toml", OpenKeymapFile, None, None),
    a!("settings: open settings.toml", OpenSettingsFile, None, None),
    a!(
        "scripts: trust this collection",
        TrustCollection,
        None,
        None
    ),
    a!(
        "scripts: revoke trust for this collection",
        RevokeCollectionTrust,
        None,
        None
    ),
    a!("scripts: regenerate fw.d.ts", RegenerateFwTypes, None, None),
    a!(
        "quick search: open result beside",
        OpenResultBeside,
        Some("secondary-enter"),
        Some("Modal")
    ),
    a!(
        "input: move left",
        InputLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move right",
        InputRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move word left",
        InputWordLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move word right",
        InputWordRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move to line start",
        InputLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move to line end",
        InputLineEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move up",
        InputUp,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move down",
        InputDown,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move to start",
        InputDocStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: move to end",
        InputDocEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select left",
        InputSelectLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select right",
        InputSelectRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select word left",
        InputSelectWordLeft,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select word right",
        InputSelectWordRight,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select to line start",
        InputSelectLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select to line end",
        InputSelectLineEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select up",
        InputSelectUp,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select down",
        InputSelectDown,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select to start",
        InputSelectDocStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select to end",
        InputSelectDocEnd,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: select all",
        InputSelectAll,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: copy",
        InputCopy,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: cut",
        InputCut,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: paste",
        InputPaste,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: undo",
        InputUndo,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: redo",
        InputRedo,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: delete backward",
        InputBackspace,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: delete forward",
        InputDelete,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: delete word backward",
        InputDeleteWordBack,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: delete word forward",
        InputDeleteWordForward,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: delete to line start",
        InputDeleteToLineStart,
        None,
        Some(crate::input_keys::CONTEXT)
    ),
    a!(
        "input: context menu",
        InputContextMenu,
        None,
        Some(crate::input_keys::CONTEXT)
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
    ("workspace: new collection", "NewCollection"),
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
    "AddCollection",
    "Import",
    "QuickSearch",
    "OpenKeybindings",
    "RevealInFileManager",
    "ToggleLeftDock",
    "ToggleBottomDock",
    "ToggleSplitOrientation",
    "ShowWorkspacePanel",
    "ShowHistoryPanel",
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

fn norm_keys(keys: &str) -> String {
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
        if !unbound {
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
fn default_bindings() -> impl Iterator<Item = (&'static ActionInfo, &'static str)> {
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
            ("command palette: open", "secondary-shift-p"),
            ("workspace: new request", "secondary-n"),
            ("file: save", "secondary-s"),
            ("run: run folder", "secondary-shift-r"),
            ("environment: switch", "secondary-e"),
            ("view: toggle left dock", "secondary-b"),
            ("view: toggle bottom dock", "secondary-j"),
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
            ("workspace: new collection", "collection: new…"),
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
