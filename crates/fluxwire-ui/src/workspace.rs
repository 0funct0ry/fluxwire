//! The root view: window state, action handlers, title bar, status bar and modals.

use std::{cell::Cell, path::PathBuf, rc::Rc};

use gpui::{
    Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, WindowAppearance, div, prelude::*, px,
};

use crate::{
    actions::*,
    assets::{BUFFER_FONT, UI_FONT},
    mock::{self, Mock},
    tabs::TabKind,
    text_input::{TextInput, TextInputEvent},
    theme::{Appearance, Theme, theme},
    ui::{icon, kbd_label, row},
    workspace_model::Recent,
};

/// Where recent workspaces and sessions are stored. Unset in tests, so nothing touches the disk.
#[derive(Clone)]
pub struct StateDir(pub PathBuf);
impl gpui::Global for StateDir {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Workspace,
    History,
    Environments,
    Sync,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BottomTab {
    Console,
    Problems,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModalKind {
    Palette,
    QuickSearch,
    Environment,
    /// Workspace switcher: recent workspaces and "New workspace…".
    Workspaces,
    /// Single-line text prompt (new request, rename, ...).
    Prompt,
    /// `theme: select…`: every theme, previewed live.
    ThemePicker,
}

#[derive(Clone, Debug)]
pub enum PromptPurpose {
    Rename {
        key: String,
    },
    RenameCollection {
        key: String,
    },
    RenameEnvironment {
        root: PathBuf,
        name: String,
    },
    /// `editor: go to line…` in the editor of `target` of request `key`.
    GoToLine {
        key: String,
        target: crate::workspace_model::Target,
    },
}

/// A request or folder being named in place, as a row in the tree.
#[derive(Clone, Debug)]
pub struct InlineNew {
    /// Workspace-relative key of the folder (or collection directory) it is created in.
    pub parent: String,
    pub dir: PathBuf,
    pub websocket: bool,
    pub folder: bool,
    pub text: Entity<TextInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Request,
    Folder,
    Collection,
    Doc,
    /// The Workspace node itself (header row of the Workspace panel).
    Workspace,
    /// An environment row of the Environments panel; the menu key is its name.
    Environment,
}

#[derive(Clone, Debug)]
pub struct TreeMenu {
    pub key: String,
    pub kind: MenuKind,
    pub pos: Point<Pixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Params,
    Headers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Col {
    Key,
    Value,
}

/// The table cell being edited. Every keystroke is applied to the document immediately.
#[derive(Clone, Debug)]
pub struct CellEdit {
    pub key: String,
    pub table: Table,
    pub row: usize,
    pub col: Col,
    /// Mirror of the input's text.
    pub text: String,
    pub input: Entity<TextInput>,
}

#[derive(Clone, Debug)]
pub enum ItemRun {
    Action(usize),
    Open {
        key: String,
        title: String,
        kind: TabKind,
    },
    /// Activate (or with `None` clear) the active environment of a collection.
    EnvActivate {
        root: PathBuf,
        name: Option<String>,
    },
    /// "New environment…" row.
    EnvNew(PathBuf),
    /// "Create environment “q”" row: create with defaults.
    EnvCreate {
        root: PathBuf,
        name: String,
    },
    /// "Manage environments…" row.
    EnvManage(PathBuf),
    Folder(String),
    History(i64),
    Workspace(String),
    /// "New workspace…" entry of the switcher.
    NewWorkspace,
    /// A colour theme row (picker or `~` palette).
    Theme(String),
    /// "Follow system appearance" row of the theme picker.
    ThemeSystem,
}

#[derive(Clone, Debug)]
pub struct ModalItem {
    /// Search-index id (`req:…`, `act:…`); empty for non-indexed items.
    pub id: String,
    pub label: String,
    pub detail: String,
    /// Char indices of `label` matched by the query.
    pub indices: Vec<u32>,
    pub run: ItemRun,
}

pub struct Modal {
    pub kind: ModalKind,
    pub query: Entity<TextInput>,
    _sub: gpui::Subscription,
    pub selected: usize,
    pub results: Vec<ModalItem>,
}

/// Ctrl+Tab in progress: tabs by recency and the one being previewed.
#[derive(Clone, Debug)]
pub struct Switcher {
    pub order: Vec<u64>,
    pub idx: usize,
}

/// Most results shown for one query.
const MODAL_LIMIT: usize = 200;
/// Palette/quick-search items remembered for the recency boost.
const RECENT_IDS: usize = 40;

/// Per-tab view state (sub-tab selections, URL text).
#[derive(Clone, Debug)]
pub struct TabUi {
    pub sub: usize,
    pub resp: usize,
    pub method: usize,
    pub post_script: bool,
    /// Tests tab: false shows the Expect editor, true the `test` script.
    pub tests_script: bool,
    /// Docs tab: false shows the rendered Preview, true the Markdown Source editor.
    pub docs_source: bool,
    /// Selected saved message of a WebSocket tab (the one Cmd/Ctrl+Enter sends).
    pub msg_sel: usize,
    /// Saved scroll offset of the editor area while this tab is not showing.
    pub scroll: f32,
}

impl TabUi {
    pub fn new() -> Self {
        Self {
            sub: 0,
            resp: 0,
            method: 0,
            post_script: false,
            tests_script: false,
            docs_source: false,
            msg_sel: 0,
            scroll: 0.0,
        }
    }
}

impl Default for TabUi {
    fn default() -> Self {
        Self::new()
    }
}

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub struct WorkspaceView {
    pub scope: crate::open_workspace::OpenWorkspace,
    pub mock: Mock,
    pub history: crate::history::HistoryState,
    pub history_focus: FocusHandle,
    pub modal: Option<Modal>,
    /// The focused element, refreshed on every render, so fields know whether to draw a caret.
    pub focused: Option<FocusHandle>,
    /// Workspace/Collection lifecycle dialogs (New workspace, New collection, confirms).
    pub dialog: Option<crate::dialogs::Dialog>,
    pub dialog_focus: FocusHandle,
    pub modal_scroll: ScrollHandle,
    pub switcher: Option<Switcher>,
    pub keymap_watcher: Option<fluxwire_format::WatchHandle>,
    pub keymap_issues: Vec<crate::actions::KeymapIssue>,
    pub resizing_bottom: bool,
    /// Pointer position and editor share when a drag of the request/response divider began.
    pub resizing_split: Option<(f32, f32)>,
    pub keybind_query: String,
    /// The Keybindings tab is in edit mode (`keymap: edit`).
    pub keybind_editing: bool,
    pub keybind_edit: Option<crate::keybinding_editor::Pending>,
    pub keybind_input: crate::text_input::LazyInput,
    pub keybind_focus: FocusHandle,
    /// Most recently run palette/quick-search item ids, newest first.
    pub recent_ids: Vec<String>,
    pub prompt: Option<PromptPurpose>,
    pub inline_focus: FocusHandle,
    /// Transient message for the status bar (errors, notices).
    pub notice: Option<String>,
    /// The theme to put back when a live theme preview ends without being kept.
    pub theme_restore: Option<Theme>,
    pub open_error: Option<String>,
    pub recent: Recent,
    pub state_dir: Option<PathBuf>,
    pub watch_enabled: bool,
    pub session_dirty: bool,
    pub editor_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Focus handles and active pane of the Environment bulk-edit editors.
    pub editor_scroll: ScrollHandle,
    pub resizing: bool,
    pub root_focus: FocusHandle,
    pub modal_focus: FocusHandle,
    pub env_focus: FocusHandle,
    pub editor_focus: FocusHandle,
    pub filter_focus: FocusHandle,
    pub find_focus: FocusHandle,
    pub log_focus: FocusHandle,
    /// Counts observable state changes for `ui.state`.
    pub generation: u64,
    pub(crate) last_state: Option<crate::state_snapshot::UiState>,
}

impl WorkspaceView {
    /// A window with no workspace open (the welcome view). Restores the last workspace when a
    /// state directory is configured.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root_focus = cx.focus_handle();
        window.focus(&root_focus);
        window
            .observe_window_appearance(|window, cx| {
                // A configured theme ignores the OS appearance.
                if cx
                    .try_global::<crate::theme::ThemeChoice>()
                    .is_some_and(|c| c.configured.is_some())
                {
                    return;
                }
                set_theme(
                    Theme::for_appearance(appearance_of(window.appearance())),
                    cx,
                );
            })
            .detach();
        let state_dir = cx.try_global::<StateDir>().map(|s| s.0.clone());
        let state_dir_is_set = state_dir.is_some();
        let recent = state_dir
            .as_deref()
            .map(crate::workspace_model::load_recent)
            .unwrap_or_default();
        let settings = cx
            .try_global::<crate::envs::ConfigDir>()
            .map(|c| fluxwire_format::load_settings(&c.0))
            .unwrap_or_default();
        let mut this = Self {
            scope: crate::open_workspace::OpenWorkspace::default(),
            mock: mock::payments_api(),
            history: crate::history::HistoryState::new(state_dir_is_set),
            history_focus: cx.focus_handle(),
            modal: None,
            focused: None,
            dialog: None,
            dialog_focus: cx.focus_handle(),
            modal_scroll: ScrollHandle::new(),
            switcher: None,
            keymap_watcher: None,
            keymap_issues: Vec::new(),
            resizing_bottom: false,
            resizing_split: None,
            keybind_query: String::new(),
            keybind_editing: false,
            keybind_edit: None,
            keybind_input: Default::default(),
            keybind_focus: cx.focus_handle(),
            recent_ids: Vec::new(),
            prompt: None,
            inline_focus: cx.focus_handle(),
            notice: cx
                .try_global::<crate::theme::StartupNotice>()
                .map(|n| n.0.clone()),
            theme_restore: None,
            open_error: None,
            recent,
            watch_enabled: state_dir.is_some(),
            state_dir,
            session_dirty: false,
            editor_bounds: Rc::new(Cell::new(Bounds::default())),
            editor_scroll: ScrollHandle::new(),
            resizing: false,
            root_focus,
            modal_focus: cx.focus_handle(),
            env_focus: cx.focus_handle(),
            editor_focus: cx.focus_handle(),
            filter_focus: cx.focus_handle(),
            find_focus: cx.focus_handle(),
            log_focus: cx.focus_handle(),
            generation: 0,
            last_state: None,
        };
        this.scope.envs.settings = settings;
        this.recent_ids = this
            .state_dir
            .as_deref()
            .map(crate::workspace_model::load_recent_ids)
            .unwrap_or_default();
        this.history.input = Some(crate::text_input::bound_input(
            "Filter by name or URL",
            cx,
            |this: &mut Self, text, cx| {
                this.history.query.text = text;
                this.history.query.nav = None;
                this.history_reload(cx);
            },
        ));
        this.start_session_timer(cx);
        this.reload_keymap(cx);
        if this.watch_enabled {
            this.start_config_watching(cx);
        }
        cx.on_app_quit(|this, _cx| {
            this.save_session_now();
            async {}
        })
        .detach();
        if let Some(last) = this.recent.last.clone() {
            let p = PathBuf::from(last);
            if p.is_dir() {
                this.open_workspace(&p, window, cx);
            }
        }
        this
    }

    pub fn active_ui(&mut self) -> Option<&mut TabUi> {
        let id = self.scope.tabs.active_id()?;
        self.scope.tab_ui.get_mut(&id)
    }

    /// Key of the file shown by the active request tab.
    pub fn active_key(&self) -> Option<String> {
        let t = self.scope.tabs.active()?;
        matches!(t.kind, TabKind::Http { .. } | TabKind::WebSocket).then(|| t.key.clone())
    }

    /// Open non-request tabs (history, environment, ...).
    pub fn open_tab(&mut self, key: &str, title: &str, kind: TabKind, permanent: bool) {
        let id = if permanent {
            self.scope.tabs.open_permanent(key, title, kind)
        } else {
            self.scope.tabs.open_preview(key, title, kind)
        };
        self.scope.tab_ui.entry(id).or_default();
    }

    // ---- modals ---------------------------------------------------------------------------

    pub fn open_modal(&mut self, kind: ModalKind, window: &mut Window, cx: &mut Context<Self>) {
        // A modal replacing another one ends its theme preview.
        self.revert_theme_preview(cx);
        let placeholder = match kind {
            ModalKind::Palette => "Type a command…",
            ModalKind::QuickSearch => "Search requests, folders, environments…",
            ModalKind::Environment => "Switch environment…",
            ModalKind::Workspaces => "Switch workspace…",
            ModalKind::Prompt => "Name",
            ModalKind::ThemePicker => "Select a theme…",
        };
        let query = cx.new(|cx| {
            let mut i = TextInput::new(placeholder, cx);
            // The result list owns Up/Down.
            i.set_consume_vertical(false);
            // Keep the `Modal` key context, so its bindings apply while the field has focus.
            i.set_owner_context("Modal");
            i
        });
        let sub = cx.subscribe(&query, |this, _, ev: &TextInputEvent, cx| {
            if *ev == TextInputEvent::Changed {
                if let Some(m) = &mut this.modal {
                    m.selected = 0;
                }
                this.modal_scroll.scroll_to_item(0);
                this.recompute_modal(cx);
                this.preview_highlighted_theme(cx);
                cx.notify();
            }
        });
        let focus = query.read(cx).focus_handle(cx);
        self.modal = Some(Modal {
            kind,
            query,
            _sub: sub,
            selected: 0,
            results: Vec::new(),
        });
        self.recompute_modal(cx);
        if kind == ModalKind::ThemePicker {
            self.theme_restore = Some(theme(cx).clone());
            let row = self.current_theme_row(cx);
            if let Some(m) = &mut self.modal {
                m.selected = row;
            }
            self.modal_scroll.scroll_to_item(row);
        }
        window.focus(&focus);
        cx.notify();
    }

    /// Pre-fill the open modal's field, caret at the end (rename prompts).
    pub fn prefill_modal(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(m) = &self.modal {
            m.query.update(cx, |i, cx| {
                i.set_text(text, cx);
            });
        }
    }

    pub fn modal_text(&self, cx: &gpui::App) -> String {
        self.modal
            .as_ref()
            .map(|m| m.query.read(cx).text().to_string())
            .unwrap_or_default()
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.revert_theme_preview(cx);
        self.prompt = None;
        if self.modal.take().is_some() {
            window.focus(&self.root_focus);
            cx.notify();
        }
    }

    /// Bring the search index up to date with the workspace. Only the delta is applied.
    pub fn refresh_search_index(&mut self, cx: &gpui::App) {
        use fluxwire_search::{Item, Kind};
        let mut want: Vec<Item> = Vec::new();
        if let Some(p) = &self.scope.workspace {
            want.extend(p.search_items());
        }
        if let Some(root) = self.env_root() {
            for e in fluxwire_format::list_environments(&root) {
                want.push(
                    Item::new(format!("env:{e}"), Kind::Environment, e).detail("environment"),
                );
            }
        }
        want.extend(self.history.entries.iter().map(|e| {
            let status = e.status.map_or("—".to_string(), |s| s.to_string());
            Item::new(
                format!("hist:{}", e.id),
                Kind::History,
                format!("{} {}", e.method, e.request_name),
            )
            .detail(format!("{status} · {}", crate::history::day_label(e.ts_ms)))
            .extra(e.url.clone())
        }));
        want.extend(self.recent.workspaces.iter().map(|w| {
            let name = std::path::Path::new(w)
                .file_name()
                .map_or(w.clone(), |n| n.to_string_lossy().into_owned());
            Item::new(format!("ws:{w}"), Kind::Workspace, name).detail(w.clone())
        }));
        want.extend(crate::theme::registry().all().iter().map(|t| {
            // No detail on the item: the fuzzy matcher would match "dark" and "light" too.
            Item::new(
                format!("theme:{}", t.name),
                Kind::Theme,
                format!("Theme: {}", t.name),
            )
        }));
        let bindings = cx.try_global::<KeymapState>();
        want.extend(
            ACTIONS
                .iter()
                .filter(|a| a.palette && self.action_available(a.name))
                .map(|a| {
                    let keys = match bindings {
                        Some(b) => b.label_for(a.name).unwrap_or_default(),
                        None => a.binding.map(kbd_label).unwrap_or_default(),
                    };
                    Item::new(format!("act:{}", a.name), Kind::Action, a.label).detail(keys)
                }),
        );
        self.scope.search.sync(want);
    }

    pub fn is_focused(&self, h: &FocusHandle) -> bool {
        self.focused.as_ref() == Some(h)
    }

    /// Whether the level an action belongs to is open (SPEC §17.9).
    pub fn action_available(&self, name: &str) -> bool {
        if name.rsplit("::").next() == Some("DeleteEnvironment") && self.env_target().is_none() {
            return false;
        }
        match scope_of(name) {
            Scope::App => true,
            Scope::Workspace => self.scope.workspace.is_some(),
            Scope::Collection => self
                .scope
                .workspace
                .as_ref()
                .is_some_and(|w| !w.collections.is_empty()),
        }
    }

    fn search_results(&mut self, query: &str, cx: &gpui::App) -> Vec<ModalItem> {
        self.refresh_search_index(cx);
        self.scope.search.set_recency(self.recent_ids.clone());
        let hits = self.scope.search.query(query, MODAL_LIMIT);
        hits.into_iter()
            .filter_map(|h| {
                let item = self.scope.search.item(h.pos)?;
                let mut mi = modal_item(item, h.label_indices);
                if let Some(name) = mi.id.strip_prefix("env:")
                    && let Some(root) = self.env_root()
                {
                    mi.run = ItemRun::EnvActivate {
                        root,
                        name: Some(name.to_string()),
                    };
                }
                Some(mi)
            })
            .collect()
    }

    /// Rebuild the open modal's results from its query.
    pub fn recompute_modal(&mut self, cx: &gpui::App) {
        let Some(m) = &self.modal else {
            return;
        };
        let (kind, q) = (m.kind, m.query.read(cx).text().to_string());
        let results = match kind {
            ModalKind::Palette if q.starts_with('~') => self.search_results(&q, cx),
            ModalKind::Palette => self.search_results(&format!(">{q}"), cx),
            ModalKind::ThemePicker => crate::theme_picker::picker_items(&q),
            ModalKind::QuickSearch => self.search_results(&q, cx),
            ModalKind::Prompt => vec![],
            ModalKind::Workspaces => {
                let current = self.scope.workspace.as_ref().map(|w| w.root.clone());
                let mut v = vec![ModalItem {
                    id: String::new(),
                    label: "New workspace…".into(),
                    detail: crate::ui::kbd_label("secondary-shift-n"),
                    indices: Vec::new(),
                    run: ItemRun::NewWorkspace,
                }];
                v.extend(self.recent.workspaces.iter().map(|p| {
                    let path = std::path::Path::new(p);
                    let name = path
                        .file_name()
                        .map_or(p.clone(), |n| n.to_string_lossy().into_owned());
                    let detail = if current.as_deref() == Some(path) {
                        format!("current · {p}")
                    } else if !path.is_dir() {
                        format!("Not found · {p}")
                    } else {
                        p.clone()
                    };
                    ModalItem {
                        id: String::new(),
                        label: name,
                        detail,
                        indices: Vec::new(),
                        run: ItemRun::Workspace(p.clone()),
                    }
                }));
                v.retain(|i| fuzzy(&q, &i.label) || fuzzy(&q, &i.detail));
                v
            }
            ModalKind::Environment => self.env_switcher_items(&q),
        };
        if let Some(m) = &mut self.modal {
            m.selected = m.selected.min(results.len().saturating_sub(1));
            if kind == ModalKind::Environment && !q.is_empty() {
                // With a query, Enter should pick a match, not "New environment…".
                m.selected = results
                    .iter()
                    .position(|i| {
                        matches!(
                            i.run,
                            ItemRun::EnvCreate { .. } | ItemRun::EnvActivate { .. }
                        )
                    })
                    .or_else(|| {
                        results
                            .iter()
                            .position(|i| !matches!(i.run, ItemRun::EnvNew(_)))
                    })
                    .unwrap_or(0);
            }
            m.results = results;
        }
    }

    /// Results for the open modal.
    pub fn modal_items(&self) -> Vec<ModalItem> {
        self.modal
            .as_ref()
            .map(|m| m.results.clone())
            .unwrap_or_default()
    }

    fn remember_recent(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        self.recent_ids.retain(|i| i != id);
        self.recent_ids.insert(0, id.to_string());
        self.recent_ids.truncate(RECENT_IDS);
        if let Some(d) = &self.state_dir {
            crate::workspace_model::save_recent_ids(d, &self.recent_ids);
        }
    }

    /// Run a result. `beside` (Cmd/Ctrl+Enter) opens the tab next to the active one, or the
    /// workspace in a new window.
    pub fn run_modal_item(
        &mut self,
        item: ModalItem,
        beside: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &item.run {
            ItemRun::Theme(name) => self.commit_theme(Some(name.clone()), cx),
            ItemRun::ThemeSystem => self.commit_theme(None, cx),
            _ => {}
        }
        self.close_modal(window, cx);
        self.remember_recent(&item.id);
        let prev = self.scope.tabs.active_id();
        match item.run {
            ItemRun::Action(i) => window.dispatch_action((ACTIONS[i].build)(), cx),
            ItemRun::Open { key, title, kind } => {
                if matches!(kind, TabKind::Http { .. } | TabKind::WebSocket) {
                    self.open_request(&key, true, window, cx);
                } else if matches!(kind, TabKind::Environment) {
                    self.open_env_tab(&title);
                } else {
                    self.open_tab(&key, &title, kind, true);
                }
            }
            ItemRun::EnvActivate { root, name } => match (beside, name) {
                // Cmd/Ctrl+Enter opens the editor without activating.
                (true, Some(n)) => self.open_env_doc(&root, &n, true),
                (_, name) => self.set_active_env(name, cx),
            },
            ItemRun::EnvNew(root) => self.start_new_env_in(&root, "", window, cx),
            ItemRun::EnvCreate { root, name } => {
                if let Some(e) = self.env_name_error(&root, &name, None) {
                    self.notice = Some(e);
                } else if let Err(e) = fluxwire_format::create_environment(&root, &name) {
                    self.notice = Some(format!("Could not create {name}: {e}"));
                } else {
                    self.scope.envs.invalidate();
                    self.set_active_env(Some(name.clone()), cx);
                    self.open_env_doc(&root, &name, true);
                    self.env_focus_first_cell(&(root, name), window, cx);
                }
            }
            ItemRun::EnvManage(root) => {
                self.scope.left_open = true;
                self.scope.panel = Panel::Environments;
                if let Some(n) = self.active_env_name() {
                    self.open_env_doc(&root, &n, true);
                }
            }
            ItemRun::Folder(key) => {
                self.scope.left_open = true;
                self.scope.panel = Panel::Workspace;
                for part in folder_chain(&key) {
                    self.scope.collapsed.remove(&part);
                }
                self.scope.selected = Some(key);
            }
            ItemRun::History(id) => self.open_history_entry(id, true, cx),
            ItemRun::NewWorkspace => self.start_new_workspace(window, cx),
            ItemRun::Theme(_) | ItemRun::ThemeSystem => {}
            ItemRun::Workspace(path) => {
                let path = PathBuf::from(path);
                if !path.is_dir() {
                    self.notice = Some(format!("{} was not found.", path.display()));
                } else if self
                    .scope
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.root == path)
                {
                    // Already the open workspace.
                } else if beside {
                    open_window_at(cx, Some(path));
                } else {
                    self.request_close(crate::dialogs::AfterClose::Open(path), window, cx);
                }
            }
        }
        if beside
            && let (Some(prev), Some(now)) = (prev, self.scope.tabs.active_id())
            && prev != now
        {
            self.scope.tabs.place_after(now, prev);
        }
        self.session_dirty = true;
        cx.notify();
    }

    /// Run the highlighted result (Enter, or Cmd/Ctrl+Enter for `beside`).
    pub fn run_selected(&mut self, beside: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() {
            return;
        }
        let sel = self.modal.as_ref().map_or(0, |m| m.selected);
        let item = self
            .modal
            .as_ref()
            .and_then(|m| m.results.get(sel).cloned());
        if let Some(item) = item {
            self.run_modal_item(item, beside, window, cx);
        }
    }

    fn on_modal_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.modal.as_ref().map_or(0, |m| m.results.len());
        let key = ev.keystroke.key.as_str();
        let moves = matches!(key, "down" | "up" | "pageup" | "pagedown" | "home" | "end")
            && self.modal.is_some();
        match key {
            "down" | "up" => {
                if let Some(m) = &mut self.modal {
                    m.selected = if key == "down" {
                        (m.selected + 1).min(count.saturating_sub(1))
                    } else {
                        m.selected.saturating_sub(1)
                    };
                    self.modal_scroll.scroll_to_item(m.selected);
                }
            }
            "pageup" | "pagedown" | "home" | "end"
                if self
                    .modal
                    .as_ref()
                    .is_some_and(|m| m.kind == ModalKind::ThemePicker) =>
            {
                if let Some(m) = &mut self.modal {
                    let last = count.saturating_sub(1);
                    m.selected = match key {
                        "pagedown" => (m.selected + 8).min(last),
                        "pageup" => m.selected.saturating_sub(8),
                        "home" => 0,
                        _ => last,
                    };
                    self.modal_scroll.scroll_to_item(m.selected);
                }
            }
            "enter" if self.prompt.is_some() => self.confirm_prompt(window, cx),
            "enter" => {
                let sel = self.modal.as_ref().map_or(0, |m| m.selected);
                let item = self
                    .modal
                    .as_ref()
                    .and_then(|m| m.results.get(sel).cloned());
                if let Some(item) = item {
                    let beside = ev.keystroke.modifiers.secondary();
                    self.run_modal_item(item, beside, window, cx);
                }
            }
            _ => {}
        }
        if moves {
            self.preview_highlighted_theme(cx);
        }
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------------------------

    fn render_title_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let env = self.env_label();
        let has_collection = self.scope.workspace.is_some();
        let has_active = self.active_env_name().is_some();
        let t = t.clone();
        row()
            .id("title-bar")
            .window_control_area(gpui::WindowControlArea::Drag)
            .on_click(|ev, window, _| {
                // Native behaviour: zoom or minimize per the system setting on macOS.
                if ev.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .h(px(32.))
            .flex_none()
            .gap_3()
            .px_2()
            .when(cfg!(target_os = "macos"), |d| d.pl(px(78.)))
            .bg(t.title_bar)
            .border_b_1()
            .border_color(t.border)
            .text_size(px(14.))
            .child(
                row()
                    .gap_1()
                    .px_2()
                    .rounded(px(4.))
                    .text_color(t.text)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .id("workspace-switcher")
                    .cursor_pointer()
                    .hover(|s| s.bg(t.element_hover))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(
                        cx.listener(|this, _, w, cx| this.open_modal(ModalKind::Workspaces, w, cx)),
                    )
                    .child(self.workspace_label())
                    .child(icon("chevron-down", 12., t.text_muted)),
            )
            .children(self.branch_label().map(|b| {
                row()
                    .gap_1()
                    .text_color(t.text_muted)
                    .child(icon("git-branch", 12., t.text_muted))
                    .child(b)
            }))
            .when(has_collection, |d| {
                d.child(
                    row()
                        .id("env-pill")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .gap_2()
                        .px_2()
                        .h(px(22.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .text_color(t.text)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.element_hover))
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.open_modal(ModalKind::Environment, w, cx)
                        }))
                        .tooltip({
                            let tip = format!(
                                "Switch environment ({})",
                                crate::ui::kbd_label("secondary-e")
                            );
                            move |_, cx| cx.new(|_| crate::ui::VarTip { text: tip.clone() }).into()
                        })
                        .child(div().size(px(7.)).rounded_full().bg(if has_active {
                            t.modified
                        } else {
                            t.border
                        }))
                        .child(env)
                        .child(icon("chevron-down", 12., t.text_muted)),
                )
            })
            .child(div().flex_1())
            .child(
                row()
                    .id("search-pill")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .gap_2()
                    .px_2()
                    .w(px(220.))
                    .h(px(22.))
                    .justify_between()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.editor)
                    .text_color(t.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.element_hover))
                    .on_click(
                        cx.listener(|this, _, w, cx| {
                            this.open_modal(ModalKind::QuickSearch, w, cx)
                        }),
                    )
                    .child(
                        row()
                            .gap_1()
                            .child(icon("search", 12., t.text_muted))
                            .child("Search"),
                    )
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(kbd_label("secondary-k")),
                    ),
            )
    }

    fn render_status_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let t = t.clone();
        let panel_btn = |id: &'static str, glyph: &'static str, p: Panel, active: bool| {
            let t = t.clone();
            div()
                .id(id)
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .when(active, |d| d.bg(t.element_selected))
                .hover(|s| s.bg(t.element_hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.scope.panel == p && this.scope.left_open {
                        this.scope.left_open = false;
                    } else {
                        this.scope.panel = p;
                        this.scope.left_open = true;
                        if p == Panel::History {
                            this.history_open(cx);
                        }
                    }
                    cx.notify();
                }))
                .child(icon(glyph, 14., if active { t.text } else { t.text_muted }))
        };
        let problems = self.problem_total();
        let notice = self.notice.clone();
        let sync_down = self.mock.sync.iter().filter(|c| c.arrow == "↓").count();
        let sync_up = self.mock.sync.len() - sync_down;
        row()
            .h(px(24.))
            .flex_none()
            .px_2()
            .gap_3()
            .bg(t.surface)
            .border_t_1()
            .border_color(t.border)
            .text_size(px(12.))
            .text_color(t.text_muted)
            .child(
                row()
                    .gap_1()
                    .child(panel_btn(
                        "p-workspace",
                        "folder",
                        Panel::Workspace,
                        self.scope.left_open && self.scope.panel == Panel::Workspace,
                    ))
                    .child(panel_btn(
                        "p-history",
                        "file-text",
                        Panel::History,
                        self.scope.left_open && self.scope.panel == Panel::History,
                    ))
                    .child(panel_btn(
                        "p-envs",
                        "layers",
                        Panel::Environments,
                        self.scope.left_open && self.scope.panel == Panel::Environments,
                    ))
                    .child(panel_btn(
                        "p-sync",
                        "refresh-cw",
                        Panel::Sync,
                        self.scope.left_open && self.scope.panel == Panel::Sync,
                    ))
                    .child(
                        div()
                            .id("p-bottom")
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(self.scope.bottom_open, |d| d.bg(t.element_selected))
                            .hover(|s| s.bg(t.element_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.scope.bottom_open = !this.scope.bottom_open;
                                cx.notify();
                            }))
                            .child(icon(
                                "terminal",
                                14.,
                                if self.scope.bottom_open {
                                    t.text
                                } else {
                                    t.text_muted
                                },
                            )),
                    ),
            )
            .children(self.branch_label().map(|b| {
                row()
                    .gap_1()
                    .child(icon("git-branch", 12., t.text_muted))
                    .child(b)
            }))
            .child(
                row()
                    .id("problems-count")
                    .gap_1()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scope.bottom_open = true;
                        this.scope.bottom_tab = BottomTab::Problems;
                        cx.notify();
                    }))
                    .child(icon("triangle-alert", 12., t.modified))
                    .child(problems.to_string()),
            )
            .child(
                div()
                    .id("sync-status")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_tab("sync", "Sync", TabKind::Sync, true);
                        cx.notify();
                    }))
                    .child(format!("OpenAPI ↓{sync_down} ↑{sync_up}")),
            )
            .child(div().flex_1())
            .children(notice.map(|n| {
                div()
                    .id("notice")
                    .cursor_pointer()
                    .text_color(t.modified)
                    .max_w(px(520.))
                    .truncate()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.notice = None;
                        cx.notify();
                    }))
                    .child(n)
            }))
            .child(div().font_family(BUFFER_FONT).child("QuickJS"))
            .child(div().child("UTF-8"))
    }

    fn render_modal(&self, t: &Theme, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let m = self.modal.as_ref()?;
        let t = t.clone();
        let items = m.results.clone();
        let title = self.prompt_title().or_else(|| self.env_modal_title());
        let selected = m.selected;
        let original_theme = self.original_theme_name(cx);
        Some(
            div()
                .id("modal-backdrop")
                .absolute()
                .size_full()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(80.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, w, cx| this.close_modal(w, cx)),
                )
                .child(
                    div()
                        .id("modal")
                        .key_context("Modal")
                        .track_focus(&self.modal_focus)
                        .on_key_down(cx.listener(|this, ev, w, cx| this.on_modal_key(ev, w, cx)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .w(px(560.))
                        .flex()
                        .flex_col()
                        .bg(t.elevated)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(6.))
                        .shadow_lg()
                        .overflow_hidden()
                        .children(title.map(|title| {
                            div()
                                .px_3()
                                .pt_2()
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child(title)
                        }))
                        .child(
                            row()
                                .h(px(36.))
                                .px_3()
                                .border_b_1()
                                .border_color(t.border_variant)
                                .text_size(px(14.))
                                .child(div().flex_1().child(m.query.clone())),
                        )
                        .child(
                            div()
                                .id("modal-results")
                                .track_scroll(&self.modal_scroll)
                                .flex()
                                .flex_col()
                                .max_h(px(360.))
                                .overflow_y_scroll()
                                .py_1()
                                .children(items.into_iter().enumerate().map(|(i, item)| {
                                    let click_item = item.clone();
                                    let edit_target = match &item.run {
                                        ItemRun::EnvActivate {
                                            root,
                                            name: Some(n),
                                        } => Some((root.clone(), n.clone())),
                                        _ => None,
                                    };
                                    row()
                                        .id(("modal-item", i))
                                        .h(px(28.))
                                        .px_3()
                                        .justify_between()
                                        .text_size(px(14.))
                                        .text_color(t.text)
                                        .when(i == selected, |d| d.bg(t.element_selected))
                                        .hover(|s| s.bg(t.element_hover))
                                        .cursor_pointer()
                                        .on_click(cx.listener(
                                            move |this, ev: &gpui::ClickEvent, w, cx| {
                                                let beside = ev.modifiers().secondary();
                                                this.run_modal_item(
                                                    click_item.clone(),
                                                    beside,
                                                    w,
                                                    cx,
                                                )
                                            },
                                        ))
                                        .child(
                                            row()
                                                .gap_2()
                                                .children(match &item.run {
                                                    ItemRun::Theme(n) => Some(icon(
                                                        if *n == original_theme {
                                                            "check"
                                                        } else {
                                                            "palette"
                                                        },
                                                        14.,
                                                        t.text_muted,
                                                    )),
                                                    _ => None,
                                                })
                                                .child(highlighted(&item.label, &item.indices, &t)),
                                        )
                                        .child(
                                            row()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(BUFFER_FONT)
                                                        .text_size(px(11.))
                                                        .text_color(t.text_placeholder)
                                                        .child(item.detail.clone()),
                                                )
                                                .children(edit_target.map(|(root, name)| {
                                                    div()
                                                        .id(("modal-edit", i))
                                                        .on_click(cx.listener(
                                                            move |this, _, w, cx| {
                                                                cx.stop_propagation();
                                                                this.close_modal(w, cx);
                                                                this.open_env_doc(
                                                                    &root, &name, true,
                                                                );
                                                                cx.notify();
                                                            },
                                                        ))
                                                        .child(icon(
                                                            "pencil",
                                                            12.,
                                                            t.text_placeholder,
                                                        ))
                                                })),
                                        )
                                })),
                        ),
                ),
        )
    }
}

