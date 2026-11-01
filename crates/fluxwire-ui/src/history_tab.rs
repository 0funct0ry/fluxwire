//! The History tab (SPEC §13.1): one per Workspace, a master/detail viewer. This file holds the
//! tab itself, its state changes (selection, pin, note, delete, export) and the keys; the table,
//! toolbar and detail pane live in `history_table.rs`, `history_filters.rs` and
//! `history_detail.rs`.

use fluxwire_store::{ExportFormat, Redactor};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Styled, Window, div, prelude::*, px,
};

use crate::{
    dialogs::{Confirm, ConfirmAction, Dialog},
    history::blocking,
    history_query::HistRow,
    tabs::TabKind,
    text_input::{TextInput, TextInputEvent},
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
};

pub const TAB_KEY: &str = "history";

impl WorkspaceView {
    /// Open the workspace's History tab, or focus it when it is already open.
    pub fn open_history_tab(&mut self, cx: &mut Context<Self>) {
        self.open_tab(TAB_KEY, "History", TabKind::History, true);
        self.session_dirty = true;
        cx.notify();
    }

    /// `history: open`.
    pub fn history_open(&mut self, cx: &mut Context<Self>) {
        self.open_history_tab(cx);
        self.history_reload(cx);
    }

    /// The entry ids a row action applies to: the ticked rows, else the selected one.
    pub fn history_targets(&self) -> Vec<i64> {
        if self.history.ticked.is_empty() {
            self.history.selected.into_iter().collect()
        } else {
            self.history.ticked.iter().copied().collect()
        }
    }

    pub fn history_select(&mut self, id: i64, cx: &mut Context<Self>) {
        self.history_commit_note(cx);
        self.history.selected = Some(id);
        self.history.note_for = None;
        self.history_note_input(cx);
        self.history_fetch(vec![id], cx, move |this, cx| this.history_load_note(id, cx));
        cx.notify();
    }

    pub fn history_toggle_tick(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.history.ticked.remove(&id) {
            self.history.ticked.insert(id);
        }
        cx.notify();
    }

    /// Diff the two ticked rows.
    pub fn history_diff_ticked(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<i64> = self.history.ticked.iter().copied().collect();
        if let [a, b] = ids[..] {
            self.open_history_diff(a, b, cx);
        }
    }

