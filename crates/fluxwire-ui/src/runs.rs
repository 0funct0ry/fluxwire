//! Folder runs in the UI (SPEC §12): the Folder run tab, its options bar, the live result
//! tree, export, "Copy CLI command", and the history entries every request of a run writes.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};

use fluxwire_runner::{
    FolderRunEvent, FolderRunResult, PlannedRequest, RequestRun, RequestStatus, RunMode, RunPlan,
    load_data_file, runner, to_json, to_junit,
};
use fluxwire_store::{NewEntry, Redactor};
use gpui::{
    AnyElement, ClipboardItem, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, div, prelude::*, px, relative,
    uniform_list,
};

use crate::{
    assets::BUFFER_FONT,
    center::button,
    history::blocking,
    tabs::Tab,
    theme::Theme,
    ui::row,
    workspace::{Panel, WorkspaceView},
};

/// Where a request is in the run.
#[derive(Clone)]
pub enum Slot {
    Pending,
    Running(Instant),
    Done(Arc<RequestRun>),
}

#[derive(Clone)]
pub struct Entry {
    pub iteration: u32,
    pub request: PlannedRequest,
    pub slot: Slot,
}

#[derive(Clone)]
pub enum RunRow {
    Iteration(u32),
    Request(usize),
    Detail { ok: bool, text: String },
    Open(usize),
}

pub struct RunTab {
    pub folder: String,
    pub sequential: bool,
    pub concurrency: u32,
    pub iterations: u32,
    pub bail: bool,
    pub data_file: Option<PathBuf>,
    pub filter: String,
    pub filter_input: gpui::Entity<crate::text_input::TextInput>,
    pub focus: gpui::FocusHandle,
    /// Run only these keys (set by "Re-run failed", cleared by Run).
    pub only: Option<Vec<String>>,
    pub seq: u64,
    pub cancel: Option<fluxwire_runner::CancellationToken>,
    pub started: Option<Instant>,
    pub run_id: Option<String>,
    pub iterations_run: u32,
    pub entries: Vec<Entry>,
    pub expanded: HashSet<usize>,
    pub flat: Arc<Vec<RunRow>>,
    pub scroll: UniformListScrollHandle,
    pub max_ms: u64,
    pub result: Option<Arc<FolderRunResult>>,
    pub error: Option<String>,
}

impl RunTab {
    fn new(id: u64, folder: String, cx: &mut Context<WorkspaceView>) -> Self {
        let filter_input = crate::text_input::bound_input(
            "tag:smoke, name:*refund*",
            cx,
            move |this: &mut WorkspaceView, text, cx| {
                if let Some(r) = this.scope.runs.get_mut(&id) {
                    r.filter = text;
                }
                cx.notify();
            },
        );
        Self {
            folder,
            sequential: false,
            concurrency: fluxwire_runner::DEFAULT_CONCURRENCY,
            iterations: 1,
            bail: false,
            data_file: None,
            filter: String::new(),
            filter_input,
            focus: cx.focus_handle(),
            only: None,
            seq: 0,
            cancel: None,
            started: None,
            run_id: None,
            iterations_run: 0,
            entries: Vec::new(),
            expanded: HashSet::new(),
            flat: Arc::default(),
            scroll: UniformListScrollHandle::new(),
            max_ms: 1,
            result: None,
            error: None,
        }
    }

    pub fn running(&self) -> bool {
        self.cancel.is_some()
    }

    fn counts(&self) -> (usize, usize, usize, usize) {
        let (mut p, mut f, mut s, mut pending) = (0, 0, 0, 0);
        for e in &self.entries {
            match &e.slot {
                Slot::Done(r) => match r.status() {
                    RequestStatus::Passed => p += 1,
                    RequestStatus::Failed => f += 1,
                    RequestStatus::Skipped => s += 1,
                },
                _ => pending += 1,
            }
        }
        (p, f, s, pending)
    }