fn folder_chain(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for part in key.split('/').filter(|p| !p.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        out.push(acc.clone());
    }
    out
}

/// A search result row from an index item; the id prefix says what running it does.
fn modal_item(item: &fluxwire_search::Item, indices: Vec<u32>) -> ModalItem {
    let id = item.id.clone();
    let method = item.extra.split(' ').next().unwrap_or("GET").to_string();
    let (run, detail) = if let Some(key) = id.strip_prefix("req:") {
        let kind = if method == "WS" {
            TabKind::WebSocket
        } else {
            TabKind::Http {
                method: method.clone(),
            }
        };
        (
            ItemRun::Open {
                key: key.into(),
                title: item.label.clone(),
                kind,
            },
            format!("{method} {}", item.detail),
        )
    } else if let Some(key) = id.strip_prefix("folder:") {
        (ItemRun::Folder(key.into()), item.detail.clone())
    } else if let Some(key) = id.strip_prefix("doc:") {
        (
            ItemRun::Open {
                key: key.into(),
                title: item.label.clone(),
                kind: TabKind::Docs,
            },
            item.detail.clone(),
        )
    } else if let Some(rest) = id.strip_prefix("head:") {
        let key = rest.rsplit_once('#').map_or(rest, |(k, _)| k);
        (
            ItemRun::Open {
                key: key.into(),
                title: item.label.clone(),
                kind: TabKind::Http {
                    method: "GET".into(),
                },
            },
            format!("¶ {}", item.detail),
        )
    } else if id.starts_with("env:") {
        (
            ItemRun::Open {
                key: format!("env/{}", item.label),
                title: item.label.clone(),
                kind: TabKind::Environment,
            },
            item.detail.clone(),
        )
    } else if let Some(n) = id.strip_prefix("hist:") {
        (
            ItemRun::History(n.parse().unwrap_or_default()),
            item.detail.clone(),
        )
    } else if let Some(p) = id.strip_prefix("ws:") {
        (ItemRun::Workspace(p.into()), item.detail.clone())
    } else if let Some(name) = id.strip_prefix("theme:") {
        let dark = crate::theme::registry()
            .get(name)
            .is_some_and(|t| t.is_dark());
        (
            ItemRun::Theme(name.into()),
            if dark { "dark" } else { "light" }.into(),
        )
    } else {
        let name = id.strip_prefix("act:").unwrap_or(&id);
        let i = ACTIONS.iter().position(|a| a.name == name).unwrap_or(0);
        (ItemRun::Action(i), item.detail.clone())
    };
    ModalItem {
        id,
        label: item.label.clone(),
        detail,
        indices,
        run,
    }
}

