//! The root view: window state, action handlers, title bar, status bar and modals.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use gpui::{
    Bounds, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, WindowAppearance, div, prelude::*, px,
};

use crate::{
    actions::*,
    assets::{BUFFER_FONT, UI_FONT},
    mock::{self, Mock},
    project::{Project, Recent, Target},
    tabs::{TabKind, Tabs},
    theme::{Appearance, Theme, theme},
    ui::{TextEdit, icon, kbd_label, row},
};

/// Where recent workspaces and sessions are stored. Unset in tests, so nothing touches the disk.
#[derive(Clone)]
pub struct StateDir(pub PathBuf);
impl gpui::Global for StateDir {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Project,
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
    /// Single-line text prompt (new request, rename, ...).
    Prompt,
}

#[derive(Clone, Debug)]
pub enum PromptPurpose {
    Rename { key: String },
}

/// A request or folder being named in place, as a row in the tree.
#[derive(Clone, Debug)]
pub struct InlineNew {
    /// Workspace-relative key of the folder (or collection directory) it is created in.
    pub parent: String,
    pub dir: PathBuf,
    pub websocket: bool,
    pub folder: bool,
    pub text: TextEdit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Request,
    Folder,
    Collection,
    Doc,
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
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum ItemRun {
    Action(usize),
    Open {
        key: String,
        title: String,
        kind: TabKind,
    },
    Environment(usize),
}

#[derive(Clone, Debug)]
pub struct ModalItem {
    pub label: String,
    pub detail: String,
    pub run: ItemRun,
}

pub struct Modal {
    pub kind: ModalKind,
    pub query: TextEdit,
    pub selected: usize,
}

/// Per-tab view state (sub-tab selections, URL text).
#[derive(Clone, Debug)]
pub struct TabUi {
    pub sub: usize,
    pub resp: usize,
    pub method: usize,
    pub post_script: bool,
    pub connected: bool,
}

impl TabUi {
    pub fn new() -> Self {
        Self {
            sub: 0,
            resp: 0,
            method: 0,
            post_script: false,
            connected: false,
        }
    }
}

impl Default for TabUi {
    fn default() -> Self {
        Self::new()
    }
}

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub struct Workspace {
    pub mock: Mock,
    pub project: Option<Project>,
    pub tabs: Tabs,
    pub tab_ui: HashMap<u64, TabUi>,
    /// Run state and response per request tab.
    pub responses: HashMap<u64, crate::response::TabResponse>,
    /// Send log shown in the bottom dock.
    pub console: Vec<String>,
    pub left_open: bool,
    pub left_width: f32,
    pub panel: Panel,
    pub bottom_open: bool,
    pub bottom_tab: BottomTab,
    pub split_vertical: bool,
    pub env: usize,
    pub collapsed: HashSet<String>,
    pub modal: Option<Modal>,
    pub prompt: Option<PromptPurpose>,
    pub tree_menu: Option<TreeMenu>,
    pub inline: Option<InlineNew>,
    pub inline_focus: FocusHandle,
    /// Selected tree row (workspace-relative key); target of rename/duplicate/delete/new.
    pub selected: Option<String>,
    pub cell: Option<CellEdit>,
    /// Surface that receives editor key presses.
    pub target: Target,
    /// Transient message for the status bar (errors, notices).
    pub notice: Option<String>,
    pub open_error: Option<String>,
    pub recent: Recent,
    pub state_dir: Option<PathBuf>,
    pub watch_enabled: bool,
    pub watcher: Option<fluxwire_format::WatchHandle>,
    pub session_dirty: bool,
    pub editor_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Selection in the URL bar (char indices, anchor then head) and whether a drag is active.
    pub url_sel: Option<(usize, usize)>,
    pub url_drag: bool,
    pub url_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub editor_scroll: ScrollHandle,
    pub resizing: bool,
    pub method_menu: bool,
    pub root_focus: FocusHandle,
    pub modal_focus: FocusHandle,
    pub url_focus: FocusHandle,
    pub cell_focus: FocusHandle,
    pub editor_focus: FocusHandle,
    pub filter_focus: FocusHandle,
    pub find_focus: FocusHandle,
}

impl Workspace {
    /// A window with no workspace open (the welcome view). Restores the last workspace when a
    /// state directory is configured.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root_focus = cx.focus_handle();
        window.focus(&root_focus);
        window
            .observe_window_appearance(|window, cx| {
                set_theme(
                    Theme::for_appearance(appearance_of(window.appearance())),
                    cx,
                );
            })
            .detach();
        let state_dir = cx.try_global::<StateDir>().map(|s| s.0.clone());
        let recent = state_dir
            .as_deref()
            .map(crate::project::load_recent)
            .unwrap_or_default();
        let mut this = Self {
            mock: mock::payments_api(),
            project: None,
            tabs: Tabs::new(),
            tab_ui: HashMap::new(),
            responses: HashMap::new(),
            console: Vec::new(),
            left_open: true,
            left_width: 260.0,
            panel: Panel::Project,
            bottom_open: false,
            bottom_tab: BottomTab::Console,
            split_vertical: true,
            env: 1,
            collapsed: HashSet::new(),
            modal: None,
            prompt: None,
            tree_menu: None,
            inline: None,
            inline_focus: cx.focus_handle(),
            selected: None,
            cell: None,
            target: Target::Body,
            notice: None,
            open_error: None,
            recent,
            watch_enabled: state_dir.is_some(),
            state_dir,
            watcher: None,
            session_dirty: false,
            editor_bounds: Rc::new(Cell::new(Bounds::default())),
            url_sel: None,
            url_drag: false,
            url_bounds: Rc::new(Cell::new(Bounds::default())),
            editor_scroll: ScrollHandle::new(),
            resizing: false,
            method_menu: false,
            root_focus,
            modal_focus: cx.focus_handle(),
            url_focus: cx.focus_handle(),
            cell_focus: cx.focus_handle(),
            editor_focus: cx.focus_handle(),
            filter_focus: cx.focus_handle(),
            find_focus: cx.focus_handle(),
        };
        this.start_session_timer(cx);
        cx.on_app_quit(|this, _cx| {
            this.save_session_now();
            async {}
        })
        .detach();
        if let Some(last) = this.recent.last.clone() {
            let p = PathBuf::from(last);
            if p.is_dir() {
                this.open_project(&p, window, cx);
            }
        }
        this
    }