    /// Diff an entry against the last earlier send of the same request.
    pub fn history_compare_previous(&mut self, id: i64, cx: &mut Context<Self>) {
        let (Some(store), true) = (self.history.store.clone(), true) else {
            return;
        };
        let task = blocking(move || store.previous_of(id).ok().flatten());
        cx.spawn(async move |this, cx| {
            let prev = task.await.flatten();
            this.update(cx, |this, cx| match prev {
                Some(p) => this.open_history_diff(p, id, cx),
                None => {
                    this.notice = Some("No earlier send of this request.".into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Pin every target, or unpin them when all are pinned.
    pub fn history_toggle_pin(&mut self, cx: &mut Context<Self>) {
        let ids = self.history_targets();
        let all_pinned = !ids.is_empty()
            && ids.iter().all(|id| {
                self.history
                    .entries
                    .iter()
                    .find(|e| e.id == *id)
                    .is_some_and(|e| e.pinned)
            });
        self.history_set_pinned(ids, !all_pinned, cx);
    }

    pub fn history_set_pinned(&mut self, ids: Vec<i64>, pinned: bool, cx: &mut Context<Self>) {
        let Some(store) = self.history.store.clone() else {
            return;
        };
        if ids.is_empty() {
            return;
        }
        let task = blocking(move || {
            for id in ids {
                let _ = store.set_pinned(id, pinned);
            }
        });
        cx.spawn(async move |this, cx| {
            task.await;
            this.update(cx, |this, cx| {
                this.history.cache.clear();
                this.history_reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Save the note field's text when it changed (also on blur).
    pub fn history_commit_note(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(input), Some(store)) = (
            self.history.note_for,
            self.history.note_input.clone(),
            self.history.store.clone(),
        ) else {
            return;
        };
        let text = input.read(cx).text().to_string();
        let old = self
            .history
            .entries
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.note.clone())
            .unwrap_or_default();
        if text.trim() == old {
            return;
        }
        let note = Some(text).filter(|t| !t.trim().is_empty());
        let task = blocking(move || {
            let _ = store.set_note(id, note, &Redactor::default());
        });
        cx.spawn(async move |this, cx| {
            task.await;
            this.update(cx, |this, cx| {
                this.history.cache.remove(&id);
                this.history_reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The note field, created on first use and saved on blur.
    pub(crate) fn history_note_input(&mut self, cx: &mut Context<Self>) -> gpui::Entity<TextInput> {
        if let Some(i) = self.history.note_input.clone() {
            return i;
        }
        let input = cx.new(|cx| TextInput::new("Add a note", cx));
        cx.subscribe(&input, |this, _, ev: &TextInputEvent, cx| {
            if *ev == TextInputEvent::Blur {
                this.history_commit_note(cx);
            }
        })
        .detach();
        self.history.note_input = Some(input.clone());
        input
    }

    /// Ask before deleting entries.
    pub fn history_confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.history_targets();
        if ids.is_empty() {
            return;
        }
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: format!(
                "Delete {} history entr{}?",
                ids.len(),
                if ids.len() == 1 { "y" } else { "ies" }
            ),
            lines: vec!["Pinned entries are deleted too when selected.".into()],
            buttons: vec![
                ("Delete".into(), ConfirmAction::DeleteHistory(ids)),
                ("Cancel".into(), ConfirmAction::Cancel),
            ],
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    pub fn history_delete(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        let Some(store) = self.history.store.clone() else {
            return;
        };
        let task = blocking(move || store.delete(&ids).unwrap_or(0));
        cx.spawn(async move |this, cx| {
            let n = task.await.unwrap_or(0);
            this.update(cx, |this, cx| {
                this.history.cache.clear();
                this.history.selected = None;
                this.history.ticked.clear();
                this.notice = Some(format!("Deleted {n} history entries."));
                this.history_reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Export the whole filtered set (not just the loaded page) to a file.
    pub fn history_export(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let (Some(store), Some(ws)) = (self.history.store.clone(), self.history.ws_key.clone())
        else {
            return;
        };
        self.history.export_menu = false;
        let filter = self.history.filter(None, 0);
        let name = match format {
            ExportFormat::Json => "fluxwire-history.json",
            ExportFormat::Csv => "fluxwire-history.csv",
        };
        let dir = self
            .scope
            .workspace
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let task = blocking(move || store.export(&ws, &filter, format));
        let rx = cx.prompt_for_new_path(&dir, Some(name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let text = task.await.and_then(Result::ok).unwrap_or_default();
            let msg = match std::fs::write(&path, text) {
                Ok(()) => format!("Saved {}", path.display()),
                Err(e) => format!("Could not save {}: {e}", path.display()),
            };
            this.update(cx, |this, cx| {
                this.notice = Some(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Clear every filter and the search box.
    pub fn history_clear_filters(&mut self, cx: &mut Context<Self>) {
        self.history.query.clear();
        self.history.query.nav = None;
        if let Some(i) = &self.history.input {
            i.update(cx, |i, cx| i.set_text("", cx));
        }
        self.history_reload(cx);
    }

    pub fn history_toggle_run(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.history.expanded.remove(id) {
            self.history.expanded.insert(id.to_string());
        }
        self.history.rebuild_rows();
        cx.notify();
    }

    pub fn history_set_sort(&mut self, col: fluxwire_store::SortCol, cx: &mut Context<Self>) {
        self.history.query.sort = crate::history_query::next_sort(self.history.query.sort, col);
        self.session_dirty = true;
        self.history_reload(cx);
    }

    /// Move the selection to the previous or next visible entry row.
    fn history_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let rows = self.history.rows.clone();
        let visible: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                HistRow::Entry(i) => Some(*i),
                _ => None,
            })
            .collect();
        if visible.is_empty() {
            return;
        }
        let cur = self.history.selected.and_then(|id| {
            visible
                .iter()
                .position(|i| self.history.entries[*i].id == id)
        });
        let next = match (cur, down) {
            (None, _) => 0,
            (Some(c), true) => (c + 1).min(visible.len() - 1),
            (Some(c), false) => c.saturating_sub(1),
        };
        let id = self.history.entries[visible[next]].id;
        self.history_select(id, cx);
        if let Some(pos) = rows
            .iter()
            .position(|r| *r == HistRow::Entry(visible[next]))
        {
            self.history
                .scroll
                .scroll_to_item(pos, gpui::ScrollStrategy::Center);
        }
    }

    /// Table keys (SPEC §13.1). They only act when the table, not a text field, has focus.
    pub fn on_history_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &ev.keystroke;
        if ks.key == "escape" {
            if self.history.query.has_filters() {
                self.history_clear_filters(cx);
            }
            self.history.clear_menu = false;
            self.history.export_menu = false;
            return;
        }
        if !self.history_focus.is_focused(window)
            || ks.modifiers.platform
            || ks.modifiers.control
            || ks.modifiers.alt
        {
            return;
        }
        let sel = self.history.selected;
        match ks.key.as_str() {
            "down" => self.history_move(true, cx),
            "up" => self.history_move(false, cx),
            "enter" => {
                if let Some(e) = sel.and_then(|id| self.history.entries.iter().find(|e| e.id == id))
                {
                    let (id, key) = (e.id, e.request_path.clone());
                    self.history_restore(id, key, window, cx);
                }
            }
            "space" => {
                if let Some(id) = sel {
                    self.open_history_entry(id, true, cx);
                }
            }
            "d" => {
                if let Some(id) = sel {
                    self.history_toggle_tick(id, cx);
                    if self.history.ticked.len() == 2 {
                        self.history_diff_ticked(cx);
                    }
                }
            }
            "p" => self.history_toggle_pin(cx),
            "n" => {
                if sel.is_some() {
                    let i = self.history_note_input(cx);
                    i.update(cx, |i, _| i.focus(window));
                }
            }
            "delete" | "backspace" => self.history_confirm_delete(window, cx),
            "/" => self.history_focus_search(window, cx),
            _ => {}
        }
        cx.stop_propagation();
    }

    /// `history: focus search`.
    pub fn history_focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_history_tab(cx);
        if let Some(i) = &self.history.input {
            i.update(cx, |i, _| i.focus(window));
        }
    }

    pub(crate) fn render_history_viewer(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let empty = self.history.ws_key.is_none()
            || (self.history.entries.is_empty()
                && !self.history.query.has_filters()
                && self.history.counts.requests.is_empty());
        let header = self.render_history_header(t, cx);
        let body: AnyElement = if empty {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.text_placeholder)
                .child(if self.history.ws_key.is_none() {
                    "Open a workspace to see its history."
                } else {
                    "Send a request to start a history"
                })
                .into_any_element()
        } else {
            let toolbar = self.render_history_toolbar(t, cx);
            let stats = self.render_history_stats(t);
            let table = self.render_history_table(t, cx);
            let detail = self.render_history_detail(t, cx);
            let divider = t.border_variant;
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(toolbar)
                .children(stats)
                .child(
                    row()
                        .flex_1()
                        .min_h_0()
                        .child(table)
                        .child(
                            div()
                                .id("hist-divider")
                                .w(px(5.))
                                .h_full()
                                .flex_none()
                                .cursor_col_resize()
                                .border_l_1()
                                .border_color(divider)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                                        this.history.resizing =
                                            Some(crate::history::HistResize::Divider(
                                                f32::from(ev.position.x),
                                                this.history.divider,
                                            ));
                                        cx.notify();
                                    }),
                                ),
                        )
                        .child(detail),
                )
                .into_any_element()
        };
        div()
            .id("history-viewer")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Pointer drags of column edges and the detail divider (called from the root handlers).
    pub(crate) fn history_drag(&mut self, x: f32, cx: &mut Context<Self>) {
        use crate::history::HistResize;
        match self.history.resizing {
            Some(HistResize::Col(i, x0, w0)) => {
                self.history.widths[i] = crate::history_query::clamp_width(w0 + x - x0);
                cx.notify();
            }
            Some(HistResize::Divider(x0, w0)) => {
                self.history.divider = (w0 - (x - x0)).clamp(240.0, 900.0);
                cx.notify();
            }
            None => {}
        }
    }

    pub(crate) fn history_drag_end(&mut self, cx: &mut Context<Self>) -> bool {
        if self.history.resizing.take().is_some() {
            self.session_dirty = true;
            cx.notify();
            return true;
        }
        false
    }
}