/// `label` with the matched chars in the accent color.
fn highlighted(label: &str, indices: &[u32], t: &Theme) -> gpui::Div {
    let mut out = row().whitespace_nowrap().overflow_hidden();
    let mut seg = String::new();
    let mut on = false;
    let flush = |out: gpui::Div, seg: &mut String, on: bool| {
        if seg.is_empty() {
            return out;
        }
        let text = std::mem::take(seg);
        out.child(if on {
            div()
                .text_color(t.accent)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(text)
        } else {
            div().child(text)
        })
    };
    for (i, c) in label.chars().enumerate() {
        let m = indices.contains(&(i as u32));
        if m != on {
            out = flush(out, &mut seg, on);
            on = m;
        }
        seg.push(c);
    }
    flush(out, &mut seg, on)
}

/// Subsequence match, case-insensitive. An empty query matches everything.
pub fn fuzzy(query: &str, text: &str) -> bool {
    let mut hay = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|q| hay.any(|c| c == q))
}

pub fn appearance_of(a: WindowAppearance) -> Appearance {
    match a {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        _ => Appearance::Dark,
    }
}

pub fn set_theme(theme: Theme, cx: &mut gpui::App) {
    cx.set_global(theme);
    cx.refresh_windows();
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.root_focus.clone()
    }
}

