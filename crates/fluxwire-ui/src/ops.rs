//! Workspace operations: opening folders, documents, edits, saving, file operations, watching
//! and session persistence. Rendering lives in `center.rs` and `dock.rs`.

use std::path::{Path, PathBuf};

use fluxwire_format as fmt;
use fluxwire_model::{BlockKind, RequestKind, TreeNode};
use gpui::{Context, KeyDownEvent, PathPromptOptions, Window};

use crate::{
    editor::EditorState,
    envs::{CompleteKey, CompleteTarget},
    project::{
        OpenDoc, Problem, Project, Session, Target, remember_workspace, save_recent, save_session,
    },
    tabs::TabKind,
    workspace::{
        CellEdit, Col, METHODS, MenuKind, ModalKind, PromptPurpose, TabUi, Table, Workspace,
    },
};

fn tab_kind(doc: &OpenDoc) -> TabKind {
    if doc.request_kind() == RequestKind::WebSocket {
        TabKind::WebSocket
    } else {
        TabKind::Http {
            method: doc.method(),
        }
    }
}

impl Workspace {
    // ---- labels ---------------------------------------------------------------------------

    pub fn workspace_label(&self) -> String {
        self.project
            .as_ref()
            .map_or_else(|| "Fluxwire".to_string(), |p| p.name.clone())
    }

    pub fn branch_label(&self) -> Option<String> {
        self.project.as_ref().and_then(|p| p.branch.clone())
    }

    pub fn prompt_title(&self) -> Option<String> {
        match self.prompt.as_ref()? {
            PromptPurpose::Rename { .. } => Some("Rename to".to_string()),
            PromptPurpose::NewEnvironment => Some("New environment name".to_string()),
            PromptPurpose::RenameEnvironment { .. } => Some("Rename environment to".to_string()),
        }
    }

    /// Name of the folder that Run/Docs act on (selected folder, else collection name).
    pub fn selected_folder_name(&self) -> String {
        let key = self.selected.clone().unwrap_or_default();
        let last = key.rsplit('/').next().unwrap_or("");
        if last.is_empty() || last.ends_with(".md") {
            self.project
                .as_ref()
                .map_or("workspace".into(), |p| p.name.clone())
        } else {
            last.to_string()
        }
    }

    // ---- opening --------------------------------------------------------------------------

    /// Open a folder as the workspace, replacing the current one.
    pub fn open_project(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.save_session_now();
        match Project::open(path) {
            Ok(p) => {
                self.open_error = None;
                self.tabs = Default::default();
                self.tab_ui.clear();
                self.collapsed.clear();
                self.selected = None;
                self.cell = None;
                self.watcher = None;
                self.project = Some(p);
                self.history_project_changed(cx);
                let remembered = self
                    .state_dir
                    .as_ref()
                    .and_then(|d| crate::project::load_session(d, path).active_env);
                self.envs.runtime.clear();
                self.envs.states.clear();
                self.envs.warned.clear();
                self.envs.edit = None;
                self.init_active_env(remembered);
                if let Some(dir) = &self.state_dir {
                    remember_workspace(&mut self.recent, path);
                    save_recent(dir, &self.recent);
                }
                if self.watch_enabled {
                    self.start_watching(path, cx);
                }
                self.restore_session(window, cx);
            }
            Err(e) => {
                self.open_error = Some(e.to_string());
                self.project = None;
            }
        }
        cx.notify();
    }

    /// Turn the opened folder into a collection (writes `fluxwire.toml` and friends).
    pub fn create_collection_here(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        let name = root
            .file_name()
            .map_or("Collection".into(), |n| n.to_string_lossy().into_owned());
        match fmt::create_collection(&root, &name) {
            Ok(()) => self.open_project(&root, window, cx),
            Err(e) => {
                self.notice = Some(format!("Could not create collection: {e}"));
                cx.notify();
            }
        }
    }

    pub fn close_project(&mut self) {
        self.save_session_now();
        self.project = None;
        self.watcher = None;
        self.history.ws_key = None;
        self.history.entries.clear();
        self.history.rebuild_rows_pub();
        self.tabs = Default::default();
        self.tab_ui.clear();
        self.selected = None;
    }