    fn rebuild_flat(&mut self) {
        let mut rows = Vec::new();
        let mut last_iter = None;
        for (i, e) in self.entries.iter().enumerate() {
            if self.iterations_run > 1 && last_iter != Some(e.iteration) {
                rows.push(RunRow::Iteration(e.iteration));
                last_iter = Some(e.iteration);
            }
            rows.push(RunRow::Request(i));
            if self.expanded.contains(&i)
                && let Slot::Done(r) = &e.slot
            {
                let res = &r.result;
                if let Some(err) = &res.error {
                    rows.push(RunRow::Detail {
                        ok: false,
                        text: res.redact(&err.describe().0),
                    });
                }
                for s in &res.script_errors {
                    rows.push(RunRow::Detail {
                        ok: false,
                        text: res.redact(&format!("{} ({})", s.message, s.location())),
                    });
                }
                if let Some(why) = &res.skipped {
                    rows.push(RunRow::Detail {
                        ok: true,
                        text: why.clone(),
                    });
                }
                for t in &res.tests {
                    let mut text = res.redact(&t.name);
                    if !t.passed {
                        if let Some(m) = &t.message {
                            text += &format!(" — {}", res.redact(m));
                        }
                        if let Some(x) = &t.expected {
                            text += &format!("\n    expected: {}", res.redact(x));
                        }
                        if let Some(a) = &t.actual {
                            text += &format!("\n    actual:   {}", res.redact(a));
                        }
                    }
                    rows.push(RunRow::Detail { ok: t.passed, text });
                }
                rows.push(RunRow::Open(i));
            }
        }
        self.flat = Arc::new(rows);
    }
}