macro_rules! handlers {
    ($el:expr, $cx:expr, { $($act:ident => |$this:ident, $w:ident, $c:ident| $body:expr),* $(,)? }) => {{
        let mut el = $el;
        $(
            el = el.on_action($cx.listener(|$this, _: &$act, $w, $c| {
                if !$this.action_available(stringify!($act)) {
                    return;
                }
                let () = { $body };
                $c.notify();
            }));
        )*
        el
    }};
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focused = window.focused(cx);
        self.track_generation(window, cx);
        self.sync_scroll();
        let t = theme(cx).clone();
        let body = if self.scope.workspace.is_some() {
            self.render_body(&t, cx).into_any_element()
        } else {
            self.render_welcome(&t, cx).into_any_element()
        };
        let root = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.root_focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(t.editor)
            .text_color(t.text)
            .font_family(UI_FONT)
            .text_size(px(14.))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if this.history.resizing.is_some() {
                    this.history_drag(f32::from(ev.position.x), cx);
                }
                if this.resizing {
                    this.scope.left_width = f32::from(ev.position.x).clamp(180.0, 520.0);
                    cx.notify();
                }
                if let Some((p0, f0)) = this.resizing_split {
                    // The editor/response area is roughly the window minus the chrome around it.
                    let size = window.bounds().size;
                    let (p, total) = if this.scope.split_vertical {
                        (f32::from(ev.position.y), f32::from(size.height) - 150.0)
                    } else {
                        let dock = if this.scope.left_open {
                            this.scope.left_width
                        } else {
                            0.0
                        };
                        (f32::from(ev.position.x), f32::from(size.width) - dock)
                    };
                    // Keep both panes wide enough for their tab strips and toolbars.
                    let total = total.max(200.0);
                    let min_px = if this.scope.split_vertical {
                        140.0
                    } else {
                        360.0
                    };
                    let lo = (min_px / total).min(0.5);
                    this.scope.split_frac = (f0 + (p - p0) / total).clamp(lo, 1.0 - lo);
                    cx.notify();
                }
                if this.resizing_bottom {
                    // Distance from the pointer to the window bottom, minus the 24 px status bar.
                    let below =
                        f32::from(window.bounds().size.height) - f32::from(ev.position.y) - 24.0;
                    this.scope.bottom_height = below.clamp(80.0, 600.0);
                    cx.notify();
                }
            }))
            .on_modifiers_changed(
                cx.listener(|this, ev: &gpui::ModifiersChangedEvent, _, cx| {
                    let m = ev.modifiers;
                    if this.switcher.is_some() && !m.control && !m.platform && !m.alt {
                        this.commit_switcher();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.history_drag_end(cx);
                    if this.resizing_split.take().is_some() {
                        cx.notify();
                    }
                    if this.resizing || this.resizing_bottom {
                        this.resizing = false;
                        this.resizing_bottom = false;
                        this.session_dirty = true;
                        cx.notify();
                    }
                }),
            );
        let root = handlers!(root, cx, {
            QuickSearch => |this, w, c| this.open_modal(ModalKind::QuickSearch, w, c),
            CommandPalette => |this, w, c| {
                if this.modal.as_ref().is_some_and(|m| m.kind == ModalKind::Palette) {
                    this.close_modal(w, c);
                } else {
                    this.open_modal(ModalKind::Palette, w, c);
                }
            },
            NewWorkspaceCollection => |this, w, c| this.start_new_collection(None, w, c),
            CopyWorkspacePath => |this, _w, c| this.copy_workspace_path(c),
            EditKeymap => |this, _w, c| this.edit_keymap(c),
            SwitchEnvironment => |this, w, c| this.open_modal(ModalKind::Environment, w, c),
            SwitchWorkspace => |this, w, c| this.open_modal(ModalKind::Workspaces, w, c),
            Dismiss => |this, w, c| {
                if this.dialog.is_some() { this.close_dialog(w, c); return; }
                if let Some(sw) = this.switcher.take() { if let Some(first) = sw.order.first() { this.scope.tabs.peek(*first) } return; }
                let hist_open = this.history.menu.is_some() || this.history.nav_menu.is_some() || this.history.clear_menu || this.history.export_menu;
                this.history.menu = None; this.history.nav_menu = None; this.history.clear_menu = false; this.history.export_menu = false;
                let open = hist_open || this.scope.tab_menu.is_some() || this.scope.method_menu || this.scope.tree_menu.is_some() || this.scope.cell.is_some() || this.scope.inline.is_some() || this.modal.is_some();
                this.scope.tab_menu = None; this.scope.method_menu = false; this.scope.tree_menu = None; this.scope.cell = None; this.scope.inline = None; this.close_modal(w, c);
                // Esc with nothing to dismiss cancels an in-flight send
                if !open { this.cancel_request(c); }
            },
            ToggleLeftDock => |this, _w, _c| { this.scope.left_open = !this.scope.left_open; this.session_dirty = true },
            ToggleBottomDock => |this, _w, _c| { this.scope.bottom_open = !this.scope.bottom_open; this.session_dirty = true },
            ShowWorkspacePanel => |this, _w, _c| { this.scope.panel = Panel::Workspace; this.scope.left_open = true; this.session_dirty = true },
            HistoryOpen => |this, _w, c| this.history_open(c),
            HistoryFocusSearch => |this, w, c| this.history_focus_search(w, c),
            HistoryPin => |this, _w, c| this.history_toggle_pin(c),
            HistoryExport => |this, _w, c| this.history_export(fluxwire_store::ExportFormat::Json, c),
            ShowHistoryPanel => |this, _w, _c| { this.scope.panel = Panel::History; this.scope.left_open = true; this.session_dirty = true },
            ClearHistoryAll => |this, w, c| this.confirm_clear_history(fluxwire_store::Scope::All, "this workspace".into(), w, c),
            ClearHistoryCollection => |this, _w, c| this.history_clear(crate::history::ClearScope::Collection, c),
            ClearHistoryRequest => |this, _w, c| this.history_clear(crate::history::ClearScope::Request, c),
            ShowEnvironmentsPanel => |this, _w, _c| { this.scope.panel = Panel::Environments; this.scope.left_open = true; this.session_dirty = true },
            ShowSyncPanel => |this, _w, _c| { this.scope.panel = Panel::Sync; this.scope.left_open = true; this.session_dirty = true },
            ShowConsole => |this, _w, _c| { this.scope.bottom_tab = BottomTab::Console; this.scope.bottom_open = true; this.session_dirty = true },
            ShowProblems => |this, _w, _c| { this.scope.bottom_tab = BottomTab::Problems; this.scope.bottom_open = true; this.session_dirty = true },
            ToggleTheme => |_this, _w, c| {
                let next = match theme(c).appearance {
                    Appearance::Dark => Appearance::Light,
                    Appearance::Light => Appearance::Dark,
                };
                set_theme(Theme::for_appearance(next), c);
            },
            SelectTheme => |this, w, c| this.open_modal(ModalKind::ThemePicker, w, c),
            ToggleSplitOrientation => |this, _w, _c| { this.scope.split_vertical = !this.scope.split_vertical; this.session_dirty = true },
            CloseTab => |this, w, c| if let Some(id) = this.scope.tabs.active_id() { if this.is_env_tab(id) { this.close_env_tab(id, w, c) } else { this.scope.tabs.close(id); this.session_dirty = true } },
            PinTab => |this, _w, _c| if let Some(id) = this.scope.tabs.active_id() { this.scope.tabs.toggle_pin(id); this.session_dirty = true },
            NextTab => |this, _w, _c| this.switch_tab(1),
            PrevTab => |this, _w, _c| this.switch_tab(-1),
            ReopenClosedTab => |this, w, c| this.reopen_closed_tab(w, c),
            GroupTabsByKind => |this, _w, _c| { this.scope.tabs.group_by_kind = !this.scope.tabs.group_by_kind; this.session_dirty = true },
            CloseOtherTabs => |this, _w, _c| if let Some(id) = this.scope.tabs.active_id() { this.scope.tabs.close_others(id); this.session_dirty = true },
            CloseTabsToRight => |this, _w, _c| if let Some(id) = this.scope.tabs.active_id() { this.scope.tabs.close_to_right(id); this.session_dirty = true },
            CloseCleanTabs => |this, _w, _c| { this.scope.tabs.close_clean(); this.session_dirty = true },
            CloseAllTabs => |this, _w, _c| { this.scope.tabs.close_all(); this.session_dirty = true },
            CopyTabPath => |this, _w, c| if let Some(id) = this.scope.tabs.active_id() { this.copy_tab_path(id, c) },
            ShowParams => |this, _w, _c| this.show_request_tab(0),
            ShowHeaders => |this, _w, _c| this.show_request_tab(1),
            ShowBody => |this, _w, _c| this.show_request_tab(2),
            ShowAuth => |this, _w, _c| this.show_request_tab(3),
            ShowScripts => |this, _w, _c| this.show_request_tab(4),
            ShowTests => |this, _w, _c| this.show_request_tab(5),
            ShowDocsTab => |this, _w, _c| this.show_request_tab(6),
            ShowSettings => |this, _w, _c| this.show_request_tab(7),
            ShowResponseBody => |this, _w, _c| this.show_response_tab(0),
            ShowResponseHeaders => |this, _w, _c| this.show_response_tab(1),
            ShowResponseCookies => |this, _w, _c| this.show_response_tab(2),
            ShowResponseTimeline => |this, _w, _c| this.show_response_tab(3),
            ShowResponseTests => |this, _w, _c| this.show_response_tab(4),
            ShowResponseConsole => |this, _w, _c| this.show_response_tab(5),
            ToggleMethodMenu => |this, _w, _c| this.scope.method_menu = !this.scope.method_menu,
            NewEnvironment => |this, w, c| this.start_new_env(w, c),
            RenameEnvironment => |this, w, c| if let Some((r, n)) = this.env_target() { this.start_rename_env_in(&r, &n, w, c) },
            DuplicateEnvironment => |this, _w, c| if let Some((r, n)) = this.env_target() { this.duplicate_env_in(&r, &n, c) },
            DeleteEnvironment => |this, w, c| if let Some((r, n)) = this.env_target() { this.confirm_delete_env(&r, &n, w, c) },
            NewWindow => |_this, _w, c| open_window_at(c, None),
            OpenResultBeside => |this, w, c| this.run_selected(true, w, c),
            OpenKeybindings => |this, _w, _c| this.open_tab("keybindings", "Keybindings", TabKind::Keybindings, true),
            ReloadKeymap => |this, _w, c| this.reload_keymap(c),
            OpenKeymapFile => |this, _w, c| this.open_config_file("keymap.toml", KEYMAP_TEMPLATE, c),
            OpenSettingsFile => |this, _w, c| this.open_config_file("settings.toml", SETTINGS_TEMPLATE, c),
            TrustCollection => |this, _w, c| this.set_collection_trust(true, c),
            RevokeCollectionTrust => |this, _w, c| this.set_collection_trust(false, c),
            RegenerateFwTypes => |this, _w, c| this.regenerate_fw_types(true, c),
            RunFolder => |this, _w, c| this.run_selected_folder(c),
            OpenDocs => |this, _w, _c| {
                let folder = this.selected_folder_name();
                this.open_tab(&format!("docs/{folder}"), &format!("Docs: {folder}"), TabKind::Docs, true)
            },
            EditActiveEnvironment => |this, _w, _c| {
                match this.active_env_name() {
                    Some(name) => this.open_env_tab(&name),
                    None => this.notice = Some("No active environment to edit.".into()),
                }
            },
            ManageEnvironments => |this, _w, _c| {
                this.scope.left_open = true;
                this.scope.panel = Panel::Environments;
                this.session_dirty = true;
                if let Some(name) = this.active_env_name() {
                    this.open_env_tab(&name)
                }
            },
            OpenSync => |this, _w, _c| this.open_tab("sync", "Sync", TabKind::Sync, true),
            CloseWorkspace => |this, w, c| this.request_close(crate::dialogs::AfterClose::Welcome, w, c),
            OpenWorkspace => |this, w, c| this.open_folder_dialog(w, c),
            NewWorkspace => |this, w, c| this.start_new_workspace(w, c),
            NewCollection => |this, w, c| this.start_new_collection(None, w, c),
            AddCollection => |this, w, c| this.start_add_collection(w, c),
            NewRequest => |this, w, c| this.start_new_request(w, c),
            QuickRequest => |this, w, c| this.route_new(false, false, w, c),
            NewWebSocketRequest => |this, w, c| this.route_new(true, false, w, c),
            NewFolder => |this, w, c| this.route_new(false, true, w, c),
            RenameItem => |this, w, c| this.start_rename(w, c),
            DuplicateItem => |this, w, c| this.duplicate_selected(w, c),
            DeleteItem => |this, w, c| this.delete_selected(w, c),
            RevealInFileManager => |this, _w, c| this.reveal_selected(c),
            ReloadFromDisk => |this, _w, c| this.resolve_conflict(true, c),
            KeepMine => |this, _w, c| this.resolve_conflict(false, c),
            FocusUrlBar => |this, w, c| this.focus_url(w, c),
            Save => |this, _w, c| this.save_active(c),
            Import => |this, _w, _c| this.notice = Some("Import arrives in a later milestone (M11).".into()),
            OpenAsText => |this, _w, c| this.toggle_text_mode(c),
            EditorGoToLine => |this, w, c| this.start_go_to_line(w, c),
            RequestFormatBody => |this, _w, c| this.format_body(c),
            Quit => |this, w, c| this.request_close(crate::dialogs::AfterClose::Quit, w, c),
            Send => |this, w, c| if matches!(this.dialog, Some(crate::dialogs::Dialog::NewRequest(_))) { this.nr_enter(true, w, c) } else if this.is_ws_tab() { this.ws_send_action(w, c) } else { this.send_or_cancel(c) },
            CancelRequest => |this, _w, c| if !this.cancel_request(c) { this.scope.method_menu = false },
            CopyAsCurl => |this, _w, c| this.copy_curl(false, c),
            CopyAsCurlWithSecrets => |this, _w, c| this.copy_curl(true, c),
            FindInResponse => |this, w, c| {
                if let Some(id) = this.scope.tabs.active_id()
                    && let Some(i) = this.viewer_input(id, crate::response::ViewerInput::Find, c)
                {
                    w.focus(&i.focus_handle(c));
                }
            },
            CopyResponseBody => |this, _w, c| this.copy_response_body(c),
            SaveResponseBody => |this, _w, c| this.save_response_body(c),
            Connect => |this, _w, c| this.ws_toggle(c),
        });
        root.child(self.render_title_bar(&t, cx))
            .child(body)
            .child(self.render_status_bar(&t, cx))
            .children(self.render_modal(&t, cx))
            .children(self.render_dialog(&t, cx))
            .children(self.render_tree_menu(&t, cx))
            .children(self.render_tab_menu(&t, cx))
            .children(self.render_history_menu(&t, cx))
            .children(self.render_history_nav_menu(&t, cx))
            .children(self.render_switcher(&t))
    }
}