    pub fn open_folder_dialog(
        &mut self,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if create {
                    let name = path
                        .file_name()
                        .map_or("Collection".into(), |n| n.to_string_lossy().into_owned());
                    if let Err(e) = fmt::create_collection(&path, &name) {
                        this.notice = Some(format!("Could not create collection: {e}"));
                        cx.notify();
                        return;
                    }
                }
                this.open_project(&path, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn start_watching(&mut self, root: &Path, cx: &mut Context<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(handle) = fmt::watch(root, tx) else {
            return;
        };
        self.watcher = Some(handle);
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
                if this
                    .update(cx, |this, cx| this.on_fs_changes(&batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// External file changes (from the watcher, or called directly in tests).
    pub fn on_fs_changes(&mut self, batch: &[PathBuf], cx: &mut Context<Self>) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let changed = p.handle_fs(batch);
        self.on_env_fs_change(batch);
        for key in changed {
            self.sync_tab(&key);
        }
        self.reconcile_tabs();
        cx.notify();
    }

    /// Keep tab title, kind and dirty dot in line with the document.
    pub fn sync_tab(&mut self, key: &str) {
        let Some(doc) = self.project.as_ref().and_then(|p| p.doc(key)) else {
            return;
        };
        let (title, kind, dirty) = (doc.title(), tab_kind(doc), doc.dirty());
        self.tabs.update(key, &title, kind);
        if let Some(id) = self.tabs.id_for_key(key) {
            self.tabs.set_dirty(id, dirty);
        }
    }

    /// After structural changes: load missing documents for open tabs, close tabs whose file is gone.
    pub fn reconcile_tabs(&mut self) {
        let keys: Vec<String> = self
            .tabs
            .all()
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Http { .. } | TabKind::WebSocket))
            .map(|t| t.key.clone())
            .collect();
        for key in keys {
            let Some(p) = self.project.as_mut() else {
                return;
            };
            if p.doc(&key).is_none() && p.open_doc(&key).is_err() {
                self.tabs.close_key(&key);
            }
            self.sync_tab(&key);
        }
    }

    pub fn open_request(
        &mut self,
        key: &str,
        permanent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.project.as_mut() else {
            return;
        };
        let doc = match p.open_doc(key) {
            Ok(d) => d,
            Err(e) => {
                self.notice = Some(format!("Could not open {key}: {e}"));
                cx.notify();
                return;
            }
        };
        let (title, kind) = (doc.title(), tab_kind(doc));
        let dirty = doc.dirty();
        let id = if permanent {
            self.tabs.open_permanent(key, &title, kind)
        } else {
            self.tabs.open_preview(key, &title, kind)
        };
        self.tabs.set_dirty(id, dirty);
        if !dirty && !permanent {
            // set_dirty(false) never changes preview-ness.
        }
        let method = self
            .project
            .as_ref()
            .and_then(|p| p.doc(key))
            .and_then(|d| METHODS.iter().position(|m| *m == d.method()))
            .unwrap_or(0);
        self.tab_ui.entry(id).or_insert_with(|| TabUi {
            method,
            ..TabUi::new()
        });
        self.selected = Some(key.to_string());
        self.cell = None;
        self.target = Target::Body;
        window.focus(&self.root_focus);
        self.session_dirty = true;
        cx.notify();
    }

    // ---- editing --------------------------------------------------------------------------

    pub fn after_edit_pub(&mut self, key: &str, cx: &mut Context<Self>) {
        self.after_edit(key, cx)
    }

    fn after_edit(&mut self, key: &str, cx: &mut Context<Self>) {
        self.sync_tab(key);
        if let Some(id) = self.tabs.id_for_key(key)
            && self
                .project
                .as_ref()
                .and_then(|p| p.doc(key))
                .is_some_and(|d| d.dirty())
        {
            self.tabs.set_dirty(id, true);
        }
        self.session_dirty = true;
        cx.notify();
    }

    /// Character index under a pointer position in the URL bar (monospace cells).
    fn url_index(&self, window: &Window, pos: gpui::Point<gpui::Pixels>) -> usize {
        let ts = window.text_system();
        let id = ts.resolve_font(&gpui::font(crate::assets::BUFFER_FONT));
        let cw = ts
            .advance(id, gpui::px(14.), 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(8.4)
            .max(1.0);
        let x = f32::from(pos.x - self.url_bounds.get().origin.x);
        let len = self
            .active_key()
            .and_then(|k| {
                self.project
                    .as_ref()?
                    .doc(&k)?
                    .parsed
                    .request()
                    .map(|r| r.target.chars().count())
            })
            .unwrap_or(0);
        ((x / cw).round().max(0.0) as usize).min(len)
    }

    pub(crate) fn url_mouse_down(
        &mut self,
        ev: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.url_focus);
        let idx = self.url_index(window, ev.position);
        let text = self
            .active_key()
            .and_then(|k| {
                self.project
                    .as_ref()?
                    .doc(&k)?
                    .parsed
                    .request()
                    .map(|r| r.target.clone())
            })
            .unwrap_or_default();
        let len = text.chars().count();
        self.url_sel = match ev.click_count {
            2 => Some(crate::ui::word_range(&text, idx)),
            n if n >= 3 => Some((0, len)),
            _ => Some((idx, idx)),
        };
        self.url_drag = ev.click_count == 1;
        cx.notify();
    }

    pub(crate) fn url_mouse_move(
        &mut self,
        ev: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.url_drag || ev.pressed_button != Some(gpui::MouseButton::Left) {
            return;
        }
        let idx = self.url_index(window, ev.position);
        if let Some((a, _)) = self.url_sel {
            self.url_sel = Some((a, idx));
            cx.notify();
        }
    }

    /// Typing in the URL bar edits the request target through the span editor. A selection is
    /// replaced by typing or pasting and removed by Backspace; Cmd/Ctrl+A/C/X work on it.
    pub(crate) fn on_url_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        if self.envs.completion.is_some() {
            let cur = self
                .project
                .as_ref()
                .and_then(|p| p.doc(&key))
                .and_then(|d| d.parsed.request().map(|r| r.target.clone()))
                .unwrap_or_default();
            match self.completion_key(ev, CompleteTarget::Url, &cur) {
                CompleteKey::Consumed => {
                    cx.notify();
                    return;
                }
                CompleteKey::Accept(new) => {
                    self.set_url_text(&key, &new, cx);
                    return;
                }
                CompleteKey::Pass => {}
            }
        }
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let Some(req) = doc.parsed.request() else {
            return;
        };
        let (method, target) = (req.method.clone(), req.target.clone());
        let mut chars: Vec<char> = target.chars().collect();
        let sel = self
            .url_sel
            .map(|(a, b)| (a.min(b).min(chars.len()), a.max(b).min(chars.len())))
            .filter(|(a, b)| a != b);
        let ks = &ev.keystroke;
        // Replace the selection (or append at the end) with `ins`.
        let insert = |chars: &mut Vec<char>, ins: &str| {
            let ins = ins
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control());
            match sel {
                Some((a, b)) => {
                    chars.splice(a..b, ins);
                }
                None => chars.extend(ins),
            }
        };
        if ks.modifiers.secondary() && matches!(ks.key.as_str(), "a" | "c" | "x") {
            match ks.key.as_str() {
                "a" => {
                    self.url_sel = Some((0, chars.len()));
                    cx.notify();
                    return;
                }
                k => {
                    let Some((a, b)) = sel else { return };
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        chars[a..b].iter().collect(),
                    ));
                    if k == "c" {
                        return;
                    }
                    chars.drain(a..b);
                }
            }
        } else if ks.modifiers.secondary() && ks.key == "v" {
            // Paste: a `curl ...` command fills the whole request (SPEC §9.5); anything else
            // replaces the selection or is appended to the URL.
            let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
                return;
            };
            if text.trim_start().starts_with("curl")
                && let Some(edits) = fluxwire_runner::runner().edits_from_paste(&doc.parsed, &text)
            {
                if doc.apply(&edits) {
                    self.url_sel = None;
                    self.after_edit(&key, cx);
                    self.notice = Some("Filled the request from a cURL command.".into());
                }
                return;
            }
            insert(&mut chars, &text);
        } else if ks.modifiers.control || ks.modifiers.platform {
            return;
        } else if ks.key == "backspace" || ks.key == "delete" {
            match sel {
                Some((a, b)) => {
                    chars.drain(a..b);
                }
                None => {
                    chars.pop();
                }
            }
        } else if let Some(s) = ks.key_char.as_deref().filter(|s| !s.is_empty()) {
            insert(&mut chars, s);
        } else {
            return;
        }
        self.url_sel = None;
        let target: String = chars.into_iter().collect();
        let edits = fmt::set_request_line(&doc.parsed, &method, &target);
        if doc.apply(&edits) {
            self.after_edit(&key, cx);
            self.refresh_completion(CompleteTarget::Url, &target);
        }
    }

    /// Replace the whole request target (used to accept a `{{` completion).
    pub(crate) fn set_url_text(&mut self, key: &str, target: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let Some(method) = doc.parsed.request().map(|r| r.method.clone()) else {
            return;
        };
        let edits = fmt::set_request_line(&doc.parsed, &method, target);
        if doc.apply(&edits) {
            self.url_sel = None;
            self.after_edit(key, cx);
        }
    }

    pub fn set_method(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let Some(req) = doc.parsed.request() else {
            return;
        };
        let target = req.target.clone();
        let edits = fmt::set_request_line(&doc.parsed, METHODS[idx], &target);
        if doc.apply(&edits) {
            if let Some(ui) = self.active_ui() {
                ui.method = idx;
            }
            self.after_edit(&key, cx);
        }
    }

    pub fn toggle_text_mode(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        if let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) {
            doc.text_mode = !doc.text_mode;
            self.target = if doc.text_mode {
                Target::Text
            } else {
                Target::Body
            };
        }
        cx.notify();
    }

    /// Route a key press to the focused editor surface of the active document.
    pub fn on_editor_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let target = self.target;
        let clip = cx.read_from_clipboard().and_then(|c| c.text());
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let mut text = doc.target_text(target);
        let mut st = doc.editors.remove(&target).unwrap_or_default();
        let out = st.handle_key(&mut text, ev, clip.as_deref());
        let line = crate::editor::line_col(&text, st.cursor).0;
        doc.editors.insert(target, st);
        match out {
            crate::editor::KeyOutcome::Edited => {
                if text != doc.target_text(target) {
                    doc.set_target_text(target, &text);
                }
                self.after_edit(&key, cx);
            }
            crate::editor::KeyOutcome::Moved => cx.notify(),
            crate::editor::KeyOutcome::Ignored => return,
        }
        self.scroll_to_line(line);
    }

    pub fn scroll_to_line(&mut self, line: usize) {
        let h = self.editor_scroll.bounds().size.height;
        if f32::from(h) <= 0.0 {
            return;
        }
        let y = gpui::px(line as f32 * crate::editor::LINE_HEIGHT + 8.0);
        let off = self.editor_scroll.offset();
        let top = -off.y;
        if y < top {
            self.editor_scroll.set_offset(gpui::point(off.x, -y));
        } else if y + gpui::px(crate::editor::LINE_HEIGHT) > top + h {
            self.editor_scroll.set_offset(gpui::point(
                off.x,
                -(y + gpui::px(crate::editor::LINE_HEIGHT) - h),
            ));
        }
    }

    pub fn click_editor(&mut self, pos: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let target = self.target;
        let bounds = self.editor_bounds.get();
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let text = doc.target_text(target);
        let off = EditorState::offset_for_click(&text, bounds, pos);
        doc.editors.entry(target).or_default().cursor = off;
        cx.notify();
    }

    /// Add `kind` block (e.g. a pre script) through the span editor and focus it.
    pub fn add_block(&mut self, kind: BlockKind, info: &str, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let edits = fmt::insert_block(&doc.parsed, kind.clone(), info, "");
        if doc.apply(&edits) {
            if let Some(i) = doc.find_block(kind, None) {
                self.target = Target::Block(i);
            }
            self.after_edit(&key, cx);
        }
    }

    // ---- key/value tables -----------------------------------------------------------------

    pub fn table_rows(&self, key: &str, table: Table) -> Vec<(bool, String, String)> {
        let Some(req) = self
            .project
            .as_ref()
            .and_then(|p| p.doc(key))
            .and_then(|d| d.parsed.request())
        else {
            return vec![];
        };
        match table {
            Table::Params => req
                .query
                .iter()
                .map(|q| {
                    (
                        q.enabled,
                        q.name.clone(),
                        q.value.clone().unwrap_or_default(),
                    )
                })
                .collect(),
            Table::Headers => req
                .headers
                .iter()
                .map(|h| (h.enabled, h.name.clone(), h.value.clone()))
                .collect(),
        }
    }

    pub fn start_cell(
        &mut self,
        table: Table,
        row: usize,
        col: Col,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.active_key() else { return };
        let rows = self.table_rows(&key, table);
        let text = rows
            .get(row)
            .map(|r| {
                if col == Col::Key {
                    r.1.clone()
                } else {
                    r.2.clone()
                }
            })
            .unwrap_or_default();
        self.cell = Some(CellEdit {
            key,
            table,
            row,
            col,
            text,
        });
        window.focus(&self.cell_focus);
        cx.notify();
    }

    fn apply_row(
        &mut self,
        key: &str,
        table: Table,
        row: usize,
        enabled: bool,
        name: &str,
        value: &str,
    ) -> bool {
        let len = self.table_rows_len(key, table);
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(key)) else {
            return false;
        };
        let edits = match (table, row < len) {
            (Table::Headers, true) => fmt::set_header(&doc.parsed, row, enabled, name, value),
            (Table::Headers, false) => fmt::add_header(&doc.parsed, name, value),
            (Table::Params, true) => fmt::set_query(
                &doc.parsed,
                row,
                enabled,
                name,
                (!value.is_empty()).then_some(value),
            ),
            (Table::Params, false) => {
                fmt::add_query(&doc.parsed, name, (!value.is_empty()).then_some(value))
            }
        };
        doc.apply(&edits)
    }

    fn table_rows_len(&self, key: &str, table: Table) -> usize {
        self.table_rows(key, table).len()
    }

    pub fn on_cell_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut cell) = self.cell.clone() else {
            return;
        };
        let ks = &ev.keystroke;
        let mut accepted = false;
        if self.envs.completion.is_some() {
            match self.completion_key(ev, CompleteTarget::Cell, &cell.text) {
                CompleteKey::Consumed => {
                    cx.notify();
                    return;
                }
                CompleteKey::Accept(new) => {
                    cell.text = new;
                    accepted = true;
                }
                CompleteKey::Pass => {}
            }
        }
        match ks.key.as_str() {
            _ if accepted => {}
            "enter" | "escape" => {
                self.cell = None;
                self.envs.completion = None;
                window.focus(&self.root_focus);
                cx.notify();
                return;
            }
            "tab" => {
                self.envs.completion = None;
                if cell.col == Col::Key {
                    self.start_cell(cell.table, cell.row, Col::Value, window, cx);
                } else {
                    self.cell = None;
                    window.focus(&self.root_focus);
                }
                cx.notify();
                return;
            }
            "backspace" => {
                cell.text.pop();
            }
            _ => {
                if ks.modifiers.control || ks.modifiers.platform {
                    return;
                }
                match ks.key_char.as_deref() {
                    Some(s) if !s.is_empty() => {
                        cell.text.extend(s.chars().filter(|c| !c.is_control()))
                    }
                    _ => return,
                }
            }
        }
        self.commit_cell(cell, cx);
    }

    /// Apply the edited cell text to the document (after typing or accepting a completion).
    pub(crate) fn apply_cell_text(&mut self, cx: &mut Context<Self>) {
        if let Some(cell) = self.cell.clone() {
            self.commit_cell(cell, cx);
        }
    }

    fn commit_cell(&mut self, mut cell: CellEdit, cx: &mut Context<Self>) {
        let rows = self.table_rows(&cell.key, cell.table);
        let (enabled, mut name, mut value) =
            rows.get(cell.row)
                .cloned()
                .unwrap_or((true, String::new(), String::new()));
        if cell.col == Col::Key {
            name = cell.text.clone();
        } else {
            value = cell.text.clone();
        }
        let is_new = cell.row >= rows.len();
        // A new row only reaches the document once it has a name.
        if !(is_new && name.trim().is_empty()) {
            let name = name.trim().replace(char::is_whitespace, "");
            if self.apply_row(&cell.key, cell.table, cell.row, enabled, &name, &value) {
                if is_new {
                    cell.row = self.table_rows_len(&cell.key, cell.table).saturating_sub(1);
                }
                let key = cell.key.clone();
                let text = cell.text.clone();
                self.cell = Some(cell);
                self.after_edit(&key, cx);
                self.refresh_completion(CompleteTarget::Cell, &text);
                return;
            }
        }
        let text = cell.text.clone();
        self.cell = Some(cell);
        self.refresh_completion(CompleteTarget::Cell, &text);
        cx.notify();
    }

    pub fn toggle_row(&mut self, table: Table, row: usize, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let rows = self.table_rows(&key, table);
        if let Some((en, n, v)) = rows.get(row)
            && self.apply_row(&key, table, row, !en, n, v)
        {
            self.after_edit(&key, cx);
        }
    }

    pub fn delete_row(&mut self, table: Table, row: usize, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let edits = match table {
            Table::Headers => fmt::remove_header(&doc.parsed, row),
            Table::Params => fmt::remove_query(&doc.parsed, row),
        };
        if doc.apply(&edits) {
            self.cell = None;
            self.after_edit(&key, cx);
        }
    }

    // ---- saving, conflicts ----------------------------------------------------------------

    pub fn save_active(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        match doc.save() {
            Ok(()) => {
                self.notice = None;
                self.sync_tab(&key);
                if let Some(id) = self.tabs.id_for_key(&key) {
                    self.tabs.set_dirty(id, false);
                }
            }
            Err(e) => self.notice = Some(format!("Could not save: {e}")),
        }
        self.session_dirty = true;
        cx.notify();
    }

    /// Conflict bar: take the disk version (`reload`) or keep the in-memory edits.
    pub fn resolve_conflict(&mut self, reload: bool, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        if let Some(disk) = doc.conflict.take() {
            if reload {
                doc.reload_from(disk);
            } else {
                doc.saved = disk;
            }
        } else if doc.missing && !reload {
            doc.missing = false;
        }
        self.after_edit(&key, cx);
    }

    // ---- problems -------------------------------------------------------------------------

    pub fn goto_problem(&mut self, p: &Problem, window: &mut Window, cx: &mut Context<Self>) {
        if p.key == "keymap.toml" {
            self.open_config_file("keymap.toml", "", cx);
            return;
        }
        self.open_request(&p.key.clone(), true, window, cx);
        let Some(doc) = self.project.as_mut().and_then(|pr| pr.doc_mut(&p.key)) else {
            return;
        };
        doc.text_mode = true;
        let off = p.span.start.min(doc.text.len());
        doc.editors.entry(Target::Text).or_default().cursor = off;
        self.target = Target::Text;
        let line = crate::editor::line_col(&doc.text, off).0;
        window.focus(&self.root_focus);
        self.editor_scroll.set_offset(gpui::point(
            gpui::px(0.),
            -gpui::px((line.saturating_sub(3)) as f32 * crate::editor::LINE_HEIGHT),
        ));
        cx.notify();
    }

    // ---- file operations ------------------------------------------------------------------

    /// Directory new items go into: the selected folder, the folder of the selected request, or
    /// the first collection root.
    fn target_dir(&self) -> Option<PathBuf> {
        let p = self.project.as_ref()?;
        if let Some(sel) = &self.selected {
            let abs = p.abs_of(sel);
            if abs.is_dir() {
                return Some(abs);
            }
            if let Some(parent) = abs.parent()
                && parent.is_dir()
            {
                return Some(parent.to_path_buf());
            }
        }
        p.collections.first().map(|c| c.root.clone())
    }

    /// Begin creating a request or folder: an empty, focused row appears in the tree.
    pub fn start_new(
        &mut self,
        websocket: bool,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self.target_dir() else {
            self.notice = Some("Open or create a collection first.".into());
            cx.notify();
            return;
        };
        let parent = self.key_for(&dir).unwrap_or_default();
        self.collapsed.remove(&parent);
        self.left_open = true;
        self.panel = crate::workspace::Panel::Project;
        self.inline = Some(crate::workspace::InlineNew {
            parent,
            dir,
            websocket,
            folder,
            text: crate::ui::TextEdit::default(),
        });
        window.focus(&self.inline_focus);
        cx.notify();
    }

    pub fn cancel_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inline.take().is_some() {
            window.focus(&self.root_focus);
            cx.notify();
        }
    }

    pub fn on_inline_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(inline) = self.inline.as_mut() else {
            return;
        };
        match ev.keystroke.key.as_str() {
            "escape" => self.cancel_inline(window, cx),
            "enter" => self.confirm_inline(window, cx),
            _ => {
                inline.text.handle_key(ev);
                cx.notify();
            }
        }
    }

    pub fn confirm_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inline) = self.inline.take() else {
            return;
        };
        window.focus(&self.root_focus);
        let name = inline.text.text().trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        let result = if inline.folder {
            fmt::create_folder(&inline.dir, &name).map(|p| (p, false))
        } else {
            fmt::create_request(&inline.dir, &name, "", inline.websocket).map(|p| (p, true))
        };
        match result {
            Ok((path, is_request)) => {
                if let Some(p) = self.project.as_mut() {
                    p.rescan();
                }
                let key = self.key_for(&path).unwrap_or_default();
                if is_request {
                    self.open_request(&key, true, window, cx);
                } else {
                    self.selected = Some(key);
                }
                self.notice = None;
            }
            Err(e) => self.notice = Some(format!("Could not create {name}: {e}")),
        }
        cx.notify();
    }

    pub fn start_new_in(
        &mut self,
        key: &str,
        websocket: bool,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(key.to_string());
        self.start_new(websocket, folder, window, cx);
    }

    pub(crate) fn open_modal_keep_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.prompt.take();
        self.open_modal(ModalKind::Prompt, window, cx);
        self.prompt = prompt;
    }

    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        self.prompt = Some(PromptPurpose::Rename { key: key.clone() });
        self.open_modal_keep_prompt(window, cx);
        // Pre-fill with the current display name.
        let current = self.display_name(&key);
        if let Some(m) = self.modal.as_mut() {
            m.query.insert(&current);
        }
    }

    fn display_name(&self, key: &str) -> String {
        fn find(nodes: &[TreeNode], key: &str) -> Option<String> {
            for n in nodes {
                if n.path() == key {
                    return Some(n.name().to_string());
                }
                if let TreeNode::Folder { children, .. } = n
                    && let Some(x) = find(children, key)
                {
                    return Some(x);
                }
            }
            None
        }
        self.project
            .as_ref()
            .and_then(|p| p.collections.iter().find_map(|c| find(&c.tree, key)))
            .unwrap_or_else(|| key.rsplit('/').next().unwrap_or(key).to_string())
    }

    pub fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .modal
            .as_ref()
            .map(|m| m.query.text().trim().to_string())
            .unwrap_or_default();
        let purpose = self.prompt.clone();
        self.close_modal(window, cx);
        let Some(purpose) = purpose else { return };
        if name.is_empty() {
            return;
        }
        let result: std::io::Result<Option<String>> = match purpose {
            PromptPurpose::Rename { key } => self.do_rename(&key, &name),
            p @ (PromptPurpose::NewEnvironment | PromptPurpose::RenameEnvironment { .. }) => {
                self.finish_env_prompt(p, &name);
                cx.notify();
                return;
            }
        };
        match result {
            Ok(open) => {
                if let Some(p) = self.project.as_mut() {
                    p.rescan();
                }
                self.reconcile_tabs();
                if let Some(key) = open {
                    self.open_request(&key, true, window, cx);
                }
                self.notice = None;
            }
            Err(e) => self.notice = Some(format!("Operation failed: {e}")),
        }
        cx.notify();
    }

    fn key_for(&self, abs: &Path) -> Option<String> {
        self.project.as_ref().map(|p| p.key_of(abs))
    }

    /// Unsaved documents at or under `key`; file operations refuse to run over them.
    fn dirty_under(&self, key: &str) -> Vec<String> {
        let Some(p) = &self.project else {
            return vec![];
        };
        p.dirty_keys()
            .into_iter()
            .filter(|k| k == key || k.starts_with(&format!("{key}/")))
            .collect()
    }

    fn do_rename(&mut self, key: &str, name: &str) -> std::io::Result<Option<String>> {
        if !self.dirty_under(key).is_empty() {
            return Err(std::io::Error::other(
                "save or revert unsaved changes before renaming",
            ));
        }
        let abs = self
            .project
            .as_ref()
            .map(|p| p.abs_of(key))
            .ok_or_else(|| std::io::Error::other("no workspace"))?;
        let new_abs = fmt::rename(&abs, name)?;
        let new_key = self.key_for(&new_abs).unwrap_or_default();
        self.after_move(key, &new_key);
        Ok(None)
    }

    /// A file or folder moved on disk: re-point tabs and drop stale in-memory documents.
    fn after_move(&mut self, old: &str, new: &str) {
        self.tabs.rekey(old, new);
        if let Some(p) = self.project.as_mut() {
            let stale: Vec<String> = p
                .docs
                .keys()
                .filter(|k| *k == old || k.starts_with(&format!("{old}/")))
                .cloned()
                .collect();
            for k in stale {
                p.close_doc(&k);
            }
        }
        if self.selected.as_deref() == Some(old) {
            self.selected = Some(new.to_string());
        }
        self.session_dirty = true;
    }

    pub fn duplicate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        let Some(abs) = self.project.as_ref().map(|p| p.abs_of(&key)) else {
            return;
        };
        match fmt::duplicate(&abs) {
            Ok(new_abs) => {
                if let Some(p) = self.project.as_mut() {
                    p.rescan();
                }
                let nk = self.key_for(&new_abs).unwrap_or_default();
                if new_abs.is_file() {
                    self.open_request(&nk, true, window, cx);
                } else {
                    self.selected = Some(nk);
                }
            }
            Err(e) => self.notice = Some(format!("Duplicate failed: {e}")),
        }
        cx.notify();
    }

    pub fn delete_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        let Some(abs) = self.project.as_ref().map(|p| p.abs_of(&key)) else {
            return;
        };
        match fmt::delete(&abs) {
            Ok(()) => {
                self.tabs.close_key(&key);
                if let Some(p) = self.project.as_mut() {
                    let stale: Vec<String> = p
                        .docs
                        .keys()
                        .filter(|k| **k == key || k.starts_with(&format!("{key}/")))
                        .cloned()
                        .collect();
                    for k in stale {
                        p.close_doc(&k);
                    }
                    p.rescan();
                }
                self.selected = None;
                self.notice = Some("Moved to trash.".into());
                self.session_dirty = true;
            }
            Err(e) => self.notice = Some(format!("Delete failed: {e}")),
        }
        cx.notify();
    }

    pub fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        if let Some(abs) = self.project.as_ref().map(|p| p.abs_of(&key)) {
            cx.reveal_path(&abs);
        }
    }

    pub fn move_item(&mut self, key: &str, dest_folder_key: &str, cx: &mut Context<Self>) {
        let Some(p) = self.project.as_ref() else {
            return;
        };
        let (abs, dest) = (p.abs_of(key), p.abs_of(dest_folder_key));
        if !self.dirty_under(key).is_empty() {
            self.notice = Some("Save or revert unsaved changes before moving.".into());
            cx.notify();
            return;
        }
        match fmt::move_to(&abs, &dest) {
            Ok(new_abs) => {
                let new_key = self.key_for(&new_abs).unwrap_or_default();
                self.after_move(key, &new_key);
                if let Some(p) = self.project.as_mut() {
                    p.rescan();
                }
                self.reconcile_tabs();
            }
            Err(e) => self.notice = Some(format!("Move failed: {e}")),
        }
        cx.notify();
    }

    pub fn open_tree_menu(
        &mut self,
        key: &str,
        kind: MenuKind,
        pos: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(key.to_string());
        self.tree_menu = Some(crate::workspace::TreeMenu {
            key: key.to_string(),
            kind,
            pos,
        });
        cx.notify();
    }

    // ---- session --------------------------------------------------------------------------

    pub fn session(&self) -> Session {
        let mut s = Session::default();
        for t in self.tabs.all() {
            s.tabs.push(self.session_tab(t));
        }
        s.closed = self
            .tabs
            .closed_tabs()
            .iter()
            .map(|t| self.session_tab(t))
            .collect();
        s.active = self.tabs.active().map(|t| t.key.clone());
        s.layout = self.layout();
        s.active_env = self.envs.active.clone();
        if let Some(p) = &self.project {
            for k in p.dirty_keys() {
                if let Some(d) = p.doc(&k) {
                    s.unsaved.insert(k, d.text.clone());
                }
            }
        }
        s
    }

    pub fn save_session_now(&mut self) {
        self.session_dirty = false;
        let (Some(dir), Some(p)) = (self.state_dir.clone(), self.project.as_ref()) else {
            return;
        };
        save_session(&dir, &p.root, &self.session());
    }

    pub fn start_session_timer(&mut self, cx: &mut Context<Self>) {
        if self.state_dir.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let alive = this.update(cx, |this, _| {
                    if this.session_dirty {
                        this.save_session_now();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(dir), Some(root)) = (
            self.state_dir.clone(),
            self.project.as_ref().map(|p| p.root.clone()),
        ) else {
            return;
        };
        let s = crate::project::load_session(&dir, &root);
        self.apply_layout(&s.layout);
        for t in &s.tabs {
            if !t.kind.is_empty() {
                if self.restore_other_tab(t, cx)
                    && let Some(id) = self.tabs.id_for_key(&t.key)
                {
                    if t.pinned {
                        self.tabs.toggle_pin(id);
                    }
                    if let Some(ui) = self.tab_ui.get_mut(&id) {
                        ui.scroll = t.scroll;
                    }
                }
                continue;
            }
            self.open_request(&t.key, !t.preview, window, cx);
            if let Some(text) = s.unsaved.get(&t.key)
                && let Some(d) = self.project.as_mut().and_then(|p| p.doc_mut(&t.key))
                && d.text != *text
            {
                d.set_text(text.clone());
            }
            if t.pinned
                && let Some(id) = self.tabs.id_for_key(&t.key)
            {
                self.tabs.toggle_pin(id);
            }
            if let Some(id) = self.tabs.id_for_key(&t.key)
                && let Some(ui) = self.tab_ui.get_mut(&id)
            {
                ui.scroll = t.scroll;
            }
            self.sync_tab(&t.key);
        }
        self.tabs.set_closed(
            s.closed
                .iter()
                .map(|c| {
                    let kind =
                        crate::navigation::kind_from_tag_pub(&c.kind).unwrap_or(TabKind::Http {
                            method: "GET".into(),
                        });
                    (c.key.clone(), c.title.clone(), kind, c.pinned)
                })
                .collect(),
        );
        if let Some(a) = &s.active
            && let Some(id) = self.tabs.id_for_key(a)
        {
            self.tabs.activate(id);
        }
    }
}
