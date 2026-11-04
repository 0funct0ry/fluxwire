//! Workspace operations: opening folders, documents, edits, saving, file operations, watching
//! and session persistence. Rendering lives in `center.rs` and `dock.rs`.

use std::path::{Path, PathBuf};

use fluxwire_format as fmt;
use fluxwire_model::{BlockKind, RequestKind, TreeNode};
use gpui::{Context, KeyDownEvent, PathPromptOptions, Window};

use crate::{
    envs::{CompleteKey, CompleteTarget},
    highlight::Lang,
    tabs::TabKind,
    workspace::{
        CellEdit, Col, METHODS, MenuKind, ModalKind, PromptPurpose, TabUi, Table, WorkspaceView,
    },
    workspace_model::{
        OpenDoc, Problem, Session, Target, Workspace, remember_workspace, save_recent, save_session,
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

impl WorkspaceView {
    // ---- labels ---------------------------------------------------------------------------

    pub fn workspace_label(&self) -> String {
        self.scope
            .workspace
            .as_ref()
            .map_or_else(|| "Fluxwire".to_string(), |p| p.name.clone())
    }

    pub fn branch_label(&self) -> Option<String> {
        self.scope.workspace.as_ref().and_then(|p| p.branch.clone())
    }

    pub fn prompt_title(&self) -> Option<String> {
        match self.prompt.as_ref()? {
            PromptPurpose::Rename { .. } => Some("Rename to".to_string()),
            PromptPurpose::RenameCollection { .. } => Some("Rename collection to".to_string()),
            PromptPurpose::RenameEnvironment { .. } => Some("Rename environment to".to_string()),
            PromptPurpose::GoToLine { .. } => Some("Go to line (line or line:column)".to_string()),
        }
    }

    /// Name of the folder that Run/Docs act on (selected folder, else collection name).
    pub fn selected_folder_name(&self) -> String {
        let key = self.scope.selected.clone().unwrap_or_default();
        let last = key.rsplit('/').next().unwrap_or("");
        if last.is_empty() || last.ends_with(".md") {
            self.scope
                .workspace
                .as_ref()
                .map_or("workspace".into(), |p| p.name.clone())
        } else {
            last.to_string()
        }
    }

    // ---- opening --------------------------------------------------------------------------

    /// Open a folder as the workspace, replacing the current one.
    pub fn open_workspace(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.save_session_now();
        match Workspace::open(path) {
            Ok(p) => {
                self.open_error = None;
                self.replace_scope();
                self.scope.workspace = Some(p);
                self.history_workspace_changed(cx);
                let saved = self
                    .state_dir
                    .as_ref()
                    .map(|d| crate::workspace_model::load_session(d, path))
                    .unwrap_or_default();
                self.init_active_env(saved.active_env.as_deref(), saved.no_environment);
                if let Some(dir) = &self.state_dir {
                    remember_workspace(&mut self.recent, path);
                    save_recent(dir, &self.recent);
                }
                if self.watch_enabled {
                    self.start_watching(path, cx);
                    self.regenerate_fw_types(false, cx);
                }
                self.restore_session(window, cx);
            }
            Err(e) => {
                self.open_error = Some(e.to_string());
                self.replace_scope();
            }
        }
        cx.notify();
    }

    /// Drop the current `OpenWorkspace` (cancelling its runs and sessions) and start from a
    /// fresh one; only the user's environment settings carry over.
    fn replace_scope(&mut self) {
        let settings = std::mem::take(&mut self.scope.envs.settings);
        self.scope = Default::default();
        self.scope.envs.settings = settings;
    }

    /// Close the open Workspace: save its session, shut it down and drop it. Closing means the
    /// next launch shows the welcome view.
    pub fn close_workspace(&mut self) {
        self.save_session_now();
        self.replace_scope();
        self.history.ws_key = None;
        self.history.entries.clear();
        self.history.rebuild_rows_pub();
        self.recent.last = None;
        if let Some(dir) = &self.state_dir {
            save_recent(dir, &self.recent);
        }
    }

    pub fn open_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                this.request_close(crate::dialogs::AfterClose::Open(path), window, cx)
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
        self.scope.watcher = Some(handle);
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
        let Some(p) = self.scope.workspace.as_mut() else {
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
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return;
        };
        let (title, kind, dirty) = (doc.title(), tab_kind(doc), doc.dirty());
        self.scope.tabs.update(key, &title, kind);
        if let Some(id) = self.scope.tabs.id_for_key(key) {
            self.scope.tabs.set_dirty(id, dirty);
        }
    }

    /// After structural changes: load missing documents for open tabs, close tabs whose file is gone.
    pub fn reconcile_tabs(&mut self) {
        let keys: Vec<String> = self
            .scope
            .tabs
            .all()
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Http { .. } | TabKind::WebSocket))
            .map(|t| t.key.clone())
            .collect();
        for key in keys {
            let Some(p) = self.scope.workspace.as_mut() else {
                return;
            };
            if p.doc(&key).is_none() && p.open_doc(&key).is_err() {
                self.scope.tabs.close_key(&key);
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
        let Some(p) = self.scope.workspace.as_mut() else {
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
            self.scope.tabs.open_permanent(key, &title, kind)
        } else {
            self.scope.tabs.open_preview(key, &title, kind)
        };
        self.scope.tabs.set_dirty(id, dirty);
        if !dirty && !permanent {
            // set_dirty(false) never changes preview-ness.
        }
        let method = self
            .scope
            .workspace
            .as_ref()
            .and_then(|p| p.doc(key))
            .and_then(|d| METHODS.iter().position(|m| *m == d.method()))
            .unwrap_or(0);
        self.scope.tab_ui.entry(id).or_insert_with(|| TabUi {
            method,
            ..TabUi::new()
        });
        self.scope.selected = Some(key.to_string());
        self.scope.cell = None;
        self.scope.target = Target::Body;
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
        if let Some(id) = self.scope.tabs.id_for_key(key)
            && self
                .scope
                .workspace
                .as_ref()
                .and_then(|p| p.doc(key))
                .is_some_and(|d| d.dirty())
        {
            self.scope.tabs.set_dirty(id, true);
        }
        self.session_dirty = true;
        cx.notify();
    }

    /// The URL bar input of request `key`, created on first use. Typing edits the request
    /// target through the span editor; a pasted `curl` command fills the whole request.
    pub(crate) fn url_input(
        &self,
        key: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<crate::text_input::TextInput> {
        use crate::text_input::{TextInput, TextInputEvent};
        if let Some(i) = self.scope.url_inputs.borrow().get(key) {
            return i.clone();
        }
        let input = gpui::AppContext::new(cx, |cx| {
            let mut i = TextInput::new("", cx);
            i.set_strip_whitespace(true);
            i.set_paste_intercept(|t| t.trim_start().starts_with("curl"));
            // Up/Down go to the `{{` completion popup while it is open (set when drawn).
            i.set_owner_context("RequestEditor");
            i
        });
        let k = key.to_string();
        cx.subscribe(
            &input,
            move |this, input, ev: &TextInputEvent, cx| match ev {
                TextInputEvent::Changed => {
                    let text = input.read(cx).text().to_string();
                    this.url_changed(&k, &text, cx);
                }
                TextInputEvent::PasteIntercepted(text) => this.url_curl_paste(&k, text, cx),
                _ => {}
            },
        )
        .detach();
        self.scope
            .url_inputs
            .borrow_mut()
            .insert(key.to_string(), input.clone());
        input
    }

    pub(crate) fn focus_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let input = self.url_input(&key, cx);
        window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
    }

    /// The URL input's text changed: write it to the request line.
    fn url_changed(&mut self, key: &str, text: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let Some(req) = doc.parsed.request() else {
            return;
        };
        if req.target == text {
            return;
        }
        let edits = fmt::set_request_line(&doc.parsed, &req.method.clone(), text);
        if doc.apply(&edits) {
            self.after_edit(key, cx);
            self.refresh_completion(CompleteTarget::Url, text);
        }
    }

    /// A pasted `curl ...` command fills the whole request (SPEC §9.5).
    fn url_curl_paste(&mut self, key: &str, text: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        if let Some(edits) = fluxwire_runner::runner().edits_from_paste(&doc.parsed, text)
            && doc.apply(&edits)
        {
            self.after_edit(key, cx);
            self.notice = Some("Filled the request from a cURL command.".into());
            cx.notify();
        }
    }

    /// Key presses that reach the URL bar's owner: the `{{` completion popup's keys.
    pub(crate) fn on_url_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        if self.scope.envs.completion.is_none() {
            return;
        }
        let cur = self
            .scope
            .workspace
            .as_ref()
            .and_then(|p| p.doc(&key))
            .and_then(|d| d.parsed.request().map(|r| r.target.clone()))
            .unwrap_or_default();
        match self.completion_key(ev, CompleteTarget::Url, &cur) {
            CompleteKey::Consumed => cx.notify(),
            CompleteKey::Accept(new) => self.set_url_text(&key, &new, cx),
            CompleteKey::Pass => {}
        }
    }

    /// Replace the whole request target (used to accept a `{{` completion).
    pub(crate) fn set_url_text(&mut self, key: &str, target: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let Some(method) = doc.parsed.request().map(|r| r.method.clone()) else {
            return;
        };
        let edits = fmt::set_request_line(&doc.parsed, &method, target);
        if doc.apply(&edits) {
            self.after_edit(key, cx);
        }
    }

    pub fn set_method(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
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
        if let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) {
            doc.text_mode = !doc.text_mode;
            self.scope.target = if doc.text_mode {
                Target::Text
            } else {
                Target::Body
            };
        }
        cx.notify();
    }

    /// The code editor of `target` in request `key`, created on first use and kept in sync
    /// with the document: a change made elsewhere (form view, another editor) is pushed in
    /// here, and an edit made here goes through the span editor. A trailing newline the
    /// span editor normalises away does not count as a difference.
    pub(crate) fn code_editor(
        &self,
        key: &str,
        target: Target,
        lang: Lang,
        text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<crate::code_editor::CodeEditor> {
        use crate::code_editor::{CodeEditor, CodeEditorEvent};
        let slot = (key.to_string(), target);
        let existing = self.scope.code_editors.borrow().get(&slot).cloned();
        if let Some(ed) = existing {
            let ro = self.docs_ranges(key, target);
            let provider = self.completion_provider_for(key, target);
            let settings = self.scope.envs.settings.editor.clone();
            let check = self.var_check_for(key, target);
            let chips = self.docs_chips(key, target);
            ed.update(cx, |e, cx| {
                e.set_completion_provider(Some(provider));
                e.set_var_check(check);
                e.set_chips(chips);
                if e.lang() != lang {
                    e.set_language(lang);
                    e.apply_settings(&settings, cx);
                }
                let cur = e.text();
                if cur != text && cur.trim_end_matches(['\n', '\r']) != text {
                    e.set_text(text, cx);
                }
                if let Some(r) = ro {
                    e.set_read_only_ranges(r);
                }
            });
            return ed;
        }
        let ed = gpui::AppContext::new(cx, |cx| CodeEditor::new(lang, cx));
        let ro = self.docs_ranges(key, target);
        let provider = self.completion_provider_for(key, target);
        let settings = self.scope.envs.settings.editor.clone();
        let check = self.var_check_for(key, target);
        let chips = self.docs_chips(key, target);
        ed.update(cx, |e, cx| {
            e.apply_settings(&settings, cx);
            e.set_var_check(check);
            e.set_chips(chips);
            e.set_completion_provider(Some(provider));
            e.set_text(text, cx);
            if let Some(r) = ro {
                e.set_read_only_ranges(r);
            }
            if target == Target::Docs {
                e.set_placeholder("Describe this request in Markdown…");
            }
        });
        let k = key.to_string();
        cx.subscribe(&ed, move |this, ed, ev: &CodeEditorEvent, cx| match ev {
            CodeEditorEvent::Changed => {
                let text = ed.read(cx).text().to_string();
                this.code_editor_changed(&k, target, &text, cx);
            }
            CodeEditorEvent::ChipClicked(tag) => this.open_chip_target(*tag, cx),
            CodeEditorEvent::Focused => {
                if this.scope.target != target {
                    this.scope.target = target;
                    cx.notify();
                }
            }
            _ => {}
        })
        .detach();
        self.scope
            .code_editors
            .borrow_mut()
            .insert(slot, ed.clone());
        ed
    }

    /// The completion providers of a surface (M9.06): variables first (they only answer
    /// inside `{{`), then the surface's own, then buffer words.
    fn completion_provider_for(
        &self,
        key: &str,
        target: Target,
    ) -> std::rc::Rc<dyn crate::completion::CompletionProvider> {
        use crate::completion::{CompletionProvider, Composite};
        use crate::completion_providers::*;
        use std::rc::Rc;
        let vars: Vec<(String, String)> = self
            .scope_for(key)
            .map(|scope| {
                self.var_names(&scope)
                    .into_iter()
                    .map(|n| {
                        let src = scope
                            .var(&n)
                            .map_or_else(|| "dynamic".to_string(), |v| v.source.describe());
                        (n, src)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let names: Vec<String> = vars.iter().map(|(n, _)| n.clone()).collect();
        let mut parts: Vec<Rc<dyn CompletionProvider>> = vec![Rc::new(VariablesProvider { vars })];
        let doc = self.scope.workspace.as_ref().and_then(|p| p.doc(key));
        let role = |i: usize| {
            doc.and_then(|d| d.parsed.blocks.get(i))
                .and_then(|b| match b.kind {
                    BlockKind::Pre => Some(fluxwire_script::Role::Pre),
                    BlockKind::Post => Some(fluxwire_script::Role::Post),
                    BlockKind::Test => Some(fluxwire_script::Role::Test),
                    _ => None,
                })
        };
        match target {
            Target::Block(i) => {
                if let Some(role) = role(i) {
                    parts.push(Rc::new(ScriptProvider {
                        role,
                        variables: names,
                    }));
                }
            }
            Target::Text => parts.push(Rc::new(FluxMdProvider { variables: names })),
            Target::Docs => {
                let links = self
                    .scope
                    .workspace
                    .as_ref()
                    .map(|p| p.all_requests().into_iter().map(|r| r.0).collect())
                    .unwrap_or_default();
                parts.push(Rc::new(MarkdownProvider { links }));
            }
            Target::Body => {}
        }
        parts.push(Rc::new(WordsProvider));
        Rc::new(Composite(parts))
    }

    /// `{{name}}` resolution for the Body and "Open as text" editors (colour and underline).
    fn var_check_for(&self, key: &str, target: Target) -> Option<crate::code_editor::VarCheck> {
        if !matches!(target, Target::Body | Target::Text) {
            return None;
        }
        let scope = self.scope_for(key)?;
        Some(std::rc::Rc::new(move |name: &str| {
            fluxwire_vars::interpolate(&format!("{{{{{name}}}}}"), &scope)
                .missing
                .is_empty()
        }))
    }

    /// Labels of the protected blocks of the Docs editor, each at its block's offset.
    pub(crate) fn docs_chips(&self, key: &str, target: Target) -> Vec<crate::code_editor::Chip> {
        if target != Target::Docs {
            return vec![];
        }
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return vec![];
        };
        let base = crate::docs_edit::base_of(&doc.parsed);
        doc.parsed
            .blocks
            .iter()
            .filter(|b| {
                !matches!(b.kind, BlockKind::Prose | BlockKind::Example) && b.span.start >= base
            })
            .map(|b| {
                let info = b.info_span.slice(&doc.text);
                crate::code_editor::Chip {
                    offset: b.span.start - base,
                    label: crate::docs_edit::chip_for(info).into(),
                    tag: crate::docs_edit::chip_tab(&b.kind),
                }
            })
            .collect()
    }

    /// Read-only ranges of the Docs editor (everything but prose); `None` for other surfaces.
    fn docs_ranges(&self, key: &str, target: Target) -> Option<Vec<std::ops::Range<usize>>> {
        if target != Target::Docs {
            return None;
        }
        let doc = self.scope.workspace.as_ref()?.doc(key)?;
        Some(crate::docs_edit::protected_ranges(&doc.parsed))
    }

    /// A Docs chip was clicked: show the sub-tab that owns the block (or the raw file).
    pub(crate) fn open_chip_target(&mut self, tag: usize, cx: &mut Context<Self>) {
        if tag == crate::docs_edit::OPEN_AS_TEXT {
            if let Some(doc) = self
                .active_key()
                .and_then(|k| self.scope.workspace.as_ref().and_then(|p| p.doc(&k)))
                && !doc.text_mode
            {
                self.toggle_text_mode(cx);
            }
        } else {
            self.show_request_tab(tag);
        }
        cx.notify();
    }

    /// An editor's text changed: write it to the document through the span editor.
    fn code_editor_changed(
        &mut self,
        key: &str,
        target: Target,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let refused = target == Target::Docs
            && text != doc.target_text(target)
            && crate::docs_edit::apply(&doc.parsed, &doc.target_text(target), text).is_err();
        if text != doc.target_text(target) {
            doc.set_target_text(target, text);
        }
        if refused {
            self.notice =
                Some("That would create a request block; use the Add block buttons.".into());
        }
        self.after_edit(key, cx);
    }

    /// Moves the caret of an editor (creating it from the document's text when needed) and
    /// focuses it.
    pub(crate) fn focus_code_editor(
        &mut self,
        key: &str,
        target: Target,
        offset: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return;
        };
        let text = doc.target_text(target);
        let lang = if target == Target::Text {
            Lang::FluxMd
        } else {
            Lang::Plain
        };
        let ed = self.code_editor(key, target, lang, &text, cx);
        ed.update(cx, |e, cx| {
            if let Some(o) = offset {
                e.set_cursor(o, cx);
            }
            e.focus(window);
        });
    }

    /// Add `kind` block (e.g. a pre script) through the span editor and focus it. New scripts
    /// use the collection's `script_language` default (SPEC §6.1).
    pub fn add_block(&mut self, kind: BlockKind, info: &str, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let default_js = self
            .collection_root_of(&key)
            .and_then(|r| fmt::load_config(&r).ok())
            .is_some_and(|c| c.defaults.script_language.as_deref() == Some("js"));
        let info = match info.strip_prefix("ts ") {
            Some(rest) if default_js => format!("js {rest}"),
            _ => info.to_string(),
        };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let edits = fmt::insert_block(&doc.parsed, kind.clone(), &info, "");
        if doc.apply(&edits) {
            if let Some(i) = doc.find_block(kind, None) {
                self.scope.target = Target::Block(i);
            }
            self.after_edit(&key, cx);
        }
    }

    /// Switch the first `kind` block between `ts` and `js` by rewriting only its info string.
    pub fn toggle_block_language(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let Some(i) = doc.find_block(kind, None) else {
            return;
        };
        let b = &doc.parsed.blocks[i];
        let to = if b.info.language == "ts" { "js" } else { "ts" };
        let info = b.info_span.slice(&doc.parsed.source).to_string();
        let new = match info.trim_start().strip_prefix(b.info.language.as_str()) {
            Some(rest) => format!("{to}{rest}"),
            None => return,
        };
        let edit = fluxwire_model::Edit::replace(b.info_span, new);
        if doc.apply(&[edit]) {
            self.after_edit(&key, cx);
        }
    }

    /// Insert an `http expect` block, pre-filled from the last response when there is one.
    pub fn add_expect(&mut self, from_response: bool, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let mut content = "HTTP 2xx".to_string();
        if from_response
            && let Some(x) = self
                .scope
                .tabs
                .active_id()
                .and_then(|id| self.scope.responses.get(&id))
                .and_then(|r| r.result.as_ref())
                .and_then(|r| r.outcome.as_ref())
                .and_then(|o| o.exchange.as_ref())
        {
            content = format!("HTTP {}", x.status);
            if let Some(ct) = x.header("content-type") {
                content.push_str(&format!("\nContent-Type: {ct}"));
            }
        }
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        if doc.find_block(BlockKind::Expect, None).is_some() {
            return;
        }
        let edits = fmt::insert_block(&doc.parsed, BlockKind::Expect, "http expect", &content);
        if doc.apply(&edits) {
            if let Some(i) = doc.find_block(BlockKind::Expect, None) {
                self.scope.target = Target::Block(i);
            }
            self.after_edit(&key, cx);
        }
    }

    /// Replace the status line of the `http expect` block (`201`, `2xx`, `*`).
    pub fn set_expect_status(&mut self, status: &str, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let Some(i) = doc.find_block(BlockKind::Expect, None) else {
            return;
        };
        let content = doc.parsed.blocks[i].content;
        let text = content.slice(&doc.parsed.source);
        let first = text.find('\n').unwrap_or(text.len());
        let line = text[..first].trim_end_matches('\r');
        let new = match line.trim().strip_prefix("HTTP/") {
            Some(rest) => {
                let ver = rest.split_whitespace().next().unwrap_or("1.1");
                format!("HTTP/{ver} {status}")
            }
            None => format!("HTTP {status}"),
        };
        let span = fluxwire_model::Span::new(content.start, content.start + line.len());
        if doc.apply(&[fluxwire_model::Edit::replace(span, new)]) {
            self.after_edit(&key, cx);
        }
    }

    // ---- key/value tables -----------------------------------------------------------------

    pub fn table_rows(&self, key: &str, table: Table) -> Vec<(bool, String, String)> {
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return vec![];
        };
        let Some(req) = doc.parsed.request() else {
            return vec![];
        };
        match table {
            // URL parameters first, then continuation lines (`fmt::query_rows`).
            Table::Params => fmt::query_rows(&doc.parsed)
                .into_iter()
                .map(|q| (q.enabled, q.name, q.value.unwrap_or_default()))
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
        let input = crate::text_input::bound_input("", cx, |this: &mut Self, text, cx| {
            if let Some(c) = this.scope.cell.as_mut() {
                c.text = text;
            }
            this.apply_cell_text(cx);
        });
        input.update(cx, |i, cx| i.set_text(&text, cx));
        window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
        self.scope.cell = Some(CellEdit {
            key,
            table,
            row,
            col,
            text,
            input,
        });
        cx.notify();
    }

    pub(crate) fn apply_row(
        &mut self,
        key: &str,
        table: Table,
        row: usize,
        enabled: bool,
        name: &str,
        value: &str,
    ) -> bool {
        let len = self.table_rows_len(key, table);
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
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
        let Some(mut cell) = self.scope.cell.clone() else {
            return;
        };
        let ks = &ev.keystroke;
        let mut accepted = false;
        if self.scope.envs.completion.is_some() {
            match self.completion_key(ev, CompleteTarget::Cell, &cell.text) {
                CompleteKey::Consumed => {
                    cx.notify();
                    return;
                }
                CompleteKey::Accept(new) => {
                    cell.input.update(cx, |i, cx| i.set_text(&new, cx));
                    cell.text = new;
                    accepted = true;
                }
                CompleteKey::Pass => {}
            }
        }
        match ks.key.as_str() {
            _ if accepted => {}
            "enter" | "escape" => {
                self.scope.cell = None;
                self.scope.envs.completion = None;
                window.focus(&self.root_focus);
                cx.notify();
                return;
            }
            "tab" => {
                self.scope.envs.completion = None;
                if cell.col == Col::Key {
                    self.start_cell(cell.table, cell.row, Col::Value, window, cx);
                } else {
                    self.scope.cell = None;
                    window.focus(&self.root_focus);
                }
                cx.notify();
                return;
            }
            _ => return,
        }
        self.commit_cell(cell, cx);
    }

    /// Apply the edited cell text to the document (after typing or accepting a completion).
    pub(crate) fn apply_cell_text(&mut self, cx: &mut Context<Self>) {
        if let Some(cell) = self.scope.cell.clone() {
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
                self.scope.cell = Some(cell);
                self.after_edit(&key, cx);
                self.refresh_completion(CompleteTarget::Cell, &text);
                return;
            }
        }
        let text = cell.text.clone();
        self.scope.cell = Some(cell);
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
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let edits = match table {
            Table::Headers => fmt::remove_header(&doc.parsed, row),
            Table::Params => fmt::remove_query(&doc.parsed, row),
        };
        if doc.apply(&edits) {
            self.scope.cell = None;
            self.after_edit(&key, cx);
        }
    }

    // ---- saving, conflicts ----------------------------------------------------------------

    pub fn save_active(&mut self, cx: &mut Context<Self>) {
        if self.save_active_env(cx) {
            return;
        }
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        match doc.save() {
            Ok(()) => {
                self.notice = None;
                self.sync_tab(&key);
                if let Some(id) = self.scope.tabs.id_for_key(&key) {
                    self.scope.tabs.set_dirty(id, false);
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
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
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
        if p.key.starts_with("env/") {
            if let Some((root, name)) = self.parse_env_tab_key(&p.key) {
                self.open_env_doc(&root, &name, true);
                cx.notify();
            }
            return;
        }
        self.open_request(&p.key.clone(), true, window, cx);
        let Some(doc) = self
            .scope
            .workspace
            .as_mut()
            .and_then(|pr| pr.doc_mut(&p.key))
        else {
            return;
        };
        doc.text_mode = true;
        let off = p.span.start.min(doc.text.len());
        self.scope.target = Target::Text;
        self.focus_code_editor(&p.key, Target::Text, Some(off), window, cx);
        cx.notify();
    }

    // ---- file operations ------------------------------------------------------------------

    /// Directory new items go into: the selected folder, the folder of the selected request, or
    /// the first collection root.
    fn target_dir(&self) -> Option<PathBuf> {
        let p = self.scope.workspace.as_ref()?;
        if let Some(sel) = &self.scope.selected {
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
        self.scope.collapsed.remove(&parent);
        self.scope.left_open = true;
        self.scope.panel = crate::workspace::Panel::Workspace;
        let placeholder = if folder {
            "Folder name"
        } else if websocket {
            "WebSocket request name"
        } else {
            "Request name"
        };
        let text =
            gpui::AppContext::new(cx, |cx| crate::text_input::TextInput::new(placeholder, cx));
        window.focus(&gpui::Focusable::focus_handle(text.read(cx), cx));
        self.scope.inline = Some(crate::workspace::InlineNew {
            parent,
            dir,
            websocket,
            folder,
            text,
        });
        cx.notify();
    }

    pub fn cancel_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope.inline.take().is_some() {
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
        if self.scope.inline.is_none() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "escape" => self.cancel_inline(window, cx),
            "enter" => self.confirm_inline(window, cx),
            _ => {}
        }
    }

    pub fn confirm_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inline) = self.scope.inline.take() else {
            return;
        };
        window.focus(&self.root_focus);
        let name = inline.text.read(cx).text().trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        let result = if inline.folder {
            fmt::create_folder(&inline.dir, &name).map(|p| (p, false))
        } else {
            let url = self.default_url_for(&inline.dir, inline.websocket);
            fmt::create_request_with_url(&inline.dir, &name, "", inline.websocket, &url)
                .map(|p| (p, true))
        };
        match result {
            Ok((path, is_request)) => {
                if let Some(p) = self.scope.workspace.as_mut() {
                    p.rescan();
                }
                let key = self.key_for(&path).unwrap_or_default();
                if is_request {
                    self.open_request(&key, true, window, cx);
                } else {
                    self.scope.selected = Some(key);
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
        self.scope.selected = Some(key.to_string());
        self.start_new(websocket, folder, window, cx);
    }

    pub(crate) fn open_modal_keep_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.prompt.take();
        self.open_modal(ModalKind::Prompt, window, cx);
        self.prompt = prompt;
    }

    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.scope.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        self.prompt = Some(PromptPurpose::Rename { key: key.clone() });
        self.open_modal_keep_prompt(window, cx);
        // Pre-fill with the current display name.
        let current = self.display_name(&key);
        self.prefill_modal(&current, cx);
    }

    /// `editor: go to line…` for the editor the user last focused.
    pub fn start_go_to_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let target = self.scope.target;
        if !self
            .scope
            .code_editors
            .borrow()
            .contains_key(&(key.clone(), target))
        {
            return;
        }
        self.prompt = Some(PromptPurpose::GoToLine { key, target });
        self.open_modal_keep_prompt(window, cx);
    }

    /// `request: format body`: pretty-prints a JSON or XML body as one undo step. An invalid
    /// body is left alone and the notice says where it stops being valid.
    pub fn format_body(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&key)) else {
            return;
        };
        let text = doc.target_text(Target::Body);
        let lang = crate::center::body_lang(doc, &text);
        let cfg = self
            .scope
            .envs
            .settings
            .editor
            .for_language(crate::code_editor::settings_key(lang));
        let indent = if cfg.hard_tabs {
            "\t".to_string()
        } else {
            " ".repeat(cfg.tab_size)
        };
        let result = match lang {
            Lang::Json => fmt::format_json(&text, &indent),
            Lang::Xml => fmt::format_xml(&text, &indent),
            _ => {
                self.notice = Some("Format body works on JSON and XML bodies.".into());
                return;
            }
        };
        match result {
            Err(e) => self.notice = Some(e.message()),
            Ok(new) if new == text => {}
            Ok(new) => {
                let ed = self
                    .scope
                    .code_editors
                    .borrow()
                    .get(&(key.clone(), Target::Body))
                    .cloned();
                match ed {
                    Some(ed) => ed.update(cx, |e, cx| e.replace_all(&new, cx)),
                    None => {
                        if let Some(doc) =
                            self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key))
                        {
                            doc.set_target_text(Target::Body, &new);
                        }
                        self.after_edit(&key, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn collection_name(&self, key: &str) -> Option<String> {
        let ws = self.scope.workspace.as_ref()?;
        let c = ws.collections.iter().find(|c| c.key == key)?;
        let n = &c.collection.config.collection.name;
        Some(if n.is_empty() {
            ws.name.clone()
        } else {
            n.clone()
        })
    }

    pub fn start_rename_collection(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.collection_name(key) else {
            return;
        };
        self.prompt = Some(PromptPurpose::RenameCollection {
            key: key.to_string(),
        });
        self.open_modal_keep_prompt(window, cx);
        self.prefill_modal(&current, cx);
    }

    fn do_rename_collection(&mut self, key: &str, name: &str) -> std::io::Result<Option<String>> {
        let root = self
            .scope
            .workspace
            .as_ref()
            .and_then(|w| w.collections.iter().find(|c| c.key == key))
            .map(|c| c.root.clone())
            .ok_or_else(|| std::io::Error::other("no such collection"))?;
        if !self.dirty_under(key).is_empty() {
            return Err(std::io::Error::other(
                "save or revert unsaved changes before renaming",
            ));
        }
        fmt::rename_collection(&root, name)?;
        // The collection file's title changed on disk: drop its stale in-memory copy.
        let meta = if key.is_empty() {
            "collection.flux.md".to_string()
        } else {
            format!("{key}/collection.flux.md")
        };
        if let Some(w) = self.scope.workspace.as_mut() {
            w.close_doc(&meta);
        }
        Ok(None)
    }

    /// Forget a collection's tabs, documents and selection after it left the Workspace.
    fn drop_collection_state(&mut self, key: &str) {
        let prefix = key.to_string();
        let in_collection =
            |k: &str| key.is_empty() || k == prefix || k.starts_with(&format!("{prefix}/"));
        let ids: Vec<u64> = self
            .scope
            .tabs
            .all()
            .iter()
            .filter(|t| in_collection(&t.key))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.scope.tabs.close(id);
        }
        if let Some(w) = self.scope.workspace.as_mut() {
            let stale: Vec<String> = w
                .docs
                .keys()
                .filter(|k| in_collection(k))
                .cloned()
                .collect();
            for k in stale {
                w.close_doc(&k);
            }
        }
        if self.scope.selected.as_deref().is_some_and(in_collection) {
            self.scope.selected = None;
        }
        self.session_dirty = true;
    }

    /// Take a collection out of the Workspace without touching any file.
    pub fn remove_collection(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(name) = self.collection_name(key) else {
            return;
        };
        if !self.dirty_under(key).is_empty() {
            self.notice =
                Some("Save or revert unsaved changes before removing the collection.".into());
            cx.notify();
            return;
        }
        self.drop_collection_state(key);
        if let Some(w) = self.scope.workspace.as_mut() {
            w.hidden.insert(key.to_string());
            w.rescan();
        }
        self.notice = Some(format!(
            "Removed {name} from this workspace. Use Add existing collection… to bring it back."
        ));
        cx.notify();
    }

    /// Move a collection's folder to the trash (after the confirm dialog).
    pub fn delete_collection(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(root) = self
            .scope
            .workspace
            .as_ref()
            .and_then(|w| w.collections.iter().find(|c| c.key == key))
            .map(|c| c.root.clone())
        else {
            return;
        };
        if key.is_empty() {
            self.notice = Some(
                "This collection is the workspace folder itself; close the workspace and delete the folder from your file manager.".into(),
            );
            cx.notify();
            return;
        }
        if !self.dirty_under(key).is_empty() {
            self.notice =
                Some("Save or revert unsaved changes before deleting the collection.".into());
            cx.notify();
            return;
        }
        match fmt::delete(&root) {
            Ok(()) => {
                self.drop_collection_state(key);
                if let Some(w) = self.scope.workspace.as_mut() {
                    w.hidden.remove(key);
                    w.rescan();
                }
                self.notice = Some("Moved to trash.".into());
            }
            Err(e) => self.notice = Some(format!("Delete failed: {e}")),
        }
        cx.notify();
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
        self.scope
            .workspace
            .as_ref()
            .and_then(|p| p.collections.iter().find_map(|c| find(&c.tree, key)))
            .unwrap_or_else(|| key.rsplit('/').next().unwrap_or(key).to_string())
    }

    pub fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.modal_text(cx).trim().to_string();
        let purpose = self.prompt.clone();
        self.close_modal(window, cx);
        let Some(purpose) = purpose else { return };
        if name.is_empty() {
            return;
        }
        if let PromptPurpose::GoToLine { key, target } = purpose {
            self.focus_code_editor(&key, target, None, window, cx);
            let ed = self
                .scope
                .code_editors
                .borrow()
                .get(&(key, target))
                .cloned();
            if let Some(ed) = ed
                && !ed.update(cx, |e, cx| e.go_to(&name, cx))
            {
                self.notice = Some(format!("\"{name}\" is not a line number."));
            }
            return;
        }
        let result: std::io::Result<Option<String>> = match purpose {
            PromptPurpose::Rename { key } => self.do_rename(&key, &name),
            PromptPurpose::RenameCollection { key } => self.do_rename_collection(&key, &name),
            PromptPurpose::GoToLine { .. } => return,
            p @ PromptPurpose::RenameEnvironment { .. } => {
                self.finish_env_prompt(p, &name);
                cx.notify();
                return;
            }
        };
        match result {
            Ok(open) => {
                if let Some(p) = self.scope.workspace.as_mut() {
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

    pub(crate) fn key_for(&self, abs: &Path) -> Option<String> {
        self.scope.workspace.as_ref().map(|p| p.key_of(abs))
    }

    /// Unsaved documents at or under `key`; file operations refuse to run over them.
    fn dirty_under(&self, key: &str) -> Vec<String> {
        let Some(p) = &self.scope.workspace else {
            return vec![];
        };
        p.dirty_keys()
            .into_iter()
            .filter(|k| key.is_empty() || k == key || k.starts_with(&format!("{key}/")))
            .collect()
    }

    fn do_rename(&mut self, key: &str, name: &str) -> std::io::Result<Option<String>> {
        if !self.dirty_under(key).is_empty() {
            return Err(std::io::Error::other(
                "save or revert unsaved changes before renaming",
            ));
        }
        let abs = self
            .scope
            .workspace
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
        self.scope.tabs.rekey(old, new);
        if let Some(p) = self.scope.workspace.as_mut() {
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
        if self.scope.selected.as_deref() == Some(old) {
            self.scope.selected = Some(new.to_string());
        }
        self.session_dirty = true;
    }

    pub fn duplicate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.scope.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        let Some(abs) = self.scope.workspace.as_ref().map(|p| p.abs_of(&key)) else {
            return;
        };
        match fmt::duplicate(&abs) {
            Ok(new_abs) => {
                if let Some(p) = self.scope.workspace.as_mut() {
                    p.rescan();
                }
                let nk = self.key_for(&new_abs).unwrap_or_default();
                if new_abs.is_file() {
                    self.open_request(&nk, true, window, cx);
                } else {
                    self.scope.selected = Some(nk);
                }
            }
            Err(e) => self.notice = Some(format!("Duplicate failed: {e}")),
        }
        cx.notify();
    }

    pub fn delete_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.scope.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        let Some(abs) = self.scope.workspace.as_ref().map(|p| p.abs_of(&key)) else {
            return;
        };
        match fmt::delete(&abs) {
            Ok(()) => {
                self.scope.tabs.close_key(&key);
                if let Some(p) = self.scope.workspace.as_mut() {
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
                self.scope.selected = None;
                self.notice = Some("Moved to trash.".into());
                self.session_dirty = true;
            }
            Err(e) => self.notice = Some(format!("Delete failed: {e}")),
        }
        cx.notify();
    }

    pub fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.scope.selected.clone().or_else(|| self.active_key()) else {
            return;
        };
        if let Some(abs) = self.scope.workspace.as_ref().map(|p| p.abs_of(&key)) {
            cx.reveal_path(&abs);
        }
    }

    pub fn move_item(&mut self, key: &str, dest_folder_key: &str, cx: &mut Context<Self>) {
        let Some(p) = self.scope.workspace.as_ref() else {
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
                if let Some(p) = self.scope.workspace.as_mut() {
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
        if !matches!(kind, MenuKind::Workspace | MenuKind::Environment) {
            self.scope.selected = Some(key.to_string());
        }
        self.scope.tree_menu = Some(crate::workspace::TreeMenu {
            key: key.to_string(),
            kind,
            pos,
        });
        cx.notify();
    }

    // ---- session --------------------------------------------------------------------------

    pub fn session(&self) -> Session {
        let mut s = Session::default();
        for t in self.scope.tabs.all() {
            s.tabs.push(self.session_tab(t));
        }
        s.closed = self
            .scope
            .tabs
            .closed_tabs()
            .iter()
            .map(|t| self.session_tab(t))
            .collect();
        s.active = self.scope.tabs.active().map(|t| t.key.clone());
        s.layout = self.layout();
        s.active_env = self.active_env_name();
        s.no_environment = s.active_env.is_none();
        s.hidden_collections = self
            .scope
            .workspace
            .as_ref()
            .map(|w| {
                let mut h: Vec<String> = w.hidden.iter().cloned().collect();
                h.sort();
                h
            })
            .unwrap_or_default();
        if let Some(p) = &self.scope.workspace {
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
        let (Some(dir), Some(p)) = (self.state_dir.clone(), self.scope.workspace.as_ref()) else {
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
            self.scope.workspace.as_ref().map(|p| p.root.clone()),
        ) else {
            return;
        };
        let s = crate::workspace_model::load_session(&dir, &root);
        if !s.hidden_collections.is_empty()
            && let Some(ws) = self.scope.workspace.as_mut()
        {
            ws.hidden = s.hidden_collections.iter().cloned().collect();
            ws.rescan();
        }
        self.apply_layout(&s.layout);
        for t in &s.tabs {
            if !t.kind.is_empty() {
                if self.restore_other_tab(t, cx)
                    && let Some(id) = self.scope.tabs.id_for_key(&t.key)
                {
                    if t.pinned {
                        self.scope.tabs.toggle_pin(id);
                    }
                    if let Some(ui) = self.scope.tab_ui.get_mut(&id) {
                        ui.scroll = t.scroll;
                    }
                }
                continue;
            }
            self.open_request(&t.key, !t.preview, window, cx);
            if let Some(text) = s.unsaved.get(&t.key)
                && let Some(d) = self
                    .scope
                    .workspace
                    .as_mut()
                    .and_then(|p| p.doc_mut(&t.key))
                && d.text != *text
            {
                d.set_text(text.clone());
            }
            if t.pinned
                && let Some(id) = self.scope.tabs.id_for_key(&t.key)
            {
                self.scope.tabs.toggle_pin(id);
            }
            if let Some(id) = self.scope.tabs.id_for_key(&t.key)
                && let Some(ui) = self.scope.tab_ui.get_mut(&id)
            {
                ui.scroll = t.scroll;
            }
            self.sync_tab(&t.key);
        }
        self.scope.tabs.set_closed(
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
            && let Some(id) = self.scope.tabs.id_for_key(a)
        {
            self.scope.tabs.activate(id);
        }
    }
}