const KEYMAP_TEMPLATE: &str = "# Fluxwire key bindings (overrides). Saved changes apply immediately.\n# A binding for an action replaces that action's default; `\"cmd-k\" = \"none\"` unbinds a key.\n#\n# [[bindings]]\n# context = \"RequestEditor\"\n# \"cmd-r\" = \"request::Send\"\n";
const SETTINGS_TEMPLATE: &str =
    "# Fluxwire settings.\n# allow_process_env = false\n# persist_cookies = false\n";

/// A new window, opening `path` as its workspace when given.
pub fn open_window_at(cx: &mut gpui::App, path: Option<PathBuf>) {
    if let Ok(handle) = open_window(cx)
        && let Some(path) = path
    {
        handle
            .update(cx, |ws, window, cx| ws.open_workspace(&path, window, cx))
            .ok();
    }
}

pub fn open_window(cx: &mut gpui::App) -> anyhow::Result<gpui::WindowHandle<WorkspaceView>> {
    use gpui::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, size};
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Fluxwire")),
                appears_transparent: cfg!(target_os = "macos"),
                traffic_light_position: Some(point(px(12.), px(10.))),
            }),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| WorkspaceView::new(window, cx)),
    )?;
    Ok(handle)
}

/// Applies the `theme` setting at startup; an unknown name falls back to the OS default with a
/// notice and leaves the OS appearance in charge.
pub fn install_theme(configured: Option<String>, cx: &mut gpui::App) {
    let os_dark = appearance_of(cx.window_appearance()) == Appearance::Dark;
    let known = configured
        .as_deref()
        .is_some_and(|n| crate::theme::registry().get(n).is_some());
    if let (Some(name), false) = (&configured, known) {
        let fallback = Theme::resolve(None, os_dark);
        cx.set_global(crate::theme::StartupNotice(format!(
            "Unknown theme \"{name}\" in settings.toml; using {}.",
            fallback.name
        )));
    }
    let configured = configured.filter(|_| known);
    cx.set_global(Theme::resolve(configured.as_deref(), os_dark));
    cx.set_global(crate::theme::ThemeChoice { configured });
}

