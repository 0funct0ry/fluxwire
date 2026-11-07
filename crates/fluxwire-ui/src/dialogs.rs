//! Dialogs for the Workspace and Collection lifecycle: New workspace, New collection and the
//! confirm dialog (SPEC §17.8, §17.9). Validation lives in `containment.rs`.

use crate::dev_id::{DevClick, Role};
use std::path::{Path, PathBuf};

use gpui::{
    Context, Entity, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, PathPromptOptions, SharedString, Styled, Subscription, Window, div, prelude::*,
    px,
};

use crate::{
    assets::BUFFER_FONT,
    containment::{
        self, AddOutcome, NewCollectionError, NewWorkspaceError, classify_add, plan_new_collection,
        plan_new_workspace,
    },
    text_input::{TextInput, TextInputEvent},
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
};

pub struct NewWorkspaceForm {
    pub name: Entity<TextInput>,
    pub location: PathBuf,
    _sub: Subscription,
}

pub struct NewEnvForm {
    pub name: Entity<TextInput>,
    pub collection: PathBuf,
    /// Environment to copy both files from; `None` starts empty.
    pub start_from: Option<String>,
    pub activate: bool,
    _sub: Subscription,
}

pub struct NewCollectionForm {
    pub name: Entity<TextInput>,
    pub location: PathBuf,
    pub note: Option<String>,
    _sub: Subscription,
}

/// A dialog's name field; the dialog re-renders when its text changes (validation, Create).
fn name_input(cx: &mut Context<WorkspaceView>) -> (Entity<TextInput>, Subscription) {
    let input = cx.new(|cx| TextInput::new("", cx));
    let sub = cx.subscribe(&input, |_, _, ev: &TextInputEvent, cx| {
        if *ev == TextInputEvent::Changed {
            cx.notify();
        }
    });
    (input, sub)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmAction {
    Cancel,
    /// Clear a scope of History (pinned entries stay unless `include_pinned`).
    ClearHistory {
        scope: fluxwire_store::Scope,
        include_pinned: bool,
    },
    /// Delete these History entries (pinned or not: this is explicit).
    DeleteHistory(Vec<i64>),
    /// Close the Workspace (after the confirm), then do `AfterClose`.
    Close(AfterClose, CloseMode),
    /// Start a new request/folder in this directory (collection picker).
    StartNew {
        websocket: bool,
        folder: bool,
        dir: PathBuf,
    },
    /// Write the missing collection files into this existing folder.
    MakeCollection(PathBuf),
    /// Close this Workspace and open the folder as the Workspace.
    OpenAsWorkspace(PathBuf),
    /// Move this collection (by key) to the trash.
    DeleteCollection(String),
    /// Close an Environment tab with unsaved edits: save first (true) or discard (false).
    CloseEnvTab(u64, bool),
    /// Move an environment's files to the trash.
    DeleteEnvironment {
        root: PathBuf,
        name: String,
    },
    /// Remove a Spec link from `fluxwire.toml` and its base snapshot; spec and Requests stay.
    RemoveSpecLink {
        root: PathBuf,
        name: String,
    },
    /// Open a web page in the browser.
    OpenUrl(String),
    /// Run the update check (M14).
    CheckUpdates,
    /// Flip `check_for_updates` in `settings.toml`.
    ToggleAutoUpdates,
    /// Open a crash log (M14).
    OpenPath(PathBuf),
}

/// What happens once the Workspace has been closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterClose {
    Welcome,
    Open(PathBuf),
    /// Quit the app; keeps `recent.last` so the Workspace restores on the next launch.
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    Save,
    Discard,
    Stop,
}

pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    /// First is the default (Enter); Esc always cancels.
    pub buttons: Vec<(String, ConfirmAction)>,
}

pub enum Dialog {
    NewWorkspace(NewWorkspaceForm),
    NewCollection(NewCollectionForm),
    NewEnvironment(NewEnvForm),
    NewRequest(Box<crate::new_request_dialog::NewRequestDialog>),
    Import(Box<crate::import_dialog::ImportDialog>),
    Confirm(Confirm),
}

