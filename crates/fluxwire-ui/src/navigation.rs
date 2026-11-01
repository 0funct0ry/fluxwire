//! Tab switching, tab menu, keymap loading and layout persistence glue (M6).

use std::path::PathBuf;

use gpui::{
    ClipboardItem, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored as popover, deferred, div, prelude::*, px,
};

use crate::{
    actions::KeymapState,
    envs::ConfigDir,
    tabs::TabKind,
    theme::Theme,
    ui::row,
    workspace::{BottomTab, Panel, Switcher, WorkspaceView},
    workspace_model::{Layout, SessionTab},
};

/// Stable tag for restoring a tab kind from the session file.
pub fn kind_tag(kind: &TabKind) -> &'static str {
    match kind {
        TabKind::Http { .. } | TabKind::WebSocket => "",
        TabKind::FolderRun => "run",
        TabKind::Docs => "docs",
        TabKind::Environment => "env",
        TabKind::History => "history",
        TabKind::Sync => "sync",
        TabKind::Keybindings => "keys",
    }
}

pub fn kind_from_tag_pub(tag: &str) -> Option<TabKind> {
    kind_from_tag(tag)
}

fn kind_from_tag(tag: &str) -> Option<TabKind> {
    Some(match tag {
        "run" => TabKind::FolderRun,
        "docs" => TabKind::Docs,
        "env" => TabKind::Environment,
        "history" => TabKind::History,
        "sync" => TabKind::Sync,
        "keys" => TabKind::Keybindings,
        _ => return None,
    })
}

impl WorkspaceView {
    // ---- Ctrl+Tab recency switcher --------------------------------------------------------

    /// One press of next/previous: preview the next tab by recency without reordering it.
    pub fn switch_tab(&mut self, delta: isize) {
        let sw = match self.switcher.take() {
            Some(sw) => sw,
            None => Switcher {
                order: self.scope.tabs.mru_order(),
                idx: 0,
            },
        };
        let n = sw.order.len();
        if n < 2 {
            return;
        }
        let idx = (sw.idx as isize + delta).rem_euclid(n as isize) as usize;
        self.scope.tabs.peek(sw.order[idx]);
        self.switcher = Some(Switcher {
            order: sw.order,
            idx,
        });
    }

    /// The modifier was released: the previewed tab becomes the most recent one.
    pub fn commit_switcher(&mut self) {
        if let Some(sw) = self.switcher.take()
            && let Some(id) = sw.order.get(sw.idx)
        {
            self.scope.tabs.activate(*id);
            self.session_dirty = true;
        }
    }