/// Launches the application and blocks until the last window closes.
pub fn run() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    #[cfg(feature = "dev-mcp")]
    run_with(path, HostOpts::default());
    #[cfg(not(feature = "dev-mcp"))]
    run_with(path, ());
}

/// What `run_with` hosts besides the window: the in-process MCP server under `dev-mcp`.
#[cfg(feature = "dev-mcp")]
pub type HostOpts = crate::dev_mcp::HostOpts;
#[cfg(not(feature = "dev-mcp"))]
pub type HostOpts = ();

/// Like [`run`], opening `path` as the Workspace and starting the host in `host`.
pub fn run_with(path: Option<PathBuf>, host: HostOpts) {
    #[cfg(not(feature = "dev-mcp"))]
    let () = host;
    gpui::Application::new()
        .with_assets(crate::assets::Assets)
        .run(move |cx: &mut gpui::App| {
            crate::assets::load_fonts(cx).expect("bundled fonts load");
            crate::actions::install(cx);
            let dirs = directories::ProjectDirs::from("dev", "fluxwire", "fluxwire");
            if let Some(dirs) = &dirs {
                cx.set_global(StateDir(dirs.data_dir().to_path_buf()));
                cx.set_global(crate::envs::ConfigDir(dirs.config_dir().to_path_buf()));
            }
            let configured = dirs
                .as_ref()
                .and_then(|d| fluxwire_format::load_settings(d.config_dir()).theme);
            install_theme(configured, cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let handle = open_window(cx).expect("open main window");
            // `fluxwire [PATH]` opens PATH; the full CLI arrives in M10.
            if let Some(path) = &path {
                handle
                    .update(cx, |ws, window, cx| ws.open_workspace(path, window, cx))
                    .ok();
            }
            #[cfg(feature = "dev-mcp")]
            crate::dev_mcp::start(cx, handle, host, path.as_deref());
            cx.activate(true);
        });
}