impl Dialog {
    /// A short stable name for `ui.state`.
    pub fn name(&self) -> String {
        match self {
            Dialog::NewWorkspace(_) => "new_workspace",
            Dialog::NewCollection(_) => "new_collection",
            Dialog::NewEnvironment(_) => "new_environment",
            Dialog::NewRequest(_) => "new_request",
            Dialog::Import(_) => "import",
            Dialog::Confirm(c) => return format!("confirm:{}", c.title),
        }
        .into()
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl WorkspaceView {
    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.take().is_some() {
            window.focus(&self.root_focus);
            cx.notify();
        }
    }

    // ---- New workspace --------------------------------------------------------------------

    pub fn start_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let location = self
            .recent
            .workspaces
            .first()
            .and_then(|w| Path::new(w).parent().map(Path::to_path_buf))
            .filter(|p| p.is_dir())
            .unwrap_or_else(home_dir);
        let (name, _sub) = name_input(cx);
        window.focus(&name.read(cx).focus_handle(cx));
        self.dialog = Some(Dialog::NewWorkspace(NewWorkspaceForm {
            name,
            location,
            _sub,
        }));
        cx.notify();
    }

    fn submit_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::NewWorkspace(f)) = &self.dialog else {
            return;
        };
        let Ok(target) = plan_new_workspace(f.name.read(cx).text(), &f.location) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&target) {
            self.notice = Some(format!("Could not create {}: {e}", target.display()));
            cx.notify();
            return;
        }
        self.dialog = None;
        self.request_close(AfterClose::Open(target), window, cx);
    }

    // ---- New environment ------------------------------------------------------------------

    /// Open the New environment dialog for the current collection.
    pub fn start_new_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root() {
            self.start_new_env_in(&root, "", window, cx);
        }
    }

    pub fn start_new_env_in(
        &mut self,
        root: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (input, _sub) = name_input(cx);
        input.update(cx, |i, cx| i.set_text(name, cx));
        window.focus(&input.read(cx).focus_handle(cx));
        self.dialog = Some(Dialog::NewEnvironment(NewEnvForm {
            name: input,
            collection: root.to_path_buf(),
            start_from: None,
            activate: true,
            _sub,
        }));
        cx.notify();
    }

    fn new_env_error(&self, f: &NewEnvForm, cx: &gpui::App) -> Option<String> {
        self.env_name_error(&f.collection, f.name.read(cx).text(), None)
    }

    fn submit_new_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::NewEnvironment(f)) = &self.dialog else {
            return;
        };
        if self.new_env_error(f, cx).is_some() {
            return;
        }
        let (root, name, from, activate) = (
            f.collection.clone(),
            f.name.read(cx).text().to_string(),
            f.start_from.clone(),
            f.activate,
        );
        let made = match &from {
            Some(src) => [
                (
                    fluxwire_format::env_path(&root, src),
                    fluxwire_format::env_path(&root, &name),
                ),
                (
                    fluxwire_format::local_env_path(&root, src),
                    fluxwire_format::local_env_path(&root, &name),
                ),
            ]
            .into_iter()
            .try_for_each(|(a, b)| {
                if a.exists() {
                    std::fs::create_dir_all(b.parent().unwrap_or(&root))?;
                    std::fs::copy(a, b).map(|_| ())
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                let p = fluxwire_format::env_path(&root, &name);
                if p.exists() {
                    Ok(())
                } else {
                    fluxwire_format::create_environment(&root, &name)
                }
            }),
            None => fluxwire_format::create_environment(&root, &name),
        };
        if let Err(e) = made {
            self.notice = Some(format!("Could not create {name}: {e}"));
            cx.notify();
            return;
        }
        self.dialog = None;
        self.scope.envs.invalidate();
        if activate {
            self.set_active_env(Some(name.clone()), cx);
        }
        self.open_env_doc(&root, &name, true);
        self.env_focus_first_cell(&(root, name), window, cx);
        self.session_dirty = true;
        cx.notify();
    }

    // ---- New collection -------------------------------------------------------------------

    /// Open the New collection dialog (needs an open Workspace). `note` explains why it appeared.
    pub fn start_new_collection(
        &mut self,
        note: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        if ws.root.join("fluxwire.toml").is_file() {
            self.notice = Some(SINGLE_COLLECTION_TIP.into());
            cx.notify();
            return;
        }
        let (location, note) = (ws.root.clone(), note.map(str::to_string));
        let (name, _sub) = name_input(cx);
        window.focus(&name.read(cx).focus_handle(cx));
        self.dialog = Some(Dialog::NewCollection(NewCollectionForm {
            name,
            location,
            note,
            _sub,
        }));
        cx.notify();
    }

    fn collection_plan(
        &self,
        f: &NewCollectionForm,
        cx: &gpui::App,
    ) -> Result<PathBuf, NewCollectionError> {
        let ws = self
            .scope
            .workspace
            .as_ref()
            .ok_or(NewCollectionError::EmptyName)?;
        plan_new_collection(ws, f.name.read(cx).text(), &f.location)
    }

    fn submit_new_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::NewCollection(f)) = &self.dialog else {
            return;
        };
        let Ok(target) = self.collection_plan(f, cx) else {
            return;
        };
        let name = f.name.read(cx).text().trim().to_string();
        self.dialog = None;
        self.create_collection_at(&target, &name, window, cx);
    }

    /// Create the Collection files in `dir`, rescan, expand and select it and open its
    /// `collection.flux.md` as a preview tab.
    pub fn create_collection_at(
        &mut self,
        dir: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = fluxwire_format::create_collection(dir, name) {
            self.notice = Some(format!("Could not create collection: {e}"));
            cx.notify();
            return;
        }
        let Some(ws) = self.scope.workspace.as_mut() else {
            return;
        };
        ws.rescan();
        let ckey = ws.key_of(dir);
        let doc_key = if ckey.is_empty() {
            "collection.flux.md".to_string()
        } else {
            format!("{ckey}/collection.flux.md")
        };
        self.scope.collapsed.remove(&ckey);
        self.open_request(&doc_key, false, window, cx);
        self.scope.selected = Some(ckey);
        self.session_dirty = true;
        window.focus(&self.root_focus);
        cx.notify();
    }

    // ---- Delete collection ----------------------------------------------------------------

    pub fn confirm_delete_collection(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        let Some(c) = ws.collections.iter().find(|c| c.key == key) else {
            return;
        };
        let name = c.collection.config.collection.name.clone();
        let name = if name.is_empty() {
            ws.name.clone()
        } else {
            name
        };
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: format!("Move {name} to the trash?"),
            lines: vec![if key.is_empty() {
                // The collection is the Workspace folder itself: its contents go, the folder
                // stays as an empty Workspace.
                format!(
                    "Everything in {} except .git will be moved to the trash. The Workspace stays open and empty.",
                    c.root.display()
                )
            } else {
                format!(
                    "The folder {} and everything in it will be moved to the trash.",
                    c.root.display()
                )
            }],
            buttons: vec![
                (
                    "Move to trash".into(),
                    ConfirmAction::DeleteCollection(key.to_string()),
                ),
                ("Cancel".into(), ConfirmAction::Cancel),
            ],
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    // ---- Add existing collection ----------------------------------------------------------

    pub fn start_add_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        if ws.root.join("fluxwire.toml").is_file() {
            self.notice = Some(SINGLE_COLLECTION_TIP.into());
            cx.notify();
            return;
        }
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
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.add_collection_dir(&dir, window, cx)
            })
            .ok();
        })
        .detach();
    }

    pub fn add_collection_dir(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.scope.workspace.as_mut() else {
            return;
        };
        let label = dir.file_name().map_or(dir.display().to_string(), |n| {
            n.to_string_lossy().into_owned()
        });
        match classify_add(ws, dir) {
            AddOutcome::AlreadyLoaded(p) => {
                let key = ws.key_of(&p);
                self.scope.collapsed.remove(&key);
                self.scope.selected = Some(key);
                self.notice = Some(format!("{label} is already part of this workspace."));
            }
            AddOutcome::Reveal(p) => {
                ws.hidden.remove(&ws.key_of(&p));
                ws.rescan();
                let key = ws.key_of(&p);
                self.scope.collapsed.remove(&key);
                self.scope.selected = Some(key);
            }
            AddOutcome::MakeCollection(p) => {
                self.dialog = Some(Dialog::Confirm(Confirm {
                    title: format!("Make {label} a collection?"),
                    lines: vec![
                        "Fluxwire will add fluxwire.toml, collection.flux.md, environments/ and .gitignore to this folder."
                            .into(),
                    ],
                    buttons: vec![
                        ("Make collection".into(), ConfirmAction::MakeCollection(p)),
                        ("Cancel".into(), ConfirmAction::Cancel),
                    ],
                }));
                window.focus(&self.dialog_focus);
            }
            AddOutcome::Refused(containment::Rule::Outside) => {
                self.dialog = Some(Dialog::Confirm(Confirm {
                    title: "Outside this workspace".into(),
                    lines: vec![format!(
                        "{label} is outside this workspace. Collections must live inside the workspace folder."
                    )],
                    buttons: vec![
                        (
                            format!("Open {label} as workspace"),
                            ConfirmAction::OpenAsWorkspace(dir.to_path_buf()),
                        ),
                        ("Cancel".into(), ConfirmAction::Cancel),
                    ],
                }));
                window.focus(&self.dialog_focus);
            }
            AddOutcome::Refused(rule) => {
                self.dialog = Some(Dialog::Confirm(Confirm {
                    title: format!("Can't add {label}"),
                    lines: vec![rule.to_string()],
                    buttons: vec![("OK".into(), ConfirmAction::Cancel)],
                }));
                window.focus(&self.dialog_focus);
            }
        }
        cx.notify();
    }

    pub fn run_confirm_action(
        &mut self,
        action: ConfirmAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dialog = None;
        window.focus(&self.root_focus);
        match action {
            ConfirmAction::Cancel => {}
            ConfirmAction::MakeCollection(dir) => {
                let name = dir
                    .file_name()
                    .map_or("Collection".into(), |n| n.to_string_lossy().into_owned());
                self.create_collection_at(&dir, &name, window, cx);
            }
            ConfirmAction::OpenAsWorkspace(dir) => {
                self.request_close(AfterClose::Open(dir), window, cx)
            }
            ConfirmAction::DeleteCollection(key) => self.delete_collection(&key, cx),
            ConfirmAction::ClearHistory {
                scope,
                include_pinned,
            } => self.history_clear_scope(scope, include_pinned, cx),
            ConfirmAction::DeleteHistory(ids) => self.history_delete(ids, cx),
            ConfirmAction::DeleteEnvironment { root, name } => self.delete_env_in(&root, &name, cx),
            ConfirmAction::RemoveSpecLink { root, name } => self.remove_spec_link(&root, &name, cx),
            ConfirmAction::OpenUrl(u) => cx.open_url(&u),
            ConfirmAction::CheckUpdates => self.check_updates(true, window, cx),
            ConfirmAction::ToggleAutoUpdates => self.toggle_auto_updates(window, cx),
            ConfirmAction::OpenPath(p) => cx.open_url(&format!("file://{}", p.display())),
            ConfirmAction::CloseEnvTab(id, save) => self.finish_close_env_tab(id, save, cx),
            ConfirmAction::Close(after, mode) => self.finish_close(after, mode, window, cx),
            ConfirmAction::StartNew {
                websocket,
                folder,
                dir,
            } => self.start_new_at(websocket, folder, dir, window, cx),
        }
        cx.notify();
    }

    // ---- closing --------------------------------------------------------------------------

    /// Close the Workspace (or quit) and then do `after`. One confirm when there are unsaved
    /// requests or running work; none when clean and idle.
    pub fn request_close(
        &mut self,
        after: AfterClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            self.finish_close(after, CloseMode::Discard, window, cx);
            return;
        };
        let names: Vec<String> = ws
            .dirty_keys()
            .iter()
            .map(|k| ws.doc(k).map_or(k.clone(), |d| d.title()))
            .chain(
                self.dirty_env_docs()
                    .into_iter()
                    .map(|(_, n)| format!("environment {n}")),
            )
            .collect();
        let running = self.scope.running();
        if names.is_empty() && running == 0 {
            self.finish_close(after, CloseMode::Stop, window, cx);
            return;
        }
        let quit = after == AfterClose::Quit;
        let verb = if quit { "quit" } else { "close" };
        let mut lines = vec![];
        if !names.is_empty() {
            let shown = names.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
            let more = names.len().saturating_sub(5);
            lines.push(format!(
                "{} unsaved request{}: {shown}{}",
                names.len(),
                if names.len() == 1 { "" } else { "s" },
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
        if running > 0 {
            lines.push(format!("{running} running"));
        }
        let buttons = if names.is_empty() {
            vec![
                (
                    format!("Stop and {verb}"),
                    ConfirmAction::Close(after, CloseMode::Stop),
                ),
                ("Cancel".into(), ConfirmAction::Cancel),
            ]
        } else {
            vec![
                (
                    format!("Save all and {verb}"),
                    ConfirmAction::Close(after.clone(), CloseMode::Save),
                ),
                (
                    format!("Discard and {verb}"),
                    ConfirmAction::Close(after, CloseMode::Discard),
                ),
                ("Cancel".into(), ConfirmAction::Cancel),
            ]
        };
        let title = if quit {
            "Quit Fluxwire?".to_string()
        } else {
            format!("Close {}?", ws.name)
        };
        self.dialog = Some(Dialog::Confirm(Confirm {
            title,
            lines,
            buttons,
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    fn finish_close(
        &mut self,
        after: AfterClose,
        mode: CloseMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match mode {
            CloseMode::Save => {
                if let Err(e) = self.save_all_env_docs(cx) {
                    self.notice = Some(e);
                    cx.notify();
                    return;
                }
            }
            CloseMode::Discard => {
                for k in self.dirty_env_docs() {
                    self.discard_env_doc(&k);
                }
            }
            CloseMode::Stop => {}
        }
        if let Some(ws) = self.scope.workspace.as_mut() {
            let dirty = ws.dirty_keys();
            match mode {
                CloseMode::Save => {
                    for key in &dirty {
                        if let Some(Err(e)) = ws.doc_mut(key).map(|d| d.save()) {
                            // A failed save aborts the close.
                            self.notice = Some(format!("Could not save {key}: {e}"));
                            cx.notify();
                            return;
                        }
                    }
                }
                CloseMode::Discard => {
                    for key in &dirty {
                        ws.close_doc(key);
                    }
                }
                CloseMode::Stop => {}
            }
        }
        self.dialog = None;
        match after {
            AfterClose::Quit => {
                self.save_session_now();
                cx.quit();
            }
            AfterClose::Welcome => {
                self.close_workspace();
                window.focus(&self.root_focus);
            }
            AfterClose::Open(path) => {
                self.close_workspace();
                self.open_workspace(&path, window, cx);
            }
        }
        cx.notify();
    }

    // ---- new request/folder routing -------------------------------------------------------

    /// Where a new request/folder goes: the Collection of the selected tree item, else of the
    /// active tab, else the only Collection; with several, ask. With none, New collection.
    pub fn route_new(
        &mut self,
        websocket: bool,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        if ws.collections.is_empty() {
            self.start_new_collection(
                Some("Requests live in a collection. Create one first."),
                window,
                cx,
            );
            return;
        }
        let from_key = self.scope.selected.clone().or_else(|| self.active_key());
        if let Some(key) = from_key {
            let abs = ws.abs_of(&key);
            if let Some(c) = ws
                .collections
                .iter()
                .filter(|c| abs.starts_with(&c.root))
                .max_by_key(|c| c.root.components().count())
            {
                let _ = c;
                self.start_new(websocket, folder, window, cx);
                return;
            }
        }
        if ws.collections.len() == 1 {
            self.start_new(websocket, folder, window, cx);
            return;
        }
        let mut buttons: Vec<(String, ConfirmAction)> = ws
            .collections
            .iter()
            .map(|c| {
                let name = if c.collection.config.collection.name.is_empty() {
                    ws.name.clone()
                } else {
                    c.collection.config.collection.name.clone()
                };
                (
                    format!("In {name}"),
                    ConfirmAction::StartNew {
                        websocket,
                        folder,
                        dir: c.root.clone(),
                    },
                )
            })
            .collect();
        buttons.push(("Cancel".into(), ConfirmAction::Cancel));
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "Which collection?".into(),
            lines: vec!["Requests and folders live in a collection.".into()],
            buttons,
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    fn start_new_at(
        &mut self,
        websocket: bool,
        folder: bool,
        dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.scope.workspace.as_ref().map(|w| w.key_of(&dir)) {
            self.scope.selected = Some(key);
        }
        self.start_new(websocket, folder, window, cx);
    }

    fn name_field(&self, t: &Theme, edit: &Entity<TextInput>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child("Name"),
            )
            .child(
                div()
                    .id("dlg-name")
                    .px_2()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.accent)
                    .bg(t.editor)
                    .child(div().flex_1().child(edit.clone())),
            )
    }

    // ---- keys -----------------------------------------------------------------------------

    fn on_dialog_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if key == "escape" {
            if matches!(self.dialog, Some(Dialog::NewRequest(_))) {
                self.nr_escape(window, cx);
            } else {
                self.close_dialog(window, cx);
            }
            return;
        }
        if key == "enter" && matches!(self.dialog, Some(Dialog::NewRequest(_))) {
            self.nr_enter(ev.keystroke.modifiers.secondary(), window, cx);
            return;
        }
        match (&mut self.dialog, key) {
            (Some(Dialog::NewWorkspace(_)), "enter") => self.submit_new_workspace(window, cx),
            (Some(Dialog::NewCollection(_)), "enter") => self.submit_new_collection(window, cx),
            (Some(Dialog::NewEnvironment(_)), "enter") => self.submit_new_env(window, cx),
            (Some(Dialog::Confirm(c)), "enter") => {
                if let Some((_, a)) = c.buttons.first().cloned() {
                    self.run_confirm_action(a, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn choose_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                match &mut this.dialog {
                    Some(Dialog::NewWorkspace(f)) => f.location = dir,
                    Some(Dialog::NewCollection(f)) => f.location = dir,
                    _ => {}
                }
                match &this.dialog {
                    Some(Dialog::NewWorkspace(f)) => {
                        window.focus(&f.name.read(cx).focus_handle(cx))
                    }
                    Some(Dialog::NewCollection(f)) => {
                        window.focus(&f.name.read(cx).focus_handle(cx))
                    }
                    _ => window.focus(&this.dialog_focus),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- rendering ------------------------------------------------------------------------

    pub(crate) fn render_dialog(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let dialog = self.dialog.as_ref()?;
        let t = t.clone();
        let mut panel = div()
            .id("dialog")
            .track_focus(&self.dialog_focus)
            .on_key_down(cx.listener(|this, ev, w, cx| this.on_dialog_key(ev, w, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(520.))
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .rounded(px(6.))
            .shadow_lg()
            .text_size(px(13.));
        let heading = |s: String| {
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .child(s)
        };
        match dialog {
            Dialog::NewRequest(d) => {
                panel = panel.w(px(560.)).child(self.render_new_request(d, &t, cx));
            }
            Dialog::Import(d) => {
                panel = panel.w(px(600.)).child(self.render_import(d, &t, cx));
            }
            Dialog::NewWorkspace(f) => {
                let plan = plan_new_workspace(f.name.read(cx).text(), &f.location);
                let (preview, error) = match &plan {
                    Ok(p) => (Some(p.display().to_string()), None),
                    Err(e) => (None, Some(e.clone())),
                };
                // Don't nag about an empty name before anything is typed.
                let error = error.filter(|e| *e != NewWorkspaceError::EmptyName);
                let open_existing = match &error {
                    Some(NewWorkspaceError::ExistsNonEmpty(p)) => Some(p.clone()),
                    _ => None,
                };
                panel = panel
                    .child(heading("New workspace".into()))
                    .child(self.name_field(&t, &f.name))
                    .child(location_row(&t, &f.location.display().to_string(), cx))
                    .children(preview.map(|p| muted_path(&t, format!("Will create {p}"))))
                    .children(error.map(|e| error_line(&t, e.to_string())))
                    .children(open_existing.map(|p| {
                        link(&t, "dlg-open-existing", "Open workspace").dev_click(
                            "dialog",
                            Role::Button,
                            "dialog",
                            true,
                            cx.listener(move |this, _, w, cx| {
                                this.dialog = None;
                                this.open_workspace(&p, w, cx);
                            }),
                        )
                    }))
                    .child(buttons(&t, cx, "Create", plan.is_ok(), |this, w, cx| {
                        this.submit_new_workspace(w, cx)
                    }));
            }
            Dialog::NewCollection(f) => {
                let ws = self.scope.workspace.as_ref();
                let root = ws.map(|w| w.root.clone()).unwrap_or_default();
                let plan = self.collection_plan(f, cx);
                let error = match &plan {
                    Err(NewCollectionError::EmptyName) => None,
                    Err(e) => Some(e.clone()),
                    Ok(_) => None,
                };
                let make_folder = match &error {
                    Some(NewCollectionError::NonEmptyFolder(p)) => Some(p.clone()),
                    _ => None,
                };
                let rel = |p: &Path| {
                    let r = p.strip_prefix(&root).unwrap_or(p);
                    if r.as_os_str().is_empty() {
                        "./".to_string()
                    } else {
                        format!("{}/", r.display())
                    }
                };
                let preview = plan
                    .as_ref()
                    .ok()
                    .map(|p| format!("Will create {}", rel(p)));
                panel = panel.child(heading("New collection".into()));
                if let Some(n) = &f.note {
                    panel = panel.child(div().text_color(t.text_muted).child(n.clone()));
                }
                panel = panel.child(self.name_field(&t, &f.name));
                panel = panel
                    .children(preview.map(|p| muted_path(&t, p)))
                    .children(error.map(|e| error_line(&t, e.to_string())))
                    .children(make_folder.map(|p| {
                        link(&t, "dlg-make-folder", "Make that folder a collection").dev_click(
                            "dialogs.render_dialog",
                            Role::Button,
                            "dialogs.render_dialog",
                            true,
                            cx.listener(move |this, _, w, cx| {
                                let name = this
                                    .dialog
                                    .as_ref()
                                    .and_then(|d| match d {
                                        Dialog::NewCollection(f) => {
                                            Some(f.name.read(cx).text().trim().to_string())
                                        }
                                        _ => None,
                                    })
                                    .unwrap_or_default();
                                this.dialog = None;
                                this.create_collection_at(&p, &name, w, cx);
                            }),
                        )
                    }))
                    .child(buttons(&t, cx, "Create", plan.is_ok(), |this, w, cx| {
                        this.submit_new_collection(w, cx)
                    }));
            }
            Dialog::NewEnvironment(f) => {
                let error = self
                    .new_env_error(f, cx)
                    .filter(|_| !f.name.read(cx).text().is_empty());
                let valid = self.new_env_error(f, cx).is_none();
                let chip = |id: gpui::ElementId, label: String, on: bool| {
                    div()
                        .id(id)
                        .px_2()
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .cursor_pointer()
                        .text_size(px(12.))
                        .child(label)
                };
                let label = |s: &'static str| {
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(s)
                };
                panel = panel
                    .child(heading("New environment".into()))
                    .child(self.name_field(&t, &f.name));
                let existing = fluxwire_format::list_environments(&f.collection);
                panel = panel.child(label("Start from")).child(
                    row()
                        .gap_2()
                        .child(
                            chip(
                                "env-from-empty".into(),
                                "Empty".into(),
                                f.start_from.is_none(),
                            )
                            .dev_click(
                                "env-from-empty",
                                Role::Button,
                                "Empty",
                                true,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(Dialog::NewEnvironment(f)) = &mut this.dialog {
                                        f.start_from = None;
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .children(existing.into_iter().enumerate().map(|(i, n)| {
                            let on = f.start_from.as_deref() == Some(n.as_str());
                            let pick = n.clone();
                            chip(("env-from", i).into(), format!("Duplicate of {n}"), on).dev_click(
                                "dialogs.render_dialog",
                                Role::Button,
                                "dialogs.render_dialog",
                                true,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(Dialog::NewEnvironment(f)) = &mut this.dialog {
                                        f.start_from = Some(pick.clone());
                                    }
                                    cx.notify();
                                }),
                            )
                        })),
                );
                let name_now = f.name.read(cx).text().to_string();
                panel = panel
                    .child(
                        chip(
                            "env-activate".into(),
                            format!("{} Set as active", if f.activate { "☑" } else { "☐" }),
                            false,
                        )
                        .dev_click(
                            "dialogs.render_dialog",
                            Role::Button,
                            "dialogs.render_dialog",
                            true,
                            cx.listener(|this, _, _, cx| {
                                if let Some(Dialog::NewEnvironment(f)) = &mut this.dialog {
                                    f.activate = !f.activate;
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(muted_path(
                        &t,
                        format!(
                            "environments/{}.env",
                            if name_now.is_empty() {
                                "<name>"
                            } else {
                                &name_now
                            }
                        ),
                    ))
                    .children(error.map(|e| error_line(&t, e)))
                    .child(buttons(&t, cx, "Create", valid, |this, w, cx| {
                        this.submit_new_env(w, cx)
                    }));
            }
            Dialog::Confirm(c) => {
                panel = panel.child(heading(c.title.clone()));
                for l in &c.lines {
                    panel = panel.child(div().text_color(t.text_muted).child(l.clone()));
                }
                let mut bar = row().gap_2().justify_end();
                for (i, (label, action)) in c.buttons.iter().cloned().enumerate() {
                    let primary = i == 0 && action != ConfirmAction::Cancel;
                    bar = bar.child(
                        button(&t, ("dlg-confirm", i), label, primary, true).dev_click(
                            "dialogs.render_dialog",
                            Role::Button,
                            "dialogs.render_dialog",
                            true,
                            cx.listener(move |this, _, w, cx| {
                                this.run_confirm_action(action.clone(), w, cx)
                            }),
                        ),
                    );
                }
                panel = panel.child(bar);
            }
        }
        Some(
            div()
                .id("dialog-backdrop")
                .absolute()
                .size_full()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(100.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
                )
                .child(panel),
        )
    }
}

pub const SINGLE_COLLECTION_TIP: &str = "This workspace is itself a collection (fluxwire.toml at its root). Collections can't be nested — open the parent folder as the workspace to keep several collections side by side.";

fn location_row(t: &Theme, shown: &str, cx: &mut Context<WorkspaceView>) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(t.text_placeholder)
                .child("Location"),
        )
        .child(
            row()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .truncate()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .child(crate::dock::middle_truncate(shown, 52)),
                )
                .child(
                    button(t, "dlg-choose", "Choose…".to_string(), false, true).dev_click(
                        "dlg-choose",
                        Role::Button,
                        "dlg-choose",
                        true,
                        cx.listener(|this, _, w, cx| this.choose_location(w, cx)),
                    ),
                ),
        )
}

fn muted_path(t: &Theme, s: String) -> gpui::Div {
    div()
        .font_family(BUFFER_FONT)
        .text_size(px(12.))
        .text_color(t.text_muted)
        .child(s)
}

fn error_line(t: &Theme, s: String) -> gpui::Div {
    div().text_size(px(12.)).text_color(t.deleted).child(s)
}

fn link(t: &Theme, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(t.accent)
        .child(label)
}

pub(crate) fn button(
    t: &Theme,
    id: impl Into<gpui::ElementId>,
    label: String,
    primary: bool,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let t = t.clone();
    row()
        .id(id)
        .px_3()
        .h(px(28.))
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(t.border)
        .when(enabled, |d| d.cursor_pointer())
        .when(!enabled, |d| d.opacity(0.4))
        .when(primary && enabled, |d| {
            d.bg(t.accent).text_color(t.editor).border_color(t.accent)
        })
        .when(!primary && enabled, |d| d.hover(|s| s.bg(t.element_hover)))
        .child(SharedString::from(label))
}

fn buttons(
    t: &Theme,
    cx: &mut Context<WorkspaceView>,
    create_label: &'static str,
    valid: bool,
    submit: fn(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>),
) -> gpui::Div {
    row()
        .gap_2()
        .justify_end()
        .child(
            button(t, "dlg-cancel", "Cancel".to_string(), false, true).dev_click(
                "dlg-cancel",
                Role::Button,
                "dlg-cancel",
                true,
                cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
            ),
        )
        .child(
            button(t, "dlg-create", create_label.to_string(), true, valid).dev_click(
                "dlg-create",
                Role::Button,
                "dlg-create",
                true,
                cx.listener(move |this, _, w, cx| {
                    if valid {
                        submit(this, w, cx)
                    }
                }),
            ),
        )
}
