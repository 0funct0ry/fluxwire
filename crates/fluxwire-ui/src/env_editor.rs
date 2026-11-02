//! The Environment editor tab (M9.02, SPEC §17.6): a table whose Name and Value cells are always
//! live text inputs, edits that stay in memory until Cmd/Ctrl+S, validation, secrets, reorder by
//! drag, bulk edit and the unsaved-values sync with sends. The document model is `env_doc.rs`.

use std::path::{Path, PathBuf};

use fluxwire_format::{EnvDiagnostic, EnvRow, EnvSeverity, Environment, list_environments};
use gpui::{
    AnyElement, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    center::{button, pill},
    editor::LINE_HEIGHT,
    env_doc::{Cell, EnvDoc},
    highlight::Lang,
    tabs::{Tab, TabKind},
    text_input::{TextInput, bound_input},
    theme::Theme,
    ui::{icon, row},
    workspace::WorkspaceView,
};

/// (collection root, environment name): the identity of an environment document.
pub type DocKey = (PathBuf, String);

/// Drag payload for reordering rows.
#[derive(Clone)]
struct DragEnvRow {
    key: DocKey,
    id: u64,
    name: String,
}

struct DragGhost(String);

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        div()
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .text_size(px(12.))
            .text_color(t.text)
            .child(self.0.clone())
    }
}

/// Filter id in the cells map is never a row id.
fn filter_matches(r: &EnvRow, q: &str) -> bool {
    let q = q.to_lowercase();
    q.is_empty()
        || r.name.to_lowercase().contains(&q)
        || (!r.secret && r.value.to_lowercase().contains(&q))
}

impl WorkspaceView {
    // ---- identity ---------------------------------------------------------------------------

    /// Tab key: `env/<name>`.
    pub fn env_tab_key(&self, _root: &Path, name: &str) -> String {
        format!("env/{name}")
    }

    /// The document a tab key names (environments are workspace-level). A key from a
    /// collection-scoped build, `env/<collection>/<name>`, maps to its last segment.
    pub fn parse_env_tab_key(&self, key: &str) -> Option<DocKey> {
        let rest = key.strip_prefix("env/")?;
        let name = rest.rsplit('/').next().unwrap_or(rest);
        Some((
            self.scope.workspace.as_ref()?.root.clone(),
            name.to_string(),
        ))
    }

    pub fn is_env_tab(&self, id: u64) -> bool {
        self.scope
            .tabs
            .get(id)
            .is_some_and(|t| matches!(t.kind, TabKind::Environment))
    }

    pub fn env_doc(&self, key: &DocKey) -> Option<&EnvDoc> {
        self.scope.envs.docs.get(key)
    }

    pub fn env_doc_dirty(&self, key: &DocKey) -> bool {
        self.env_doc(key).is_some_and(EnvDoc::is_dirty)
    }