    pub fn active_ui(&mut self) -> Option<&mut TabUi> {
        let id = self.tabs.active_id()?;
        self.tab_ui.get_mut(&id)
    }

    /// Key of the file shown by the active request tab.
    pub fn active_key(&self) -> Option<String> {
        let t = self.tabs.active()?;
        matches!(t.kind, TabKind::Http { .. } | TabKind::WebSocket).then(|| t.key.clone())
    }

    /// Open non-request tabs (history, environment, ...), which are still mock-backed.
    pub fn open_tab(&mut self, key: &str, title: &str, kind: TabKind, permanent: bool) {
        let id = if permanent {
            self.tabs.open_permanent(key, title, kind)
        } else {
            self.tabs.open_preview(key, title, kind)
        };
        self.tab_ui.entry(id).or_default();
    }

    // ---- modals ---------------------------------------------------------------------------

    pub fn open_modal(&mut self, kind: ModalKind, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = Some(Modal {
            kind,
            query: TextEdit::default(),
            selected: 0,
        });
        window.focus(&self.modal_focus);
        cx.notify();
    }

    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = None;
        if self.modal.take().is_some() {
            window.focus(&self.root_focus);
            cx.notify();
        }
    }

    /// Items for the open modal, filtered by its query.
    pub fn modal_items(&self) -> Vec<ModalItem> {
        let Some(m) = &self.modal else {
            return vec![];
        };
        let all: Vec<ModalItem> = match m.kind {
            ModalKind::Palette => ACTIONS
                .iter()
                .enumerate()
                .map(|(i, a)| ModalItem {
                    label: a.label.to_string(),
                    detail: a.binding.map(kbd_label).unwrap_or_default(),
                    run: ItemRun::Action(i),
                })
                .collect(),
            ModalKind::QuickSearch => {
                let mut items: Vec<ModalItem> = self
                    .project
                    .as_ref()
                    .map(|p| p.all_requests())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, name, method, kind)| ModalItem {
                        label: name.clone(),
                        detail: format!("{method} {key}"),
                        run: ItemRun::Open {
                            key,
                            title: name,
                            kind: if kind == fluxwire_model::RequestKind::WebSocket {
                                TabKind::WebSocket
                            } else {
                                TabKind::Http { method }
                            },
                        },
                    })
                    .collect();
                items.extend(self.mock.environments.iter().map(|e| ModalItem {
                    label: e.name.to_string(),
                    detail: "environment".into(),
                    run: ItemRun::Open {
                        key: format!("env/{}", e.name),
                        title: e.name.into(),
                        kind: TabKind::Environment,
                    },
                }));
                items
            }
            ModalKind::Prompt => vec![],
            ModalKind::Environment => self
                .mock
                .environments
                .iter()
                .enumerate()
                .map(|(i, e)| ModalItem {
                    label: e.name.to_string(),
                    detail: if i == self.env {
                        "active".into()
                    } else {
                        String::new()
                    },
                    run: ItemRun::Environment(i),
                })
                .collect(),
        };
        let q = m.query.text();
        all.into_iter().filter(|i| fuzzy(q, &i.label)).collect()
    }

    fn run_modal_item(&mut self, item: ModalItem, window: &mut Window, cx: &mut Context<Self>) {
        self.close_modal(window, cx);
        match item.run {
            ItemRun::Action(i) => window.dispatch_action((ACTIONS[i].build)(), cx),
            ItemRun::Open { key, title, kind } => {
                if matches!(kind, TabKind::Http { .. } | TabKind::WebSocket) {
                    self.open_request(&key, true, window, cx);
                } else {
                    self.open_tab(&key, &title, kind, true);
                }
            }
            ItemRun::Environment(i) => self.env = i,
        }
        cx.notify();
    }

    fn on_modal_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.modal_items().len();
        let key = ev.keystroke.key.as_str();
        match key {
            "down" | "up" => {
                if let Some(m) = &mut self.modal {
                    m.selected = if key == "down" {
                        (m.selected + 1).min(count.saturating_sub(1))
                    } else {
                        m.selected.saturating_sub(1)
                    };
                }
            }
            "enter" if self.prompt.is_some() => self.confirm_prompt(window, cx),
            "enter" => {
                let items = self.modal_items();
                let sel = self.modal.as_ref().map_or(0, |m| m.selected);
                if let Some(item) = items.get(sel).cloned() {
                    self.run_modal_item(item, window, cx);
                }
            }
            _ => {
                if let Some(m) = &mut self.modal
                    && m.query.handle_key(ev)
                {
                    m.selected = 0;
                }
            }
        }
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------------------------

    fn render_title_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let env = self.mock.environments[self.env].name;
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
            .child(
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
                    .on_click(
                        cx.listener(|this, _, w, cx| {
                            this.open_modal(ModalKind::Environment, w, cx)
                        }),
                    )
                    .child(div().size(px(7.)).rounded_full().bg(t.modified))
                    .child(env)
                    .child(icon("chevron-down", 12., t.text_muted)),
            )
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
                    if this.panel == p && this.left_open {
                        this.left_open = false;
                    } else {
                        this.panel = p;
                        this.left_open = true;
                    }
                    cx.notify();
                }))
                .child(icon(glyph, 14., if active { t.text } else { t.text_muted }))
        };
        let problems = self.project.as_ref().map_or(0, |p| p.problem_count());
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
                        "p-project",
                        "folder",
                        Panel::Project,
                        self.left_open && self.panel == Panel::Project,
                    ))
                    .child(panel_btn(
                        "p-history",
                        "file-text",
                        Panel::History,
                        self.left_open && self.panel == Panel::History,
                    ))
                    .child(panel_btn(
                        "p-envs",
                        "layers",
                        Panel::Environments,
                        self.left_open && self.panel == Panel::Environments,
                    ))
                    .child(panel_btn(
                        "p-sync",
                        "refresh-cw",
                        Panel::Sync,
                        self.left_open && self.panel == Panel::Sync,
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
                            .when(self.bottom_open, |d| d.bg(t.element_selected))
                            .hover(|s| s.bg(t.element_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.bottom_open = !this.bottom_open;
                                cx.notify();
                            }))
                            .child(icon(
                                "terminal",
                                14.,
                                if self.bottom_open {
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
                        this.bottom_open = true;
                        this.bottom_tab = BottomTab::Problems;
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
        let items = self.modal_items();
        let placeholder = match m.kind {
            ModalKind::Palette => "Type a command…",
            ModalKind::QuickSearch => "Search requests, folders, environments…",
            ModalKind::Environment => "Switch environment…",
            ModalKind::Prompt => "Name",
        };
        let title = self.prompt_title();
        let selected = m.selected;
        let query = m.query.text().to_string();
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
                                .child(if query.is_empty() {
                                    div().text_color(t.text_placeholder).child(placeholder)
                                } else {
                                    div().text_color(t.text).child(query)
                                })
                                .child(div().w(px(1.)).h(px(15.)).bg(t.accent)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .max_h(px(360.))
                                .overflow_hidden()
                                .py_1()
                                .children(items.into_iter().take(12).enumerate().map(
                                    |(i, item)| {
                                        let click_item = item.clone();
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
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                this.run_modal_item(click_item.clone(), w, cx)
                                            }))
                                            .child(item.label)
                                            .child(
                                                div()
                                                    .font_family(BUFFER_FONT)
                                                    .text_size(px(11.))
                                                    .text_color(t.text_placeholder)
                                                    .child(item.detail),
                                            )
                                    },
                                )),
                        ),
                ),
        )
    }
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

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.root_focus.clone()
    }
}

