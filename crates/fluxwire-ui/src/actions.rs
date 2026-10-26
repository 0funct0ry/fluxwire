//! Every visible action, registered with GPUI and listed in `ACTIONS` so the command palette,
//! keymap and tests read from one table (SPEC §17.5, §17.6). Most are no-ops in M1.

use gpui::{Action, App, KeyBinding, actions};

actions!(
    fluxwire,
    [
        Send,
        CancelRequest,
        CopyAsCurl,
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
        OpenFolder,
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
        ShowProjectPanel,
        ShowHistoryPanel,
        ShowEnvironmentsPanel,
        ShowSyncPanel,
        ShowConsole,
        ShowProblems,
        OpenAsText,
        CloseWorkspace,
        OpenDocs,
        OpenEnvironment,
        OpenSync,
        Connect,
        Dismiss,
        Quit,
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
    a!("workspace: new collection", NewCollection, None, None),
    a!(
        "workspace: open folder",
        OpenFolder,
        Some("secondary-o"),
        None
    ),
    a!("workspace: import", Import, None, None),
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
    a!("panel: project", ShowProjectPanel, None, None),
    a!("panel: history", ShowHistoryPanel, None, None),
    a!("panel: environments", ShowEnvironmentsPanel, None, None),
    a!("panel: sync", ShowSyncPanel, None, None),
    a!("panel: console", ShowConsole, None, None),
    a!("panel: problems", ShowProblems, None, None),
    a!("request: open as text", OpenAsText, None, None),
    a!(
        "workspace: close (show welcome)",
        CloseWorkspace,
        None,
        None
    ),
    a!("docs: open for request", OpenDocs, None, None),
    a!("environment: open editor", OpenEnvironment, None, None),
    a!("sync: open", OpenSync, None, None),
    a!("websocket: connect or disconnect", Connect, None, None),
    a!("modal: dismiss", Dismiss, Some("escape"), None),
    a!("app: quit", Quit, Some("secondary-q"), None),
];

pub fn key_bindings() -> Vec<KeyBinding> {
    ACTIONS
        .iter()
        .filter_map(|a| {
            let keys = a.binding?;
            Some((a.bind)(keys, a.context))
        })
        .collect()
}

/// Registers bindings with GPUI.
pub fn install(cx: &mut App) {
    cx.bind_keys(key_bindings());
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
        ] {
            let a = ACTIONS.iter().find(|a| a.label == label).unwrap();
            assert_eq!(a.binding, Some(keys), "{label}");
        }
    }
}