fn shell_quote(s: &str) -> String {
    if s.is_empty()
        || s.chars()
            .any(|c| !(c.is_alphanumeric() || "-_./:=*@".contains(c)))
    {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

impl WorkspaceView {
    /// The folder key a run acts on: the selected folder (or the folder of the selected
    /// request), else the first collection.
    fn run_target_key(&self) -> Option<String> {
        let ws = self.scope.workspace.as_ref()?;
        let sel = self.scope.selected.clone().unwrap_or_default();
        let key = if sel.ends_with(".md") {
            sel.rsplit_once('/')
                .map(|(f, _)| f.to_string())
                .unwrap_or_default()
        } else {
            sel
        };
        if !key.is_empty() && self.collection_root_of(&key).is_some() {
            return Some(key);
        }
        ws.collections.first().map(|c| c.key.clone())
    }

    /// Open the Folder run tab for the selection (palette, Cmd+Shift+R).
    pub fn run_selected_folder(&mut self, cx: &mut Context<Self>) {
        match self.run_target_key() {
            Some(k) => self.open_run_tab(&k, cx),
            None => {
                self.notice = Some("Open a workspace to run a folder.".into());
                cx.notify();
            }
        }
    }

    pub fn open_run_tab(&mut self, folder: &str, cx: &mut Context<Self>) {
        let title = if folder.is_empty() {
            "Run".to_string()
        } else {
            format!("Run: {}", folder.rsplit('/').next().unwrap_or(folder))
        };
        self.open_tab(
            &format!("run/{folder}"),
            &title,
            crate::tabs::TabKind::FolderRun,
            true,
        );
        if let Some(id) = self.scope.tabs.active_id() {
            self.run_entry(id, cx);
        }
        cx.notify();
    }

    fn run_entry(&mut self, tab_id: u64, cx: &mut Context<Self>) -> &mut RunTab {
        if !self.scope.runs.contains_key(&tab_id) {
            let folder = self
                .scope
                .tabs
                .get(tab_id)
                .and_then(|t| t.key.strip_prefix("run/").map(String::from))
                .unwrap_or_default();
            self.scope
                .runs
                .insert(tab_id, RunTab::new(tab_id, folder, cx));
        }
        self.scope.runs.get_mut(&tab_id).expect("just inserted")
    }

    /// Start (or restart) the run in `tab_id` from its options.
    pub fn start_run(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let folder = {
            let r = self.run_entry(tab_id, cx);
            if r.running() {
                return;
            }
            r.folder.clone()
        };
        let Some(root) = self.collection_root_of(&folder) else {
            self.run_entry(tab_id, cx).error =
                Some("This folder is not inside a collection.".into());
            cx.notify();
            return;
        };
        let abs = self
            .scope
            .workspace
            .as_ref()
            .map(|p| p.abs_of(&folder))
            .unwrap_or_default();
        let rel = abs
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let coll = self
            .scope
            .workspace
            .as_ref()
            .and_then(|p| p.collections.iter().find(|c| c.root == root));
        let net = fluxwire_runner::net_config_from_dir(&root);
        let settings = coll
            .map(|c| fluxwire_runner::settings_from_defaults(&c.collection.config.defaults))
            .unwrap_or_default();
        let default_conc = coll.and_then(|c| c.collection.config.defaults.concurrency);
        let limits = coll
            .map(|c| {
                let s = &c.collection.config.scripts;
                let mut l = fluxwire_runner::Limits::default();
                if let Some(ms) = s.timeout_ms {
                    l.timeout_ms = ms;
                }
                if let Some(mb) = s.memory_mb {
                    l.memory_mb = mb as usize;
                }
                l
            })
            .unwrap_or_default();
        let trusted = self.collection_trusted(&root, cx);
        let env_state = self.env_state(&root);
        let globals: Vec<(String, String)> = self
            .scope
            .envs
            .settings
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let runtime = self.scope.envs.runtime.clone();
        let env = self.active_env_name();
        let env_overlay = self.dirty_env_overlay();
        let env_root = self.env_root();
        let allow_process_env = self.scope.envs.settings.allow_process_env;
        let dirty = self.scope.workspace.as_ref().map_or(0, |p| {
            let prefix = format!("{folder}/");
            p.dirty_keys()
                .iter()
                .filter(|k| folder.is_empty() || k.starts_with(&prefix))
                .count()
        });

        let r = self.run_entry(tab_id, cx);
        let data = match &r.data_file {
            Some(p) => match load_data_file(p) {
                Ok(d) => d,
                Err(e) => {
                    r.error = Some(e);
                    cx.notify();
                    return;
                }
            },
            None => Vec::new(),
        };
        let plan = RunPlan {
            collection_root: root.clone(),
            folder: rel,
            mode: if r.sequential {
                RunMode::Sequential
            } else {
                RunMode::Concurrent
            },
            concurrency: Some(if r.concurrency == 0 {
                default_conc.unwrap_or(fluxwire_runner::DEFAULT_CONCURRENCY)
            } else {
                r.concurrency
            }),
            iterations: r.iterations.max(1),
            data,
            filter: Some(r.filter.clone()).filter(|s| !s.trim().is_empty()),
            only: r.only.take(),
            bail: r.bail,
            delay_ms: 0,
            timeout_ms: None,
            env,
            env_state,
            settings,
            net,
            scripts: fluxwire_runner::ScriptOptions {
                trusted,
                limits,
                collection_root: Some(root.clone()),
                file: None,
                globals,
                allow_process_env,
                env_overlay,
                env_root,
            },
            runtime,
        };
        let handle = runner().run_folder(plan);
        r.seq += 1;
        let seq = r.seq;
        r.cancel = Some(handle.canceller());
        r.started = Some(Instant::now());
        r.run_id = Some(handle.run_id.clone());
        r.entries.clear();
        r.expanded.clear();
        r.flat = Arc::default();
        r.result = None;
        r.error = None;
        r.max_ms = 1;
        let run_id = handle.run_id.clone();
        if dirty > 0 {
            self.notice = Some(format!(
                "{dirty} unsaved file{} in this folder: runs use the saved files.",
                if dirty == 1 { "" } else { "s" }
            ));
        }
        let mut rx = handle.events;
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.recv().await {
                let done = matches!(ev, FolderRunEvent::Finished(_));
                if this
                    .update(cx, |this, cx| {
                        this.on_folder_event(tab_id, seq, &run_id, ev, cx)
                    })
                    .is_err()
                    || done
                {
                    break;
                }
            }
        })
        .detach();
        // Progress repaints at most every 100 ms however many requests finish.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        let live = this
                            .scope
                            .runs
                            .get(&tab_id)
                            .is_some_and(|r| r.seq == seq && r.running());
                        if live {
                            cx.notify();
                        }
                        live
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn cancel_run(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        if let Some(c) = self.scope.runs.get(&tab_id).and_then(|r| r.cancel.clone()) {
            c.cancel();
        }
        cx.notify();
    }

    fn on_folder_event(
        &mut self,
        tab_id: u64,
        seq: u64,
        run_id: &str,
        ev: FolderRunEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self.scope.runs.get_mut(&tab_id).filter(|r| r.seq == seq) else {
            return;
        };
        match ev {
            FolderRunEvent::Started {
                iterations,
                requests,
            } => {
                r.iterations_run = iterations;
                r.entries = (0..iterations)
                    .flat_map(|it| {
                        requests.iter().map(move |p| Entry {
                            iteration: it,
                            request: p.clone(),
                            slot: Slot::Pending,
                        })
                    })
                    .collect();
                r.rebuild_flat();
            }
            FolderRunEvent::RequestStarted { iteration, key } => {
                if let Some(e) = r
                    .entries
                    .iter_mut()
                    .find(|e| e.iteration == iteration && e.request.key == key)
                {
                    e.slot = Slot::Running(Instant::now());
                }
            }
            FolderRunEvent::RequestFinished(run) => {
                r.max_ms = r.max_ms.max(run.duration_ms);
                let fail = run.status() == RequestStatus::Failed;
                if let Some((i, e)) =
                    r.entries.iter_mut().enumerate().find(|(_, e)| {
                        e.iteration == run.iteration && e.request.key == run.request.key
                    })
                {
                    e.slot = Slot::Done(run.clone());
                    // failures open themselves so the assertion is visible
                    if fail {
                        r.expanded.insert(i);
                        r.rebuild_flat();
                    }
                }
                self.history_record_run(&run, run_id, cx);
            }
            FolderRunEvent::Finished(res) => {
                r.cancel = None;
                r.error = res.error.clone();
                r.result = Some(Arc::new(*res));
                r.rebuild_flat();
                let folder = self
                    .scope
                    .runs
                    .get(&tab_id)
                    .map(|r| r.folder.clone())
                    .unwrap_or_default();
                if let Some(root) = self.collection_root_of(&folder) {
                    self.persist_cookies(&root);
                }
                if let Some(res) = self.scope.runs.get(&tab_id).and_then(|r| r.result.clone()) {
                    let s = res.summary();
                    self.notice = Some(format!(
                        "Run finished: {} passed, {} failed, {} skipped in {} ms.",
                        s.passed, s.failed, s.skipped, res.total_ms
                    ));
                }
                self.history_reload(cx);
            }
        }
        cx.notify();
    }

    /// One history entry per request of a run, tagged with the run id (SPEC §12).
    fn history_record_run(&mut self, run: &RequestRun, run_id: &str, cx: &mut Context<Self>) {
        let res = &run.result;
        let (Some(store), Some(ws), Some(rec)) = (
            self.history.store.clone(),
            self.history.ws_key.clone(),
            res.history.as_ref(),
        ) else {
            return;
        };
        let collection = self
            .scope
            .workspace
            .as_ref()
            .and_then(|p| {
                let abs = p.abs_of(&run.request.key);
                p.collections
                    .iter()
                    .filter(|c| abs.starts_with(&c.root))
                    .max_by_key(|c| c.root.components().count())
                    .map(|c| c.key.clone())
            })
            .unwrap_or_default();
        let x = res.outcome.as_ref().and_then(|o| o.exchange.as_ref());
        let entry = NewEntry {
            ts_ms: res
                .started_at
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            collection,
            request_path: run.request.key.clone(),
            request_name: run.request.name.clone(),
            protocol: rec.protocol.clone(),
            method: run.request.method.clone(),
            url: res
                .outcome
                .as_ref()
                .map(|o| o.final_url.clone())
                .unwrap_or_else(|| res.label.clone()),
            status: x.map(|x| i64::from(x.status)),
            total_ms: res.outcome.as_ref().map_or(0.0, |o| o.total_ms),
            env: self.active_env_name(),
            dirty: false,
            run_id: Some(run_id.to_string()),
            summary: rec.summary.clone(),
            payload: rec.payload.clone(),
        };
        let red = Redactor::with_secrets(res.secrets.clone());
        let task = blocking(move || store.insert(&ws, entry, &red));
        cx.spawn(async move |_, _| {
            task.await;
        })
        .detach();
    }

    pub fn rerun_failed(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(keys) = self
            .scope
            .runs
            .get(&tab_id)
            .and_then(|r| r.result.as_ref())
            .map(|res| res.failed_keys())
        else {
            return;
        };
        if keys.is_empty() {
            self.notice = Some("No failed requests to re-run.".into());
            cx.notify();
            return;
        }
        let r = self.run_entry(tab_id, cx);
        r.only = Some(keys);
        self.start_run(tab_id, cx);
    }

    pub fn export_run(&mut self, tab_id: u64, junit: bool, cx: &mut Context<Self>) {
        let Some(res) = self.scope.runs.get(&tab_id).and_then(|r| r.result.clone()) else {
            return;
        };
        let (name, text) = if junit {
            ("fluxwire-run.xml", to_junit(&res))
        } else {
            ("fluxwire-run.json", to_json(&res))
        };
        let dir = self
            .scope
            .workspace
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some(name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
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
    }

    /// The equivalent `fluxwire run` invocation (SPEC §18).
    pub fn run_cli_command(&self, tab_id: u64) -> Option<String> {
        let r = self.scope.runs.get(&tab_id)?;
        let mut parts = vec!["fluxwire".to_string(), "run".into()];
        let path = if r.folder.is_empty() {
            ".".to_string()
        } else {
            r.folder.clone()
        };
        parts.push(shell_quote(&path));
        if let Some(e) = &self.active_env_name() {
            parts.push("--env".into());
            parts.push(shell_quote(e));
        }
        if r.sequential {
            parts.push("--sequential".into());
        } else if r.concurrency != fluxwire_runner::DEFAULT_CONCURRENCY {
            parts.push(format!("--concurrency {}", r.concurrency));
        }
        if let Some(d) = &r.data_file {
            parts.push("--data".into());
            let shown = self
                .scope
                .workspace
                .as_ref()
                .and_then(|p| d.strip_prefix(&p.root).ok())
                .unwrap_or(d);
            parts.push(shell_quote(&shown.to_string_lossy()));
        } else if r.iterations > 1 {
            parts.push(format!("--iterations {}", r.iterations));
        }
        if !r.filter.trim().is_empty() {
            parts.push("--filter".into());
            parts.push(shell_quote(r.filter.trim()));
        }
        if r.bail {
            parts.push("--bail".into());
        }
        Some(parts.join(" "))
    }

    pub fn copy_run_command(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        if let Some(cmd) = self.run_cli_command(tab_id) {
            cx.write_to_clipboard(ClipboardItem::new_string(cmd));
            self.notice = Some("Copied the equivalent fluxwire run command.".into());
            cx.notify();
        }
    }

    pub fn pick_run_data_file(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose data file (.csv or .json)".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(p) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| {
                this.run_entry(tab_id, cx).data_file = Some(p);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show only this run's requests in the History panel.
    pub fn show_run_in_history(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(id) = self.scope.runs.get(&tab_id).and_then(|r| r.run_id.clone()) else {
            return;
        };
        self.history.query.clear();
        self.history.query.nav = None;
        self.history.query.run = Some(id);
        self.scope.panel = Panel::History;
        self.scope.left_open = true;
        self.open_history_tab(cx);
        self.history_reload(cx);
    }

    pub fn on_run_filter_key(
        &mut self,
        tab_id: u64,
        ev: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let r = self.run_entry(tab_id, cx);
        if ev.keystroke.key == "escape" {
            r.filter.clear();
            let input = r.filter_input.clone();
            input.update(cx, |i, cx| i.set_text("", cx));
        } else if ev.keystroke.key == "enter" {
            self.start_run(tab_id, cx);
            return;
        } else {
            return;
        }
        cx.notify();
    }

    // ---- view -------------------------------------------------------------------------------

    pub(crate) fn render_run_tab(
        &self,
        t: &Theme,
        tab: &Tab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = tab.id;
        let folder = tab.key.strip_prefix("run/").unwrap_or("").to_string();
        let Some(r) = self.scope.runs.get(&id) else {
            // Restored tab: no run yet. Options materialise on first interaction.
            return self
                .page(t, &tab.title, "Folder run")
                .child(
                    row().gap_2().child(
                        button(t, "run-go", "Run", true)
                            .on_click(cx.listener(move |this, _, _, cx| this.start_run(id, cx))),
                    ),
                )
                .child(div().text_color(t.text_muted).child(format!(
                    "{} — press Run to start.",
                    if folder.is_empty() {
                        "collection"
                    } else {
                        &folder
                    }
                )))
                .into_any_element();
        };
        let chip = |key: String, label: String, on: bool| {
            div()
                .id(gpui::ElementId::Name(key.into()))
                .px_2()
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .cursor_pointer()
                .text_size(px(12.))
                .when(on, |d| d.bg(t.element_selected).text_color(t.text))
                .when(!on, |d| {
                    let h = t.element_hover;
                    d.text_color(t.text_muted).hover(move |s| s.bg(h))
                })
                .child(label)
        };
        let stepper =
            |key: &'static str, label: &str, value: u32, step_cx: &mut Context<Self>| {
                row()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(label.to_string()),
                    )
                    .child(chip(format!("{key}-dec"), "−".into(), false).on_click(
                        step_cx.listener(move |this, _, _, cx| {
                            let r = this.run_entry(id, cx);
                            match key {
                                "conc" => r.concurrency = r.concurrency.saturating_sub(1).max(1),
                                _ => r.iterations = r.iterations.saturating_sub(1).max(1),
                            }
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .w(px(24.))
                            .flex()
                            .justify_center()
                            .font_family(BUFFER_FONT)
                            .text_size(px(12.))
                            .child(value.to_string()),
                    )
                    .child(chip(format!("{key}-inc"), "+".into(), false).on_click(
                        step_cx.listener(move |this, _, _, cx| {
                            let r = this.run_entry(id, cx);
                            match key {
                                "conc" => r.concurrency = (r.concurrency + 1).min(256),
                                _ => r.iterations = (r.iterations + 1).min(10_000),
                            }
                            cx.notify();
                        }),
                    ))
            };
        let running = r.running();
        let (p, f, s, pending) = r.counts();
        let total = r.entries.len();
        let elapsed = r
            .result
            .as_ref()
            .map(|x| x.total_ms)
            .or_else(|| r.started.map(|s| s.elapsed().as_millis() as u64))
            .unwrap_or(0);

        let filter_box = row()
            .id("run-filter")
            .track_focus(&r.focus)
            .on_key_down(cx.listener(move |this, ev, _, cx| this.on_run_filter_key(id, ev, cx)))
            .h(px(22.))
            .w(px(180.))
            .px_2()
            .rounded(px(4.))
            .border_1()
            .border_color(t.border)
            .focus({
                let a = t.accent;
                move |s| s.border_color(a)
            })
            .font_family(BUFFER_FONT)
            .text_size(px(12.))
            .overflow_hidden()
            .child(r.filter_input.clone());

        let data_label = r
            .data_file
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| format!("Data: {}", n.to_string_lossy()))
            .unwrap_or_else(|| "Data: none".into());

        let options = row()
            .flex_wrap()
            .gap_2()
            .child(
                chip(
                    "run-mode".into(),
                    if r.sequential {
                        "Sequential"
                    } else {
                        "Concurrent"
                    }
                    .into(),
                    r.sequential,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let r = this.run_entry(id, cx);
                    r.sequential = !r.sequential;
                    cx.notify();
                })),
            )
            .when(!r.sequential, |d| {
                d.child(stepper("conc", "Concurrency", r.concurrency, cx))
            })
            .when(r.data_file.is_none(), |d| {
                d.child(stepper("iter", "Iterations", r.iterations, cx))
            })
            .child(
                chip(
                    "run-env".into(),
                    format!("Env: {}", self.env_label()),
                    false,
                )
                .on_click(cx.listener(|this, _, w, cx| this.open_env_picker(w, cx))),
            )
            .child(
                chip("run-data".into(), data_label, r.data_file.is_some()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        if this.run_entry(id, cx).data_file.is_some() {
                            this.run_entry(id, cx).data_file = None;
                            cx.notify();
                        } else {
                            this.pick_run_data_file(id, cx);
                        }
                    },
                )),
            )
            .child(filter_box)
            .child(
                chip("run-bail".into(), "Bail".into(), r.bail).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let r = this.run_entry(id, cx);
                        r.bail = !r.bail;
                        cx.notify();
                    },
                )),
            );

        let actions = row()
            .flex_wrap()
            .gap_2()
            .child(if running {
                button(t, "run-cancel", "Cancel", true)
                    .on_click(cx.listener(move |this, _, _, cx| this.cancel_run(id, cx)))
            } else {
                button(t, "run-go", "Run", true).on_click(cx.listener(move |this, _, _, cx| {
                    this.run_entry(id, cx).only = None;
                    this.start_run(id, cx)
                }))
            })
            .child(
                button(t, "run-failed", "Re-run failed", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.rerun_failed(id, cx))),
            )
            .child(
                button(t, "run-json", "Export JSON", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.export_run(id, false, cx))),
            )
            .child(
                button(t, "run-junit", "Export JUnit", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.export_run(id, true, cx))),
            )
            .child(
                button(t, "run-cli", "Copy CLI command", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.copy_run_command(id, cx))),
            )
            .child(
                button(t, "run-history", "View in History", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.show_run_in_history(id, cx))),
            );

        let summary = row()
            .gap_3()
            .text_size(px(12.))
            .child(div().text_color(t.created).child(format!("{p} passed")))
            .child(div().text_color(t.deleted).child(format!("{f} failed")))
            .child(div().text_color(t.text_muted).child(format!("{s} skipped")))
            .when(running, |d| {
                d.child(
                    div()
                        .text_color(t.text_muted)
                        .child(format!("{} of {total} done", total - pending)),
                )
            })
            .child(
                div()
                    .font_family(BUFFER_FONT)
                    .text_color(t.text_muted)
                    .child(format!("{elapsed} ms")),
            )
            .when_some(r.result.as_ref(), |d, res| {
                d.when(res.cancelled, |d| {
                    d.child(div().text_color(t.modified).child("cancelled"))
                })
                .when(res.timed_out, |d| {
                    d.child(div().text_color(t.modified).child("timed out"))
                })
                .when(res.bailed, |d| {
                    d.child(
                        div()
                            .text_color(t.modified)
                            .child("stopped after a failure"),
                    )
                })
            });

        let n = r.flat.len();
        let list = if let Some(e) = &r.error {
            div()
                .p_3()
                .text_color(t.deleted)
                .child(e.clone())
                .into_any_element()
        } else if n == 0 {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(t.text_placeholder)
                .child(if running {
                    "Selecting requests…"
                } else {
                    "Press Run to run this folder."
                })
                .into_any_element()
        } else {
            uniform_list(
                "run-list",
                n,
                cx.processor(
                    move |this: &mut WorkspaceView, range: std::ops::Range<usize>, _w, cx| {
                        let t = crate::theme::theme(cx).clone();
                        let Some(r) = this.scope.runs.get(&id) else {
                            return Vec::new();
                        };
                        range
                            .filter_map(|i| r.flat.get(i).cloned().map(|row| (i, row)))
                            .map(|(i, row)| this.run_row(&t, id, i, row, cx))
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(r.scroll.clone())
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!(
                        "Run: {}",
                        if folder.is_empty() {
                            "collection"
                        } else {
                            &folder
                        }
                    )),
            )
            .child(options)
            .child(actions)
            .child(summary)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(t.border_variant)
                    .rounded(px(4.))
                    .child(list),
            )
            .into_any_element()
    }

    fn run_row(
        &self,
        t: &Theme,
        tab_id: u64,
        i: usize,
        row_: RunRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(r) = self.scope.runs.get(&tab_id) else {
            return div().into_any_element();
        };
        match row_ {
            RunRow::Iteration(n) => div()
                .id(("run-iter", i))
                .h(px(24.))
                .px_2()
                .flex()
                .items_center()
                .bg(t.element_hover)
                .text_size(px(11.))
                .text_color(t.text_muted)
                .child(
                    match r
                        .result
                        .as_ref()
                        .and_then(|x| x.rows.get(n as usize))
                        .filter(|m| !m.is_empty())
                    {
                        Some(m) => format!(
                            "Iteration {} — {}",
                            n + 1,
                            m.iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        None => format!("Iteration {}", n + 1),
                    },
                )
                .into_any_element(),
            RunRow::Detail { ok, text } => div()
                .id(("run-detail", i))
                .px_2()
                .pl(px(40.))
                .py_0p5()
                .font_family(BUFFER_FONT)
                .text_size(px(11.))
                .text_color(if ok { t.text_muted } else { t.deleted })
                .child(format!("{} {text}", if ok { "✓" } else { "✗" }))
                .into_any_element(),
            RunRow::Open(e) => {
                let key = r.entries[e].request.key.clone();
                div()
                    .id(("run-open", i))
                    .pl(px(40.))
                    .py_0p5()
                    .text_size(px(11.))
                    .text_color(t.accent)
                    .cursor_pointer()
                    .child("Open request")
                    .on_click(
                        cx.listener(move |this, _, w, cx| this.open_request(&key, true, w, cx)),
                    )
                    .into_any_element()
            }
            RunRow::Request(e) => {
                let ent = &r.entries[e];
                let (glyph, color) = match &ent.slot {
                    Slot::Pending => ("·", t.text_placeholder),
                    Slot::Running(_) => ("…", t.accent),
                    Slot::Done(run) => match run.status() {
                        RequestStatus::Passed => ("✓", t.created),
                        RequestStatus::Failed => ("✗", t.deleted),
                        RequestStatus::Skipped => ("–", t.text_muted),
                    },
                };
                let (ms, bar_start, bar_len, bar_color) = match &ent.slot {
                    Slot::Done(run) => {
                        let total = r
                            .result
                            .as_ref()
                            .map(|x| x.total_ms)
                            .unwrap_or_else(|| {
                                r.started.map_or(1, |s| s.elapsed().as_millis() as u64)
                            })
                            .max(1);
                        (
                            Some(run.duration_ms),
                            run.started_ms as f32 / total as f32,
                            (run.duration_ms as f32 / total as f32).max(0.004),
                            if run.status() == RequestStatus::Failed {
                                t.deleted
                            } else {
                                t.created
                            },
                        )
                    }
                    _ => (None, 0.0, 0.0, t.created),
                };
                let tests = match &ent.slot {
                    Slot::Done(run) if !run.result.tests.is_empty() => {
                        let n = run.result.tests.len();
                        let bad = run.result.tests.iter().filter(|x| !x.passed).count();
                        format!("{}/{n}", n - bad)
                    }
                    _ => String::new(),
                };
                let expandable = matches!(ent.slot, Slot::Done(_));
                row()
                    .id(("run-row", i))
                    .h(px(26.))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .when(expandable, |d| {
                        let h = t.element_hover;
                        d.cursor_pointer().hover(move |s| s.bg(h))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(r) = this.scope.runs.get_mut(&tab_id)
                            && matches!(r.entries[e].slot, Slot::Done(_))
                        {
                            if !r.expanded.remove(&e) {
                                r.expanded.insert(e);
                            }
                            r.rebuild_flat();
                            cx.notify();
                        }
                    }))
                    .child(div().w(px(14.)).text_color(color).child(glyph))
                    .child(
                        div()
                            .w(px(46.))
                            .font_family(BUFFER_FONT)
                            .text_size(px(11.))
                            .text_color(t.method_color(&ent.request.method))
                            .child(ent.request.method.clone()),
                    )
                    .child(
                        div()
                            .w(px(180.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .child(ent.request.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .overflow_hidden()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(ent.request.folder.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(6.))
                            .relative()
                            .bg(t.element_hover)
                            .rounded(px(2.))
                            .when(ms.is_some(), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .h_full()
                                        .left(relative(bar_start.min(0.99)))
                                        .w(relative(bar_len.min(1.0 - bar_start.min(0.99))))
                                        .rounded(px(2.))
                                        .bg(bar_color),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w(px(36.))
                            .text_size(px(11.))
                            .text_color(t.text_muted)
                            .child(tests),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .flex()
                            .justify_end()
                            .font_family(BUFFER_FONT)
                            .text_size(px(11.))
                            .text_color(t.text_muted)
                            .child(ms.map(|m| format!("{m} ms")).unwrap_or_default()),
                    )
                    .into_any_element()
            }
        }
    }
}