macro_rules! handlers {
    ($el:expr, $cx:expr, { $($act:ident => |$this:ident, $w:ident, $c:ident| $body:expr),* $(,)? }) => {{
        let mut el = $el;
        $(
            el = el.on_action($cx.listener(|$this, _: &$act, $w, $c| { let () = { $body }; $c.notify(); }));
        )*
        el
    }};
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let body = if self.project.is_some() {
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
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.resizing {
                    this.left_width = f32::from(ev.position.x).clamp(180.0, 520.0);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing {
                        this.resizing = false;
                        cx.notify();
                    }
                }),
            );
        let root = handlers!(root, cx, {
            QuickSearch => |this, w, c| this.open_modal(ModalKind::QuickSearch, w, c),
            CommandPalette => |this, w, c| this.open_modal(ModalKind::Palette, w, c),
            SwitchEnvironment => |this, w, c| this.open_modal(ModalKind::Environment, w, c),
            Dismiss => |this, w, c| {
                let open = this.method_menu || this.tree_menu.is_some() || this.cell.is_some() || this.inline.is_some() || this.modal.is_some();
                this.method_menu = false; this.tree_menu = None; this.cell = None; this.inline = None; this.close_modal(w, c);
                // Esc with nothing to dismiss cancels an in-flight send
                if !open { this.cancel_request(c); }
            },
            ToggleLeftDock => |this, _w, _c| this.left_open = !this.left_open,
            ToggleBottomDock => |this, _w, _c| this.bottom_open = !this.bottom_open,
            ShowProjectPanel => |this, _w, _c| { this.panel = Panel::Project; this.left_open = true },
            ShowHistoryPanel => |this, _w, _c| { this.panel = Panel::History; this.left_open = true },
            ShowEnvironmentsPanel => |this, _w, _c| { this.panel = Panel::Environments; this.left_open = true },
            ShowSyncPanel => |this, _w, _c| { this.panel = Panel::Sync; this.left_open = true },
            ShowConsole => |this, _w, _c| { this.bottom_tab = BottomTab::Console; this.bottom_open = true },
            ShowProblems => |this, _w, _c| { this.bottom_tab = BottomTab::Problems; this.bottom_open = true },
            ToggleTheme => |_this, _w, c| {
                let next = match theme(c).appearance {
                    Appearance::Dark => Appearance::Light,
                    Appearance::Light => Appearance::Dark,
                };
                set_theme(Theme::for_appearance(next), c);
            },
            ToggleSplitOrientation => |this, _w, _c| this.split_vertical = !this.split_vertical,
            CloseTab => |this, _w, _c| if let Some(id) = this.tabs.active_id() { this.tabs.close(id); this.session_dirty = true },
            PinTab => |this, _w, _c| if let Some(id) = this.tabs.active_id() { this.tabs.toggle_pin(id) },
            NextTab => |this, _w, _c| this.tabs.cycle(1),
            PrevTab => |this, _w, _c| this.tabs.cycle(-1),
            ReopenClosedTab => |this, _w, _c| { this.tabs.reopen_closed(); },
            GroupTabsByKind => |this, _w, _c| this.tabs.group_by_kind = !this.tabs.group_by_kind,
            RunFolder => |this, _w, _c| {
                let folder = this.selected_folder_name();
                this.open_tab(&format!("run/{folder}"), &format!("Run: {folder}"), TabKind::FolderRun, true)
            },
            OpenDocs => |this, _w, _c| {
                let folder = this.selected_folder_name();
                this.open_tab(&format!("docs/{folder}"), &format!("Docs: {folder}"), TabKind::Docs, true)
            },
            OpenEnvironment => |this, _w, _c| {
                let name = this.mock.environments[this.env].name;
                this.open_tab(&format!("env/{name}"), name, TabKind::Environment, true)
            },
            OpenSync => |this, _w, _c| this.open_tab("sync", "Sync", TabKind::Sync, true),
            CloseWorkspace => |this, _w, _c| this.close_project(),
            OpenFolder => |this, w, c| this.open_folder_dialog(false, w, c),
            NewCollection => |this, w, c| this.open_folder_dialog(true, w, c),
            NewRequest => |this, w, c| this.start_new(false, false, w, c),
            NewWebSocketRequest => |this, w, c| this.start_new(true, false, w, c),
            NewFolder => |this, w, c| this.start_new(false, true, w, c),
            RenameItem => |this, w, c| this.start_rename(w, c),
            DuplicateItem => |this, w, c| this.duplicate_selected(w, c),
            DeleteItem => |this, w, c| this.delete_selected(w, c),
            RevealInFileManager => |this, _w, c| this.reveal_selected(c),
            ReloadFromDisk => |this, _w, c| this.resolve_conflict(true, c),
            KeepMine => |this, _w, c| this.resolve_conflict(false, c),
            FocusUrlBar => |this, w, _c| w.focus(&this.url_focus),
            Save => |this, _w, c| this.save_active(c),
            Import => |this, _w, _c| this.notice = Some("Import arrives in a later milestone (M11).".into()),
            OpenAsText => |this, _w, c| this.toggle_text_mode(c),
            Quit => |this, _w, c| { this.save_session_now(); c.quit() },
            Send => |this, _w, c| this.send_or_cancel(c),
            CancelRequest => |this, _w, c| if !this.cancel_request(c) { this.method_menu = false },
            CopyAsCurl => |this, _w, c| this.copy_curl(false, c),
            CopyAsCurlWithSecrets => |this, _w, c| this.copy_curl(true, c),
            FindInResponse => |this, w, _c| w.focus(&this.find_focus),
            CopyResponseBody => |this, _w, c| this.copy_response_body(c),
            SaveResponseBody => |this, _w, c| this.save_response_body(c),
            Connect => |this, _w, _c| if let Some(ui) = this.active_ui() { ui.connected = !ui.connected },
        });
        root.child(self.render_title_bar(&t, cx))
            .child(body)
            .child(self.render_status_bar(&t, cx))
            .children(self.render_modal(&t, cx))
            .children(self.render_tree_menu(&t, cx))
    }
}

pub fn open_window(cx: &mut gpui::App) -> anyhow::Result<gpui::WindowHandle<Workspace>> {
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
        |window, cx| cx.new(|cx| Workspace::new(window, cx)),
    )?;
    Ok(handle)
}

/// Launches the application and blocks until the last window closes.
pub fn run() {
    gpui::Application::new()
        .with_assets(crate::assets::Assets)
        .run(|cx: &mut gpui::App| {
            crate::assets::load_fonts(cx).expect("bundled fonts load");
            let initial = Theme::for_appearance(appearance_of(cx.window_appearance()));
            cx.set_global(initial);
            crate::actions::install(cx);
            if let Some(dirs) = directories::ProjectDirs::from("dev", "fluxwire", "fluxwire") {
                cx.set_global(StateDir(dirs.data_dir().to_path_buf()));
            }
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let handle = open_window(cx).expect("open main window");
            // `fluxwire [PATH]` opens PATH; the full CLI arrives in M10.
            if let Some(path) = std::env::args()
                .nth(1)
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
            {
                handle
                    .update(cx, |ws, window, cx| ws.open_project(&path, window, cx))
                    .ok();
            }
            cx.activate(true);
        });
}