    /// Names of the open environment documents with unsaved edits (for the close confirm).
    pub fn dirty_env_docs(&self) -> Vec<DocKey> {
        let mut v: Vec<DocKey> = self
            .scope
            .envs
            .docs
            .iter()
            .filter(|(_, d)| d.is_dirty())
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    // ---- opening ----------------------------------------------------------------------------

    /// Load (or keep) the document and open its tab.
    pub fn open_env_doc(&mut self, root: &Path, name: &str, pinned: bool) {
        let key = (root.to_path_buf(), name.to_string());
        if !self.scope.envs.docs.contains_key(&key) {
            let secrets = self.secret_names(root);
            self.scope.envs.docs.insert(
                key.clone(),
                EnvDoc::load(root.to_path_buf(), name, &secrets),
            );
        }
        self.scope.envs.tracked.insert(
            key.clone(),
            fluxwire_format::local_env_is_tracked(root, name),
        );
        let tab = self.env_tab_key(root, name);
        self.open_tab(&tab, name, TabKind::Environment, pinned);
    }

    /// Open the editor of `name`.
    pub fn open_env_tab(&mut self, name: &str) {
        if let Some(root) = self.env_root() {
            self.open_env_doc(&root, name, true);
        }
    }

    /// Names and values of an environment for the panel: the open document's rows if there is
    /// one, else the files.
    pub fn env_var_views(&self, name: &str) -> Vec<crate::envs::EnvVarView> {
        let Some(root) = self.env_root() else {
            return Vec::new();
        };
        let rows = match self.scope.envs.docs.get(&(root.clone(), name.to_string())) {
            Some(d) => d.env.rows.clone(),
            None => fluxwire_format::load_environment(&root, name, &self.secret_names(&root)).rows,
        };
        rows.into_iter()
            .map(|r| crate::envs::EnvVarView {
                name: r.name,
                value: r.value,
                secret: r.secret,
            })
            .collect()
    }

    /// The active environment as edited, when it has unsaved changes: sends use it.
    pub fn dirty_env_overlay(&self) -> Option<Environment> {
        let name = self.active_env_name()?;
        let d = self.scope.envs.docs.get(&(self.env_root()?, name))?;
        d.is_dirty().then(|| d.env.clone())
    }

    // ---- editing ----------------------------------------------------------------------------

    /// After any change to a document: scopes re-resolve, the tab's dirty dot follows.
    fn env_doc_changed(&mut self, key: &DocKey, cx: &mut Context<Self>) {
        self.scope.envs.invalidate();
        let dirty = self.env_doc_dirty(key);
        let tab = self.env_tab_key(&key.0, &key.1);
        if let Some(id) = self.scope.tabs.id_for_key(&tab) {
            self.scope.tabs.set_dirty(id, dirty);
        }
        cx.notify();
    }

    fn env_cell_changed(
        &mut self,
        key: &DocKey,
        id: u64,
        cell: Cell,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.scope.envs.docs.get_mut(key) else {
            return;
        };
        let Some(i) = d.index_of(id) else { return };
        d.set_cell(i, cell, &text);
        self.env_doc_changed(key, cx);
    }

    /// The input of a row cell, created on first use with the row's text.
    pub(crate) fn env_cell(
        &self,
        key: &DocKey,
        id: u64,
        cell: Cell,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        let ck = (key.0.clone(), key.1.clone(), id, cell == Cell::Value);
        if let Some(i) = self.scope.envs.cells.borrow().get(&ck) {
            return i.clone();
        }
        let dk = key.clone();
        let input = bound_input(
            if cell == Cell::Name { "NAME" } else { "value" },
            cx,
            move |this: &mut Self, text, cx| this.env_cell_changed(&dk, id, cell, text, cx),
        );
        let init = self
            .env_doc(key)
            .and_then(|d| d.index_of(id).map(|i| &d.env.rows[i]))
            .map(|r| {
                if cell == Cell::Name {
                    r.name.clone()
                } else {
                    r.value.clone()
                }
            })
            .unwrap_or_default();
        input.update(cx, |i, cx| {
            i.set_text(&init, cx);
            i.set_consume_vertical(false);
            i.set_owner_context("EnvTable");
        });
        self.scope.envs.cells.borrow_mut().insert(ck, input.clone());
        input
    }

    /// Forget the cells of a document (rows were replaced wholesale).
    fn env_drop_cells(&self, key: &DocKey) {
        self.scope
            .envs
            .cells
            .borrow_mut()
            .retain(|(r, n, _, _), _| !(r == &key.0 && n == &key.1));
    }

    fn focus_cell(
        &self,
        key: &DocKey,
        id: u64,
        cell: Cell,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.env_cell(key, id, cell, cx);
        window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
    }

    /// Focus the Name cell of a fresh environment's first (blank, clean) row.
    pub fn env_focus_first_cell(
        &mut self,
        key: &DocKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.scope.envs.docs.get_mut(key) else {
            return;
        };
        if d.rows().is_empty() {
            d.add_blank_clean();
        }
        let Some(id) = d.ids.first().copied() else {
            return;
        };
        self.focus_cell(key, id, Cell::Name, window, cx);
    }

    pub fn env_add_row(&mut self, key: &DocKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.scope.envs.docs.get_mut(key) else {
            return;
        };
        let i = d.add_row();
        let id = d.ids[i];
        self.env_doc_changed(key, cx);
        self.focus_cell(key, id, Cell::Name, window, cx);
    }

    pub fn env_delete_row(&mut self, key: &DocKey, id: u64, cx: &mut Context<Self>) {
        if let Some(d) = self.scope.envs.docs.get_mut(key)
            && let Some(i) = d.index_of(id)
        {
            d.delete_row(i);
        }
        self.env_doc_changed(key, cx);
    }

    pub fn env_duplicate_row(
        &mut self,
        key: &DocKey,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.scope.envs.docs.get_mut(key) else {
            return;
        };
        let Some(i) = d.index_of(id).and_then(|i| d.duplicate_row(i)) else {
            return;
        };
        let new = d.ids[i];
        self.env_doc_changed(key, cx);
        self.focus_cell(key, new, Cell::Name, window, cx);
    }

    pub fn env_toggle_secret(&mut self, key: &DocKey, id: u64, cx: &mut Context<Self>) {
        let secrets = self.secret_names(&key.0);
        if let Some(d) = self.scope.envs.docs.get_mut(key)
            && let Some(i) = d.index_of(id)
        {
            d.toggle_secret(i, &secrets);
        }
        self.env_doc_changed(key, cx);
    }

    pub fn env_move_to_file(&mut self, key: &DocKey, id: u64, local: bool, cx: &mut Context<Self>) {
        if let Some(d) = self.scope.envs.docs.get_mut(key)
            && let Some(i) = d.index_of(id)
        {
            d.move_to_file(i, local);
        }
        self.env_doc_changed(key, cx);
    }

    fn env_move_row_to(
        &mut self,
        key: &DocKey,
        from_id: u64,
        onto_id: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(d) = self.scope.envs.docs.get_mut(key)
            && let (Some(a), Some(b)) = (d.index_of(from_id), d.index_of(onto_id))
        {
            d.move_row(a, b);
        }
        self.env_doc_changed(key, cx);
    }

    /// Cmd/Ctrl+Z / Shift+Z for the document when no input has focus.
    pub fn env_undo(&mut self, key: &DocKey, redo: bool, cx: &mut Context<Self>) {
        if let Some(d) = self.scope.envs.docs.get_mut(key) {
            if redo {
                d.redo();
            } else {
                d.undo();
            }
        }
        self.env_drop_cells(key);
        self.env_doc_changed(key, cx);
    }

    pub fn env_toggle_bulk(&mut self, key: &DocKey, cx: &mut Context<Self>) {
        if let Some(d) = self.scope.envs.docs.get_mut(key) {
            if d.bulk.is_some() {
                d.leave_bulk();
            } else {
                d.enter_bulk();
            }
        }
        self.env_drop_cells(key);
        self.env_doc_changed(key, cx);
    }

    pub fn env_set_active(&mut self, key: &DocKey, cx: &mut Context<Self>) {
        self.set_active_env(Some(key.1.clone()), cx);
    }

    /// Eye icon: reveal one secret for 10 s, or hide it again at once.
    pub fn env_toggle_reveal(&mut self, key: &DocKey, name: &str, cx: &mut Context<Self>) {
        let _ = key;
        if self.scope.envs.is_revealed(name) {
            self.scope.envs.reveal.remove(name);
            cx.notify();
        } else {
            self.reveal_secret(name, cx);
        }
    }

    /// Rows of an environment: the open document's, else the files'.
    pub fn env_rows_for(&self, name: &str) -> Vec<EnvRow> {
        let Some(root) = self.env_root() else {
            return Vec::new();
        };
        match self.scope.envs.docs.get(&(root.clone(), name.to_string())) {
            Some(d) => d.env.rows.clone(),
            None => fluxwire_format::load_environment(&root, name, &self.secret_names(&root)).rows,
        }
    }

    /// `NAME=value` lines of the shared (non-secret) values, for "Copy as .env".
    pub fn env_as_dotenv(&self, name: &str) -> String {
        self.env_rows_for(name)
            .into_iter()
            .filter(|r| !r.secret && !r.name.trim().is_empty())
            .map(|r| {
                format!(
                    "{}={}\n",
                    r.name.trim(),
                    fluxwire_vars::write_dotenv_value(&r.value)
                )
            })
            .collect()
    }

    /// Postman-style environment JSON. Secret values are left empty.
    pub fn env_as_postman_json(&self, name: &str) -> String {
        let values: Vec<serde_json::Value> = self
            .env_rows_for(name)
            .into_iter()
            .filter(|r| !r.name.trim().is_empty())
            .map(|r| {
                serde_json::json!({
                    "key": r.name.trim(),
                    "value": if r.secret { "" } else { r.value.as_str() },
                    "type": if r.secret { "secret" } else { "default" },
                    "enabled": true,
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "name": name,
            "values": values,
            "_postman_variable_scope": "environment",
        }))
        .unwrap_or_default()
    }

    /// Runtime variables (set by scripts and captures) with Clear, shown under the table.
    fn render_runtime_vars(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let rt = &self.scope.envs.runtime;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("Runtime variables · {}", rt.len())),
                    )
                    .child(
                        button(t, "runtime-clear", "Clear", false)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_runtime_vars(cx))),
                    ),
            )
            .when(rt.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.text_placeholder)
                        .child("None. Scripts and captures set these."),
                )
            })
            .children(rt.iter().map(|(k, (v, secret))| {
                let masked = *secret && !self.scope.envs.is_revealed(k);
                row()
                    .gap_2()
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .truncate()
                            .text_color(t.text)
                            .child(k.clone()),
                    )
                    .child(div().truncate().child(if masked {
                        "••••••••".to_string()
                    } else {
                        v.clone()
                    }))
            }))
            .into_any_element()
    }

    /// Reveal every secret of the document for 10 s.
    pub fn env_reveal_all(&mut self, key: &DocKey, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .env_doc(key)
            .map(|d| {
                d.env
                    .rows
                    .iter()
                    .filter(|r| r.secret)
                    .map(|r| r.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        for n in names {
            self.reveal_secret(&n, cx);
        }
    }

    // ---- saving, external changes -----------------------------------------------------------

    /// Write the document's files. Returns false if the document is unknown.
    pub fn save_env_doc(&mut self, key: &DocKey, cx: &mut Context<Self>) -> bool {
        let secrets = self.secret_names(&key.0);
        let Some(d) = self.scope.envs.docs.get_mut(key) else {
            return false;
        };
        let had_bulk = d.bulk.is_some();
        match d.save(&secrets) {
            Ok(0) => self.notice = Some(format!("Saved {}.", key.1)),
            Ok(n) => {
                self.notice = Some(format!(
                    "Saved {}; {n} row{} with an invalid name {} not written.",
                    key.1,
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "was" } else { "were" }
                ))
            }
            Err(e) => self.notice = Some(format!("Could not save {}: {e}", key.1)),
        }
        if had_bulk {
            self.env_drop_cells(key);
        }
        self.env_doc_changed(key, cx);
        true
    }

    /// Cmd/Ctrl+S when the active tab is an Environment: true if it handled the save.
    pub fn save_active_env(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.scope.tabs.active() else {
            return false;
        };
        if !matches!(tab.kind, TabKind::Environment) {
            return false;
        }
        let key = tab.key.clone();
        match self.parse_env_tab_key(&key) {
            Some(dk) => self.save_env_doc(&dk, cx),
            None => false,
        }
    }

    /// Save every dirty environment document (workspace close).
    pub fn save_all_env_docs(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        for key in self.dirty_env_docs() {
            let secrets = self.secret_names(&key.0);
            if let Some(d) = self.scope.envs.docs.get_mut(&key)
                && let Err(e) = d.save(&secrets)
            {
                return Err(format!("Could not save {}: {e}", key.1));
            }
            self.env_doc_changed(&key, cx);
        }
        Ok(())
    }

    /// Drop a document (tab closed, or its edits discarded).
    pub fn discard_env_doc(&mut self, key: &DocKey) {
        self.scope.envs.docs.remove(key);
        self.env_drop_cells(key);
        self.scope.envs.filters.borrow_mut().remove(key);
        self.scope.envs.invalidate();
    }

    /// Reload or flag every open document after files changed on disk.
    pub fn refresh_env_docs(&mut self) {
        let keys: Vec<DocKey> = self.scope.envs.docs.keys().cloned().collect();
        for key in keys {
            let secrets = self.secret_names(&key.0);
            let gone = !list_environments(&key.0).contains(&key.1);
            let Some(d) = self.scope.envs.docs.get_mut(&key) else {
                continue;
            };
            if gone || !d.disk_differs(&secrets) {
                continue;
            }
            if d.is_dirty() {
                d.changed_on_disk = true;
            } else {
                d.reload(&secrets);
                self.env_drop_cells(&key);
            }
        }
    }

    /// Banner buttons: take the disk version or keep the in-memory edits.
    pub fn env_resolve_conflict(&mut self, key: &DocKey, reload: bool, cx: &mut Context<Self>) {
        let secrets = self.secret_names(&key.0);
        if let Some(d) = self.scope.envs.docs.get_mut(key) {
            if reload {
                d.reload(&secrets);
            } else {
                d.changed_on_disk = false;
            }
        }
        self.env_drop_cells(key);
        self.env_doc_changed(key, cx);
    }

    /// Closing an Environment tab: confirm when it has unsaved edits.
    pub fn close_env_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let key = self
            .scope
            .tabs
            .get(tab_id)
            .and_then(|t| self.parse_env_tab_key(&t.key));
        if let Some(k) = &key
            && self.env_doc_dirty(k)
        {
            self.dialog = Some(crate::dialogs::Dialog::Confirm(crate::dialogs::Confirm {
                title: format!("Save changes to {}?", k.1),
                lines: vec![format!("{} has unsaved changes.", k.1)],
                buttons: vec![
                    (
                        "Save".into(),
                        crate::dialogs::ConfirmAction::CloseEnvTab(tab_id, true),
                    ),
                    (
                        "Discard".into(),
                        crate::dialogs::ConfirmAction::CloseEnvTab(tab_id, false),
                    ),
                    ("Cancel".into(), crate::dialogs::ConfirmAction::Cancel),
                ],
            }));
            window.focus(&self.dialog_focus);
            cx.notify();
            return;
        }
        self.finish_close_env_tab(tab_id, false, cx);
    }

    pub fn finish_close_env_tab(&mut self, tab_id: u64, save: bool, cx: &mut Context<Self>) {
        let key = self
            .scope
            .tabs
            .get(tab_id)
            .and_then(|t| self.parse_env_tab_key(&t.key));
        if let Some(k) = key {
            if save {
                self.save_env_doc(&k, cx);
            }
            self.discard_env_doc(&k);
        }
        self.scope.tabs.close(tab_id);
        self.session_dirty = true;
        cx.notify();
    }

    // ---- keyboard ---------------------------------------------------------------------------

    fn on_env_row_key(
        &mut self,
        key: &DocKey,
        id: u64,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.env_doc(key) else { return };
        let Some(i) = d.index_of(id) else { return };
        let ids = d.ids.clone();
        let name_in = self.env_cell(key, id, Cell::Name, cx);
        let in_name = name_in.read(cx).is_focused(window);
        let cell = if in_name { Cell::Name } else { Cell::Value };
        let m = &ev.keystroke.modifiers;
        match ev.keystroke.key.as_str() {
            "tab" => {
                // Name → Value → next row's Name; Shift+Tab goes back.
                let back = m.shift;
                match (in_name, back) {
                    (true, false) => self.focus_cell(key, id, Cell::Value, window, cx),
                    (false, true) => self.focus_cell(key, id, Cell::Name, window, cx),
                    (false, false) => match ids.get(i + 1) {
                        Some(next) => self.focus_cell(key, *next, Cell::Name, window, cx),
                        None => self.env_add_row(key, window, cx),
                    },
                    (true, true) => {
                        if let Some(prev) = i.checked_sub(1).and_then(|j| ids.get(j)) {
                            self.focus_cell(key, *prev, Cell::Value, window, cx)
                        }
                    }
                }
                cx.stop_propagation();
            }
            "enter" if m.secondary() => self.env_add_row(key, window, cx),
            "enter" if in_name => self.focus_cell(key, id, Cell::Value, window, cx),
            "enter" => match ids.get(i + 1) {
                Some(next) => self.focus_cell(key, *next, Cell::Name, window, cx),
                None => self.env_add_row(key, window, cx),
            },
            "escape" => {
                let reverted = self
                    .scope
                    .envs
                    .docs
                    .get_mut(key)
                    .and_then(|d| d.revert_cell(i, cell));
                if let Some(text) = reverted {
                    let input = self.env_cell(key, id, cell, cx);
                    input.update(cx, |t, cx| t.set_text(&text, cx));
                    self.env_doc_changed(key, cx);
                }
            }
            k @ ("up" | "down") => {
                let to = if k == "up" {
                    i.checked_sub(1)
                } else {
                    Some(i + 1)
                };
                if let Some(next) = to.and_then(|j| ids.get(j)) {
                    self.focus_cell(key, *next, cell, window, cx);
                }
            }
            _ => {}
        }
    }

    // ---- rendering --------------------------------------------------------------------------

    fn diag_for_row(diags: &[EnvDiagnostic], i: usize) -> Vec<&EnvDiagnostic> {
        diags.iter().filter(|d| d.row == i).collect()
    }

    pub(crate) fn render_environment(
        &self,
        t: &Theme,
        tab: &Tab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(key) = self.parse_env_tab_key(&tab.key) else {
            return div()
                .p_4()
                .text_color(t.text_muted)
                .child("This environment no longer exists.")
                .into_any_element();
        };
        let Some(doc) = self.env_doc(&key) else {
            return div()
                .p_4()
                .text_color(t.text_muted)
                .child("Reopen this environment from the Environments panel.")
                .into_any_element();
        };
        let (root, name) = key.clone();
        let secrets = self.secret_names(&root);
        let active = self.active_env_name().as_deref() == Some(name.as_str());
        let tracked = self.scope.envs.tracked.get(&key).copied().unwrap_or(false);
        let diags = doc.diagnostics(&secrets);
        let dirty = doc.is_dirty();
        let local_exists = doc.local_path().exists();
        let mut page = div()
            .id("env-page")
            .flex_1()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        // Header.
        let k_active = key.clone();
        page = page
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(name.clone()),
                    )
                    .child(div().flex_1())
                    .when(active, |d| d.child(pill(t, "active")))
                    .when(!active, |d| {
                        d.child(
                            button(t, "env-set-active", "Set as active", false).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.env_set_active(&k_active, cx)
                                }),
                            ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(format!("environments/{name}.env"))
                    .child(if local_exists {
                        format!("environments/{name}.local.env")
                    } else {
                        format!("environments/{name}.local.env (not created yet)")
                    }),
            );
        let banner = |text: String, color| {
            div()
                .p_2()
                .rounded(px(4.))
                .border_1()
                .border_color(color)
                .text_color(color)
                .text_size(px(12.))
                .child(text)
        };
        if tracked {
            page = page.child(banner(
                format!(
                    "E105 {name}.local.env is tracked by git, so its secrets may be published. Run: git rm --cached environments/{name}.local.env"
                ),
                t.deleted,
            ));
        }
        if !self.ignores_local_env_files() {
            page = page.child(banner(
                "E106 The workspace's .gitignore does not ignore *.local.env.".into(),
                t.deleted,
            ));
        }
        if doc.changed_on_disk {
            let (k1, k2) = (key.clone(), key.clone());
            page = page.child(
                row()
                    .gap_2()
                    .p_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .child(format!("environments/{name}.env changed on disk.")),
                    )
                    .child(button(t, "env-reload", "Reload", false).on_click(
                        cx.listener(move |this, _, _, cx| this.env_resolve_conflict(&k1, true, cx)),
                    ))
                    .child(
                        button(t, "env-keep", "Keep mine", false).on_click(cx.listener(
                            move |this, _, _, cx| this.env_resolve_conflict(&k2, false, cx),
                        )),
                    ),
            );
        }
        // Toolbar.
        let filter = {
            let mut f = self.scope.envs.filters.borrow_mut();
            f.entry(key.clone())
                .or_insert_with(|| {
                    let i = cx.new(|cx| TextInput::new("Filter variables", cx));
                    cx.subscribe(&i, |_, _, _: &crate::text_input::TextInputEvent, cx| {
                        cx.notify()
                    })
                    .detach();
                    i
                })
                .clone()
        };
        let q = filter.read(cx).text().to_string();
        let (ka, kb, kc, kd) = (key.clone(), key.clone(), key.clone(), key.clone());
        let bulk_on = doc.bulk.is_some();
        page = page.child(
            row()
                .gap_2()
                .child(
                    div()
                        .w(px(220.))
                        .h(px(26.))
                        .px_2()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .child(filter),
                )
                .child(div().flex_1())
                .child(
                    button(t, "env-add-var", "Add variable", false)
                        .on_click(cx.listener(move |this, _, w, cx| this.env_add_row(&ka, w, cx))),
                )
                .child(
                    button(
                        t,
                        "env-bulk",
                        if bulk_on { "Table" } else { "Bulk edit" },
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.env_toggle_bulk(&kb, cx))),
                )
                .child(
                    button(t, "env-reveal", "Reveal secrets", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.env_reveal_all(&kc, cx))),
                )
                .child(button(t, "env-save", "Save", dirty).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.save_env_doc(&kd, cx);
                    },
                ))),
        );
        if bulk_on {
            page = page.child(self.render_env_bulk(t, &key, doc, cx));
        } else {
            page = page.child(self.render_env_table(t, &key, doc, &diags, &secrets, &q, cx));
        }
        page.child(self.render_runtime_vars(t, cx))
            .child(self.render_cookies_section(t, &name, cx))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_env_table(
        &self,
        t: &Theme,
        key: &DocKey,
        doc: &EnvDoc,
        diags: &[EnvDiagnostic],
        secrets: &[String],
        q: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let head = |label: &'static str, w: Option<f32>| {
            let d = div()
                .text_size(px(11.))
                .text_color(t.text_placeholder)
                .child(label);
            match w {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1(),
            }
        };
        let rows: Vec<(usize, u64, EnvRow)> = doc
            .env
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| filter_matches(r, q))
            .map(|(i, r)| (i, doc.ids[i], r.clone()))
            .collect();
        // A name defined in both files: the .env row is overridden by .local.env (SPEC §8.2).
        let overridden = |r: &EnvRow| {
            !r.local
                && doc
                    .env
                    .rows
                    .iter()
                    .any(|o| o.local && o.name == r.name && !r.name.is_empty())
        };
        let table = div()
            .flex()
            .flex_col()
            .child(
                row()
                    .h(px(24.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(head("", Some(14.)))
                    .child(head("Name", Some(200.)))
                    .child(head("Value", None))
                    .child(head("Secret", Some(48.)))
                    .child(head("Source", Some(72.)))
                    .child(head("", Some(40.))),
            )
            .children(rows.into_iter().map(|(i, id, r)| {
                let dk = key.clone();
                let name_in = self.env_cell(key, id, Cell::Name, cx);
                let value_in = self.env_cell(key, id, Cell::Value, cx);
                let revealed = self.scope.envs.is_revealed(&r.name);
                let mask = r.secret && !revealed;
                let row_diags = Self::diag_for_row(diags, i);
                let bad = row_diags
                    .iter()
                    .any(|d| matches!(d.severity, EnvSeverity::Error));
                name_in.update(cx, |n, cx| {
                    if n.is_invalid() != bad {
                        n.set_invalid(bad.then_some("invalid"), cx);
                    }
                });
                value_in.update(cx, |v, cx| {
                    if v.is_masked() != mask {
                        v.set_masked(mask, cx);
                    }
                    let multi = r.value.contains('\n');
                    v.set_read_only(multi, cx);
                });
                let can_toggle = !secrets.contains(&r.name);
                let k7 = dk.clone();
                let (k1, k2, k3, k4, k5, k6) = (
                    dk.clone(),
                    dk.clone(),
                    dk.clone(),
                    dk.clone(),
                    dk.clone(),
                    dk.clone(),
                );
                let local = r.local;
                let drag = DragEnvRow {
                    key: dk.clone(),
                    id,
                    name: r.name.clone(),
                };
                let input_box = |inp: Entity<TextInput>| {
                    div()
                        .min_w_0()
                        .h(px(24.))
                        .px_1()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(t.border_variant)
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .child(div().flex_1().min_w_0().child(inp))
                };
                div()
                    .id(("env-row", id))
                    .flex()
                    .flex_col()
                    .on_key_down(
                        cx.listener(move |this, ev, w, cx| this.on_env_row_key(&k1, id, ev, w, cx)),
                    )
                    .drag_over::<DragEnvRow>({
                        let h = t.element_selected;
                        move |s, _, _, _| s.bg(h)
                    })
                    .on_drop(cx.listener(move |this, d: &DragEnvRow, _, cx| {
                        if d.key == k2 {
                            this.env_move_row_to(&k2, d.id, id, cx)
                        }
                    }))
                    .child(
                        row()
                            .h(px(30.))
                            .px_3()
                            .gap_3()
                            .border_b_1()
                            .border_color(t.border_variant)
                            .when(overridden(&r), |d| d.opacity(0.55))
                            .child(
                                div()
                                    .id(("env-drag", id))
                                    .w(px(14.))
                                    .flex_none()
                                    .cursor_grab()
                                    .on_drag(drag, |d, _, _, cx| {
                                        cx.new(|_| DragGhost(d.name.clone()))
                                    })
                                    .child(icon("ellipsis-vertical", 12., t.text_placeholder)),
                            )
                            .child(input_box(name_in).w(px(200.)).flex_none())
                            .child(input_box(value_in).flex_1().when(r.secret, |d| {
                                let (kv, nm) = (k7.clone(), r.name.clone());
                                d.flex().items_center().child(
                                    div()
                                        .id(("env-eye", id))
                                        .flex_none()
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.env_toggle_reveal(&kv, &nm, cx)
                                        }))
                                        .child(icon(
                                            "eye",
                                            14.,
                                            if revealed {
                                                t.accent
                                            } else {
                                                t.text_placeholder
                                            },
                                        )),
                                )
                            }))
                            .child(
                                div()
                                    .id(("env-secret", id))
                                    .w(px(48.))
                                    .flex_none()
                                    .when(can_toggle, |d| d.cursor_pointer())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.env_toggle_secret(&k3, id, cx)
                                    }))
                                    .child(
                                        div()
                                            .size(px(14.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(3.))
                                            .border_1()
                                            .border_color(if r.secret {
                                                t.accent
                                            } else {
                                                t.border
                                            })
                                            .when(r.secret, |d| d.bg(t.accent))
                                            .when(!can_toggle, |d| d.opacity(0.5))
                                            .when(r.secret, |d| {
                                                d.child(icon("check", 10., t.editor))
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id(("env-source", id))
                                    .w(px(72.))
                                    .flex_none()
                                    .cursor_pointer()
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.env_move_to_file(&k4, id, !local, cx)
                                    }))
                                    .child(if local { ".local.env" } else { ".env" }),
                            )
                            .child(
                                row()
                                    .w(px(40.))
                                    .flex_none()
                                    .gap_1()
                                    .child(
                                        div()
                                            .id(("env-dup", id))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                this.env_duplicate_row(&k5, id, w, cx)
                                            }))
                                            .child(icon("copy", 12., t.text_placeholder)),
                                    )
                                    .child(
                                        div()
                                            .id(("env-del", id))
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.env_delete_row(&k6, id, cx)
                                            }))
                                            .child(icon("x", 12., t.text_placeholder)),
                                    ),
                            ),
                    )
                    .children(row_diags.into_iter().map(|d| {
                        div()
                            .px_3()
                            .text_size(px(11.))
                            .text_color(match d.severity {
                                EnvSeverity::Error => t.deleted,
                                _ => t.text_muted,
                            })
                            .child(format!("{} {}", d.code, d.message))
                    }))
            }))
            .child({
                let k = key.clone();
                row()
                    .id("env-ghost")
                    .h(px(28.))
                    .px_3()
                    .gap_1()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .on_click(cx.listener(move |this, _, w, cx| this.env_add_row(&k, w, cx)))
                    .child(icon("plus", 12., t.text_muted))
                    .child("Add variable")
            });
        table.into_any_element()
    }

    /// Bulk edit: `.env` and `.local.env` as text in the code editor (keyboard only).
    fn render_env_bulk(
        &self,
        t: &Theme,
        key: &DocKey,
        doc: &EnvDoc,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(b) = doc.bulk.as_ref() else {
            return div().into_any_element();
        };
        let pane = |which: usize, label: &'static str, text: &String, cx: &mut Context<Self>| {
            let ed = self.env_bulk_editor(key, which, text, cx);
            let probs = EnvDoc::bulk_problems(text);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(label),
                )
                .child(
                    div()
                        .id(("env-bulk", which))
                        .h(px(LINE_HEIGHT * 10.0))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.editor)
                        .child(ed),
                )
                .children(probs.into_iter().map(|(line, msg)| {
                    div()
                        .text_size(px(11.))
                        .text_color(t.deleted)
                        .child(format!("line {line}: {msg}"))
                }))
        };
        let (public, local) = (b.public.clone(), b.local.clone());
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(pane(0, ".env", &public, cx))
            .child(pane(1, ".local.env", &local, cx))
            .into_any_element()
    }

    /// The bulk-edit editor of one pane, created on first use. Its text mirrors the
    /// document's bulk buffer; edits write back and refresh the unsaved-values sync.
    fn env_bulk_editor(
        &self,
        key: &DocKey,
        which: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<crate::code_editor::CodeEditor> {
        use crate::code_editor::{CodeEditor, CodeEditorEvent};
        let slot = (key.clone(), which);
        let existing = self.scope.env_bulk_editors.borrow().get(&slot).cloned();
        if let Some(ed) = existing {
            ed.update(cx, |e, cx| {
                if e.text() != text {
                    e.set_text(text, cx);
                }
            });
            return ed;
        }
        let ed =
            gpui::AppContext::new(cx, |cx| CodeEditor::new(Lang::Plain, cx).line_numbers(true));
        let settings = self.scope.envs.settings.editor.clone();
        ed.update(cx, |e, cx| {
            e.apply_settings(&settings, cx);
            e.set_text(text, cx)
        });
        let k = key.clone();
        cx.subscribe(&ed, move |this, ed, ev: &CodeEditorEvent, cx| {
            if *ev != CodeEditorEvent::Changed {
                return;
            }
            let t = ed.read(cx).text().to_string();
            if let Some(b) = this
                .scope
                .envs
                .docs
                .get_mut(&k)
                .and_then(|d| d.bulk.as_mut())
            {
                if which == 0 {
                    b.public = t;
                } else {
                    b.local = t;
                }
                this.env_doc_changed(&k, cx);
            }
        })
        .detach();
        self.scope
            .env_bulk_editors
            .borrow_mut()
            .insert(slot, ed.clone());
        ed
    }
}