    pub(crate) fn render_switcher(&self, t: &Theme) -> Option<gpui::AnyElement> {
        let sw = self.switcher.as_ref()?;
        let t = t.clone();
        Some(
            div()
                .absolute()
                .size_full()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .child(
                    div()
                        .w(px(360.))
                        .py_1()
                        .flex()
                        .flex_col()
                        .bg(t.elevated)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(6.))
                        .shadow_lg()
                        .children(sw.order.iter().enumerate().filter_map(|(i, id)| {
                            let tab = self.scope.tabs.get(*id)?;
                            Some(
                                row()
                                    .h(px(26.))
                                    .px_3()
                                    .gap_2()
                                    .text_size(px(13.))
                                    .text_color(t.text)
                                    .when(i == sw.idx, |d| d.bg(t.element_selected))
                                    .child(
                                        div()
                                            .w(px(40.))
                                            .text_size(px(10.))
                                            .text_color(t.text_muted)
                                            .child(tab.kind.glyph()),
                                    )
                                    .child(tab.title.clone()),
                            )
                        })),
                )
                .into_any_element(),
        )
    }

    // ---- tab operations ---------------------------------------------------------------------

    /// Cmd/Ctrl+Shift+T: reopen the last closed tab that can still be opened.
    pub fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(t) = self.scope.tabs.pop_closed() {
            match t.kind {
                TabKind::Http { .. } | TabKind::WebSocket => {
                    self.open_request(&t.key, true, window, cx);
                    if self.scope.tabs.id_for_key(&t.key).is_none() {
                        continue; // file is gone: try the one before it
                    }
                }
                TabKind::Environment => {
                    if let Some((root, name)) = self.parse_env_tab_key(&t.key) {
                        self.open_env_doc(&root, &name, true);
                    }
                }
                TabKind::History => {
                    match t.key.strip_prefix("history/").and_then(|n| n.parse().ok()) {
                        Some(id) => self.open_history_entry(id, true, cx),
                        None => self.open_tab(&t.key, &t.title, t.kind.clone(), true),
                    }
                }
                _ => self.open_tab(&t.key, &t.title, t.kind.clone(), true),
            }
            if t.pinned
                && let Some(id) = self.scope.tabs.id_for_key(&t.key)
                && self.scope.tabs.get(id).is_some_and(|x| !x.pinned)
            {
                self.scope.tabs.toggle_pin(id);
            }
            break;
        }
        self.session_dirty = true;
    }

    pub fn copy_tab_path(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.scope.tabs.get(id) else {
            return;
        };
        let text = match (&tab.kind, &self.scope.workspace) {
            (TabKind::Http { .. } | TabKind::WebSocket, Some(p)) => {
                p.abs_of(&tab.key).display().to_string()
            }
            _ => tab.title.clone(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// Select request sub-tab `ix` (Params … Settings) on the active HTTP tab.
    pub fn show_request_tab(&mut self, ix: usize) {
        if matches!(
            self.scope.tabs.active().map(|t| &t.kind),
            Some(TabKind::Http { .. })
        ) && let Some(ui) = self.active_ui()
        {
            ui.sub = ix;
        }
    }

    pub fn show_response_tab(&mut self, ix: usize) {
        if matches!(
            self.scope.tabs.active().map(|t| &t.kind),
            Some(TabKind::Http { .. })
        ) && let Some(ui) = self.active_ui()
        {
            ui.resp = ix;
        }
    }

    pub(crate) fn render_tab_menu(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (id, pos) = self.scope.tab_menu?;
        let tab = self.scope.tabs.get(id)?;
        type Act = fn(&mut WorkspaceView, u64, &mut Window, &mut Context<WorkspaceView>);
        let items: Vec<(&'static str, Act)> = vec![
            (if tab.pinned { "Unpin" } else { "Pin" }, |w, id, _, _| {
                w.scope.tabs.toggle_pin(id)
            }),
            ("Close", |w, id, _, _| w.scope.tabs.close(id)),
            ("Close others", |w, id, _, _| w.scope.tabs.close_others(id)),
            ("Close to the right", |w, id, _, _| {
                w.scope.tabs.close_to_right(id)
            }),
            ("Close clean tabs", |w, _, _, _| w.scope.tabs.close_clean()),
            ("Close all", |w, _, _, _| w.scope.tabs.close_all()),
            ("Copy path", |w, id, _, cx| w.copy_tab_path(id, cx)),
            ("Reopen closed tab", |w, _, win, cx| {
                w.reopen_closed_tab(win, cx)
            }),
        ];
        let t = t.clone();
        Some(
            deferred(
                popover().position(pos).child(
                    div()
                        .id("tab-menu")
                        .w(px(200.))
                        .py_1()
                        .flex()
                        .flex_col()
                        .bg(t.elevated)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(6.))
                        .shadow_lg()
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.scope.tab_menu = None;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .children(items.into_iter().enumerate().map(|(i, (label, act))| {
                            let hover = t.element_hover;
                            row()
                                .id(("tab-menu-item", i))
                                .h(px(26.))
                                .mx_1()
                                .px_2()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.scope.tab_menu = None;
                                    act(this, id, w, cx);
                                    this.session_dirty = true;
                                    cx.notify();
                                }))
                                .child(label)
                        })),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    // ---- scroll positions -------------------------------------------------------------------

    /// Keep `editor_scroll` showing the active tab's own offset: save the old tab's, load the new.
    pub fn sync_scroll(&mut self) {
        let active = self.scope.tabs.active_id();
        if self.scope.scroll_for == active {
            return;
        }
        if let Some(old) = self.scope.scroll_for {
            let y = -f32::from(self.editor_scroll.offset().y);
            if let Some(ui) = self.scope.tab_ui.get_mut(&old) {
                ui.scroll = y;
            }
        }
        let y = active
            .and_then(|id| self.scope.tab_ui.get(&id))
            .map_or(0.0, |u| u.scroll);
        self.editor_scroll.set_offset(gpui::point(px(0.), -px(y)));
        self.scope.scroll_for = active;
    }

    /// Current scroll of a tab: live for the one on screen, saved otherwise.
    pub fn tab_scroll(&self, id: u64) -> f32 {
        if self.scope.scroll_for == Some(id) {
            -f32::from(self.editor_scroll.offset().y)
        } else {
            self.scope.tab_ui.get(&id).map_or(0.0, |u| u.scroll)
        }
    }

    // ---- layout -----------------------------------------------------------------------------

    pub fn layout(&self) -> Layout {
        Layout {
            left_open: self.scope.left_open,
            left_width: self.scope.left_width,
            panel: match self.scope.panel {
                Panel::Workspace => "workspace",
                Panel::History => "history",
                Panel::Environments => "environments",
                Panel::Sync => "sync",
            }
            .into(),
            bottom_open: self.scope.bottom_open,
            bottom_tab: match self.scope.bottom_tab {
                BottomTab::Console => "console",
                BottomTab::Problems => "problems",
            }
            .into(),
            bottom_height: self.scope.bottom_height,
            split_vertical: self.scope.split_vertical,
            group_by_kind: self.scope.tabs.group_by_kind,
        }
    }

    pub fn apply_layout(&mut self, l: &Layout) {
        self.scope.left_open = l.left_open;
        self.scope.left_width = l.left_width.clamp(180.0, 520.0);
        self.scope.panel = match l.panel.as_str() {
            "workspace" | "project" => Panel::Workspace, // legacy-project value
            "history" => Panel::History,
            "environments" => Panel::Environments,
            "sync" => Panel::Sync,
            _ => Panel::Workspace,
        };
        self.scope.bottom_open = l.bottom_open;
        self.scope.bottom_tab = if l.bottom_tab == "problems" {
            BottomTab::Problems
        } else {
            BottomTab::Console
        };
        self.scope.bottom_height = l.bottom_height.clamp(80.0, 600.0);
        self.scope.split_vertical = l.split_vertical;
        self.scope.tabs.group_by_kind = l.group_by_kind;
    }

    /// Session entry for a tab (open or closed).
    pub(crate) fn session_tab(&self, t: &crate::tabs::Tab) -> SessionTab {
        SessionTab {
            key: t.key.clone(),
            preview: t.preview,
            pinned: t.pinned,
            kind: kind_tag(&t.kind).into(),
            title: t.title.clone(),
            scroll: self.tab_scroll(t.id),
        }
    }

    /// Reopen a saved tab of a non-request kind. Returns false if it cannot be restored.
    pub(crate) fn restore_other_tab(&mut self, t: &SessionTab, cx: &mut Context<Self>) -> bool {
        let Some(kind) = kind_from_tag(&t.kind) else {
            return false;
        };
        let title = if t.title.is_empty() {
            t.key.clone()
        } else {
            t.title.clone()
        };
        match kind {
            TabKind::Environment => {
                let Some((root, name)) = self.parse_env_tab_key(&t.key) else {
                    return false;
                };
                if !fluxwire_format::list_environments(&root).contains(&name) {
                    return false;
                }
                self.open_env_doc(&root, &name, !t.preview);
            }
            TabKind::History => {
                let Some(id) = t
                    .key
                    .strip_prefix("history/")
                    .and_then(|n| n.parse::<i64>().ok())
                else {
                    return false;
                };
                self.open_history_entry(id, !t.preview, cx);
            }
            _ => self.open_tab(&t.key, &title, kind, !t.preview),
        }
        true
    }

    /// Remove a workspace from the recent list (welcome view).
    pub fn forget_recent(&mut self, path: &str) {
        self.recent.workspaces.retain(|w| w != path);
        if self.recent.last.as_deref() == Some(path) {
            self.recent.last = None;
        }
        if let Some(d) = &self.state_dir {
            crate::workspace_model::save_recent(d, &self.recent);
        }
    }

    // ---- keymap -----------------------------------------------------------------------------

    /// (Re)load `keymap.toml` from the config dir and rebind every key, live.
    pub fn reload_keymap(&mut self, cx: &mut Context<Self>) {
        let km = cx
            .try_global::<ConfigDir>()
            .map(|c| fluxwire_format::load_keymap(&c.0))
            .unwrap_or_default();
        crate::actions::apply_keymap(cx, &km);
        self.keymap_issues = cx
            .try_global::<KeymapState>()
            .map(|s| s.issues.clone())
            .unwrap_or_default();
        cx.notify();
    }

    pub(crate) fn start_config_watching(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = cx.try_global::<ConfigDir>().map(|c| c.0.clone()) else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(handle) = fluxwire_format::watch(&dir, tx) else {
            return;
        };
        self.keymap_watcher = Some(handle);
        let (atx, arx) = smol::channel::unbounded::<Vec<PathBuf>>();
        std::thread::spawn(move || {
            while let Ok(batch) = rx.recv() {
                if atx.send_blocking(batch).is_err() {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(batch) = arx.recv().await {
                let res = this.update(cx, |this, cx| {
                    if batch
                        .iter()
                        .any(|p| p.file_name().is_some_and(|n| n == "keymap.toml"))
                    {
                        this.reload_keymap(cx);
                    }
                    if batch
                        .iter()
                        .any(|p| p.file_name().is_some_and(|n| n == "settings.toml"))
                    {
                        this.scope.envs.settings = fluxwire_format::load_settings(
                            batch[0].parent().unwrap_or(std::path::Path::new(".")),
                        );
                        this.scope.envs.invalidate();
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Open a config file in the system editor, creating it from `template` first if missing.
    pub fn open_config_file(&mut self, name: &str, template: &str, cx: &mut Context<Self>) {
        let Some(dir) = cx.try_global::<ConfigDir>().map(|c| c.0.clone()) else {
            self.notice = Some("No user config directory in this session.".into());
            return;
        };
        let path = dir.join(name);
        let _ = std::fs::create_dir_all(&dir);
        if !path.exists() {
            let _ = std::fs::write(&path, template);
        }
        cx.open_with_system(&path);
    }
}
