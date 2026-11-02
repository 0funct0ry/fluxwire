//! Sending a request and showing the response (SPEC §17.4): per-tab run state, the runner
//! bridge, cURL copy, and the response pane with a virtualized body viewer.

use crate::dev_id::{DevClick, DevTag, Role};
use std::{
    collections::BTreeSet,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use fluxwire_runner::{
    CancellationToken, PhaseTiming, RunEvent, RunResult, SendOptions, SessionHandle,
    net_config_from_dir, runner, settings_from_defaults,
};
use gpui::{
    AnyElement, ClipboardItem, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, div, img, prelude::*, px,
    relative, uniform_list,
};

use crate::{
    assets::BUFFER_FONT,
    editor::tok_color,
    highlight::Tok,
    theme::Theme,
    ui::{icon, row},
    viewer::{
        BodyKind, BodyView, ImageFormat, LineText, filter_json, lex_json_line, visible_lines,
    },
    workspace::{TabUi, WorkspaceView},
};

pub const ROW_HEIGHT: f32 = 20.0;
/// Characters shown per row; longer lines are cut (use Wrap for minified bodies).
const MAX_LINE_CHARS: usize = 4000;
const WRAP_COLUMNS: usize = 140;
const CONSOLE_CAP: usize = 500;

const RESP_TABS: [&str; 6] = ["Body", "Headers", "Cookies", "Timeline", "Tests", "Console"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BodyMode {
    Pretty,
    Raw,
    Preview,
    Hex,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewerInput {
    Filter,
    Find,
}

pub struct Sending {
    pub started: Instant,
    pub phases: Vec<PhaseTiming>,
    pub redirects: usize,
    pub cancel: CancellationToken,
    /// Send frames into, or close, a live session (WebSocket and later protocols).
    pub session: SessionHandle,
}

pub struct TabResponse {
    pub seq: u64,
    pub sending: Option<Sending>,
    pub result: Option<Arc<RunResult>>,
    /// History entry whose stored response is shown instead of the latest send.
    pub viewing: Option<i64>,
    /// The latest send, kept while an earlier response is shown.
    pub latest: Option<Arc<RunResult>>,
    pub view: Option<Arc<BodyView>>,
    pub mode: BodyMode,
    pub wrap: bool,
    pub collapsed: BTreeSet<usize>,
    pub filter: String,
    pub filter_input: crate::text_input::LazyInput,
    pub filter_error: Option<String>,
    pub filtered: Option<Arc<LineText>>,
    pub find: String,
    pub find_input: crate::text_input::LazyInput,
    pub matches: Vec<u32>,
    pub match_cursor: usize,
    /// What the list shows right now (mode, filter and wrap applied).
    pub shown: Option<Arc<LineText>>,
    /// Row -> line index when folds are collapsed.
    pub visible: Option<Arc<Vec<u32>>>,
    pub scroll: UniformListScrollHandle,
    /// Frames of a session protocol run.
    pub log: crate::session_log::SessionLog,
}

impl TabResponse {
    fn new(seq: u64) -> Self {
        Self {
            seq,
            sending: None,
            result: None,
            viewing: None,
            latest: None,
            view: None,
            mode: BodyMode::Pretty,
            wrap: false,
            collapsed: BTreeSet::new(),
            filter: String::new(),
            filter_input: Default::default(),
            filter_error: None,
            filtered: None,
            find: String::new(),
            find_input: Default::default(),
            matches: Vec::new(),
            match_cursor: 0,
            shown: None,
            visible: None,
            scroll: UniformListScrollHandle::new(),
            log: crate::session_log::SessionLog::default(),
        }
    }
}

pub fn fmt_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f < KB {
        format!("{n} B")
    } else if f < KB * KB {
        format!("{:.1} KB", f / KB)
    } else if f < KB * KB * KB {
        format!("{:.1} MB", f / (KB * KB))
    } else {
        format!("{:.2} GB", f / (KB * KB * KB))
    }
}

fn fmt_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// `fw.env.persist`: write `name=value` to the active environment's `.local.env`.
fn persist_env_value(
    root: &std::path::Path,
    env: &str,
    name: &str,
    value: &str,
) -> std::io::Result<()> {
    let secrets = fluxwire_format::load_config(root)
        .map(|c| c.secrets.names)
        .unwrap_or_default();
    let mut e = fluxwire_format::load_environment(root, env, &secrets);
    match e.rows.iter_mut().find(|r| r.name == name && r.local) {
        Some(r) => r.value = value.to_string(),
        None => e.rows.push(fluxwire_format::EnvRow {
            name: name.to_string(),
            value: value.to_string(),
            secret: true,
            local: true,
            line: 0,
        }),
    }
    fluxwire_format::save_environment(root, &e)
}

fn clock(t: SystemTime) -> String {
    let s = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn parse_set_cookie(v: &str) -> (String, String, String) {
    let mut parts = v.split(';').map(str::trim);
    let (name, value) = parts
        .next()
        .and_then(|p| p.split_once('='))
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .unwrap_or_default();
    (name, value, parts.collect::<Vec<_>>().join("; "))
}

fn extension_for(kind: BodyKind) -> &'static str {
    match kind {
        BodyKind::Json => "json",
        BodyKind::Xml => "xml",
        BodyKind::Html => "html",
        BodyKind::Text | BodyKind::Empty => "txt",
        BodyKind::Image(ImageFormat::Png) => "png",
        BodyKind::Image(ImageFormat::Jpeg) => "jpg",
        BodyKind::Image(ImageFormat::Gif) => "gif",
        BodyKind::Image(ImageFormat::Webp) => "webp",
        BodyKind::Image(ImageFormat::Svg) => "svg",
        BodyKind::Pdf => "pdf",
        BodyKind::Binary => "bin",
    }
}

impl WorkspaceView {
    /// Cookies of the active environment's jar that apply to `url`'s host, as table rows.
    fn jar_rows(&self, url: &str) -> Vec<(String, String)> {
        let host = url
            .split("://")
            .nth(1)
            .and_then(|r| r.split(['/', '?', '#']).next())
            .map(|a| a.rsplit('@').next().unwrap_or(a))
            .map(|a| {
                a.rsplit_once(':')
                    .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
                    .map_or(a, |(h, _)| h)
            })
            .unwrap_or("")
            .to_ascii_lowercase();
        let jar = (|| {
            let key = self.active_key()?;
            let root = self.collection_root_of(&key)?;
            let env = self.active_env_name()?;
            Some(self.scope.envs.states.get(&(root, env))?.cookies.list())
        })()
        .unwrap_or_default();
        jar.into_iter()
            .filter(|c| {
                if c.host_only {
                    host == c.domain
                } else {
                    host == c.domain || host.ends_with(&format!(".{}", c.domain))
                }
            })
            .map(|c| (format!("{} · jar", c.name), c.value))
            .collect()
    }

    // ---- sending ----------------------------------------------------------------------------

    pub fn is_sending(&self) -> bool {
        self.scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get(&id))
            .is_some_and(|r| r.sending.is_some())
    }

    /// Send the active request from its in-memory document (saving is never required).
    pub fn send_request(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.scope.tabs.active_id() else {
            return;
        };
        let Some(key) = self.active_key() else { return };
        if self.is_sending() {
            return;
        }
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        let Some(doc) = ws.doc(&key) else { return };
        let parsed = doc.parsed.clone();
        let abs = ws.abs_of(&key);
        let coll = ws
            .collections
            .iter()
            .filter(|c| abs.starts_with(&c.root))
            .max_by_key(|c| c.root.components().count());
        let net = coll
            .map(|c| net_config_from_dir(&c.root))
            .unwrap_or_default();
        let settings = coll
            .map(|c| settings_from_defaults(&c.collection.config.defaults))
            .unwrap_or_default();
        let root = coll.map(|c| c.root.clone());
        let Some(scope) = self.scope_for(&key) else {
            return;
        };
        // Unresolved variables warn once per tab; sending again goes ahead (SPEC §8.4).
        let unresolved = self.unresolved_in(&key);
        if !unresolved.is_empty() && self.scope.envs.warned.insert(tab_id) {
            self.notice = Some(format!(
                "Unresolved variable{}: {}. Send again to send anyway.",
                if unresolved.len() == 1 { "" } else { "s" },
                unresolved
                    .iter()
                    .map(|v| format!("{{{{{v}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            cx.notify();
            return;
        }
        let limits = coll
            .map(|c| fluxwire_runner::prepare::limits_from(&c.collection.config))
            .unwrap_or_default();

        let env = root.as_deref().and_then(|r| self.env_state(r));

        // Scripts run only in trusted collections (SPEC §21); the first send asks.
        let has_scripts = fluxwire_runner::prepare::has_scripts(&parsed, root.as_deref(), &abs);
        let trusted = root
            .as_deref()
            .is_some_and(|r| self.collection_trusted(r, cx));
        if has_scripts && !trusted {
            self.scope.trust_prompt = Some(tab_id);
            cx.notify();
            return;
        }
        self.scope.trust_prompt = None;
        let handle = runner().send(
            &parsed,
            SendOptions {
                scope: (*scope).clone(),
                settings,
                net,
                env,
                scripts: fluxwire_runner::ScriptOptions {
                    trusted,
                    limits,
                    collection_root: root.clone(),
                    file: Some(abs.clone()),
                    globals: self
                        .scope
                        .envs
                        .settings
                        .globals
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    allow_process_env: self.scope.envs.settings.allow_process_env,
                    env_overlay: self.dirty_env_overlay(),
                    env_root: self.env_root(),
                },
            },
        );
        let seq = self.scope.responses.get(&tab_id).map_or(0, |r| r.seq) + 1;
        let mut resp = TabResponse::new(seq);
        // keep viewer preferences across sends
        if let Some(old) = self.scope.responses.get(&tab_id) {
            resp.wrap = old.wrap;
        }
        resp.sending = Some(Sending {
            started: Instant::now(),
            phases: Vec::new(),
            redirects: 0,
            cancel: handle.canceller(),
            session: handle.session(),
        });
        self.scope.responses.insert(tab_id, resp);
        if let Some(ui) = self.scope.tab_ui.get_mut(&tab_id) {
            ui.resp = 0;
        }

        let mut rx = handle.events;
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.recv().await {
                let done = matches!(ev, RunEvent::Finished(_));
                if this
                    .update(cx, |this, cx| this.on_run_event(tab_id, seq, ev, cx))
                    .is_err()
                    || done
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let alive = this
                    .update(cx, |this, cx| {
                        let live = this
                            .scope
                            .responses
                            .get(&tab_id)
                            .is_some_and(|r| r.seq == seq && r.sending.is_some());
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

    /// Whether scripts may run in the collection at `root` (SPEC §21).
    pub fn collection_trusted(&self, root: &std::path::Path, cx: &Context<Self>) -> bool {
        if self.scope.session_trust.contains(root) {
            return true;
        }
        cx.try_global::<crate::envs::ConfigDir>()
            .is_some_and(|d| fluxwire_format::is_trusted(&d.0, root))
    }

    /// Write `.fluxwire/types/fw.d.ts` for every open collection (SPEC §11.3). `announce`
    /// reports the result in the status bar (the palette action); opening stays quiet.
    pub fn regenerate_fw_types(&mut self, announce: bool, cx: &mut Context<Self>) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        let mut written = 0;
        for c in &ws.collections {
            let dir = c.root.join(".fluxwire").join("types");
            let path = dir.join("fw.d.ts");
            let want = fluxwire_runner::fw_dts();
            if std::fs::read_to_string(&path).is_ok_and(|have| have == want) {
                continue;
            }
            if std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&path, want))
                .is_ok()
            {
                written += 1;
            }
        }
        if announce {
            self.notice = Some(if written == 0 {
                "fw.d.ts is up to date.".into()
            } else {
                format!("Wrote fw.d.ts for {written} collection(s) under .fluxwire/types/.")
            });
        }
        cx.notify();
    }

    fn active_collection_root(&self) -> Option<std::path::PathBuf> {
        let key = self.active_key()?;
        self.collection_root_of(&key)
    }

    /// Trust (or revoke) the scripts of the active request's collection, then resume a
    /// send that was waiting on the prompt.
    pub fn set_collection_trust(&mut self, trusted: bool, cx: &mut Context<Self>) {
        let Some(root) = self.active_collection_root() else {
            return;
        };
        let stored = cx
            .try_global::<crate::envs::ConfigDir>()
            .map(|d| fluxwire_format::set_trusted(&d.0, &root, trusted));
        match stored {
            Some(Err(e)) => {
                self.notice = Some(format!("Could not save the trust decision: {e}"));
            }
            Some(Ok(())) => {}
            None => {
                if trusted {
                    self.scope.session_trust.insert(root.clone());
                }
            }
        }
        if !trusted {
            self.scope.session_trust.remove(&root);
        }
        let resume = trusted && self.scope.trust_prompt.take().is_some();
        self.notice = Some(if trusted {
            "Scripts in this collection are trusted.".into()
        } else {
            "Scripts in this collection are no longer trusted.".into()
        });
        cx.notify();
        if resume {
            self.send_request(cx);
        }
    }

    /// Fold what a send's scripts and captures did into the workspace: runtime variables,
    /// persisted environment values, the Console and Problems.
    fn absorb_script_results(&mut self, tab_id: u64, res: &RunResult, cx: &mut Context<Self>) {
        let root = self
            .scope
            .tabs
            .all()
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| self.collection_root_of(&t.key));
        if !res.var_changes.is_empty() {
            for c in &res.var_changes {
                match &c.value {
                    Some(v) => {
                        self.scope
                            .envs
                            .runtime
                            .insert(c.name.clone(), (v.clone(), c.secret));
                    }
                    None => {
                        self.scope.envs.runtime.remove(&c.name);
                    }
                }
            }
            self.scope.envs.invalidate();
        }
        if let (Some(root), Some(env)) = (&root, self.active_env_name()) {
            for (name, value) in &res.env_persist {
                if let Err(e) = persist_env_value(root, &env, name, value) {
                    self.notice = Some(format!("Could not write {name} to {env}.local.env: {e}"));
                }
            }
            if !res.env_persist.is_empty() {
                self.scope.envs.invalidate();
            }
        }
        for l in &res.console {
            let line = format!("[{}] {}", log_glyph(l.level), l.text);
            self.log(line);
        }
        for e in res.script_errors.iter().chain(match &res.error {
            Some(fluxwire_runner::RunError::Script(es)) => es.iter(),
            _ => [].iter(),
        }) {
            let line = format!("✗ {} {} ({})", e.kind.code(), e.message, e.location());
            self.log(res.redact(&line));
        }
        if let Some(reason) = &res.skipped {
            self.log(format!("↷ skipped by a script: {reason}"));
        }
        cx.notify();
    }

    /// Cancel the active request if one is in flight. Returns whether there was one.
    pub fn cancel_request(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(r) = self
            .scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get(&id))
            .and_then(|r| r.sending.as_ref())
        else {
            return false;
        };
        r.cancel.cancel();
        cx.notify();
        true
    }

    pub fn send_or_cancel(&mut self, cx: &mut Context<Self>) {
        if !self.cancel_request(cx) {
            self.send_request(cx);
        }
    }

    fn log(&mut self, line: String) {
        #[cfg(feature = "dev-mcp")]
        crate::dev_events::console(&line);
        self.scope
            .console
            .push(format!("{}  {line}", clock(SystemTime::now())));
        if self.scope.console.len() > CONSOLE_CAP {
            self.scope
                .console
                .drain(..self.scope.console.len() - CONSOLE_CAP);
        }
    }

    fn on_run_event(&mut self, tab_id: u64, seq: u64, ev: RunEvent, cx: &mut Context<Self>) {
        let Some(r) = self
            .scope
            .responses
            .get_mut(&tab_id)
            .filter(|r| r.seq == seq)
        else {
            return;
        };
        match ev {
            RunEvent::Phase(p) => {
                if let Some(s) = &mut r.sending {
                    s.phases.push(p);
                }
            }
            RunEvent::Redirect(h) => {
                if let Some(s) = &mut r.sending {
                    s.redirects += 1;
                }
                self.log(format!("↪ {} {} → {}", h.status, h.from, h.to));
            }
            RunEvent::Frame(f) => {
                #[cfg(feature = "dev-mcp")]
                crate::dev_events::frame(tab_id, &format!("{f:?}"));
                r.log.push(f)
            }
            RunEvent::Finished(res) => self.finish_run(tab_id, seq, *res, cx),
        }
        cx.notify();
    }

    fn finish_run(&mut self, tab_id: u64, seq: u64, res: RunResult, cx: &mut Context<Self>) {
        for w in &res.warnings {
            let line = res.redact(&format!("⚠ {}", w.message));
            self.log(line);
        }
        let line = match (&res.outcome, &res.error) {
            (Some(o), _) => {
                let x = o.exchange.as_ref();
                format!(
                    "{} → {} {} ({:.0} ms)",
                    res.label,
                    x.map_or(0, |x| x.status),
                    x.map(|x| x.status_text.as_str()).unwrap_or(""),
                    o.total_ms
                )
            }
            (None, Some(e)) => {
                let (m, fix) = e.describe();
                format!(
                    "{} ✗ {} phase: {m}{}",
                    res.label,
                    e.phase(),
                    fix.map(|f| format!(" {f}")).unwrap_or_default()
                )
            }
            _ => res.label.clone(),
        };
        // Secrets never reach the Console (SPEC §8.3).
        self.log(res.redact(&line));
        self.absorb_script_results(tab_id, &res, cx);
        self.history_record(tab_id, &res, cx);
        if let Some(root) = self
            .scope
            .tabs
            .all()
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| self.collection_root_of(&t.key))
        {
            self.persist_cookies(&root);
        }
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        r.sending = None;
        r.viewing = None;
        r.latest = None;
        r.result = Some(Arc::new(res));
        self.build_body_view(tab_id, seq, cx);
    }

    /// Build the body view for the tab's current result in the background.
    fn build_body_view(&mut self, tab_id: u64, seq: u64, cx: &mut Context<Self>) {
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        let build = r
            .result
            .as_ref()
            .and_then(|res| res.outcome.as_ref())
            .and_then(|o| o.exchange.as_ref())
            .map(|x| (x.body.clone(), x.content_type.clone(), x.decoded_size));
        r.view = None;
        r.shown = None;
        let Some((body, ct, size)) = build else {
            return;
        };
        let task = cx.background_executor().spawn(async move {
            let bytes = body.read_all().unwrap_or_default();
            BodyView::build(bytes, size, ct.as_deref())
        });
        cx.spawn(async move |this, cx| {
            let view = task.await;
            this.update(cx, |this, cx| {
                if let Some(r) = this
                    .scope
                    .responses
                    .get_mut(&tab_id)
                    .filter(|r| r.seq == seq)
                {
                    r.mode = match view.kind {
                        BodyKind::Image(_) => BodyMode::Preview,
                        BodyKind::Binary | BodyKind::Pdf => BodyMode::Hex,
                        _ => BodyMode::Pretty,
                    };
                    if view.too_large {
                        r.mode = BodyMode::Hex;
                    }
                    r.view = Some(Arc::new(view));
                    this.refresh_shown(tab_id);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Show a stored history response in the tab's response pane (never re-recorded).
    pub fn show_stored_result(
        &mut self,
        tab_id: u64,
        e: &fluxwire_store::Entry,
        cx: &mut Context<Self>,
    ) {
        let Some(outcome) = runner().history_outcome(&crate::history::record_of(e)) else {
            self.notice = Some("That send failed, so it has no response to show.".into());
            cx.notify();
            return;
        };
        let res = RunResult {
            label: e.summary.summary.clone(),
            started_at: SystemTime::UNIX_EPOCH
                + Duration::from_millis(e.summary.ts_ms.max(0) as u64),
            phases: outcome.timings.clone(),
            outcome: Some(Box::new(outcome)),
            error: None,
            tests: Vec::new(),
            history: None,
            warnings: Vec::new(),
            secrets: Vec::new(),
            console: Vec::new(),
            var_changes: Vec::new(),
            env_persist: Vec::new(),
            script_errors: Vec::new(),
            skipped: None,
        };
        let resp = self
            .scope
            .responses
            .entry(tab_id)
            .or_insert_with(|| TabResponse::new(0));
        if resp.viewing.is_none() {
            resp.latest = resp.result.take();
        }
        resp.viewing = Some(e.summary.id);
        resp.seq += 1;
        resp.result = Some(Arc::new(res));
        let seq = resp.seq;
        self.build_body_view(tab_id, seq, cx);
        cx.notify();
    }

    pub fn history_back_to_latest(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.scope.tabs.active_id() else {
            return;
        };
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        if r.viewing.take().is_none() {
            return;
        }
        r.result = r.latest.take();
        r.seq += 1;
        let seq = r.seq;
        self.build_body_view(tab_id, seq, cx);
        cx.notify();
    }

    // ---- viewer state -----------------------------------------------------------------------

    /// Recompute the text the list shows after the mode, wrap, filter or view changed.
    pub fn refresh_shown(&mut self, tab_id: u64) {
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        let Some(view) = r.view.clone() else {
            r.shown = None;
            return;
        };
        let base: Option<Arc<LineText>> = match r.mode {
            BodyMode::Pretty => r.filtered.clone().or_else(|| view.pretty.clone()),
            BodyMode::Raw => view.raw.clone(),
            BodyMode::Hex => view.hex.clone(),
            BodyMode::Preview => None,
        };
        r.shown = base.map(|b| {
            if r.wrap {
                Arc::new(b.wrapped(WRAP_COLUMNS))
            } else {
                b
            }
        });
        let foldable =
            r.mode == BodyMode::Pretty && !r.wrap && r.filtered.is_none() && !view.folds.is_empty();
        r.visible = match (&r.shown, foldable && !r.collapsed.is_empty()) {
            (Some(s), true) => Some(Arc::new(visible_lines(s.len(), &view.folds, &r.collapsed))),
            _ => None,
        };
        self.refresh_find(tab_id);
    }

    fn refresh_find(&mut self, tab_id: u64) {
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        r.matches.clear();
        r.match_cursor = 0;
        let q = r.find.to_lowercase();
        let Some(shown) = r.shown.clone().filter(|_| q.len() >= 2) else {
            return;
        };
        for i in 0..shown.len() {
            if shown.line(i).to_lowercase().contains(&q) {
                r.matches.push(i as u32);
                if r.matches.len() >= 10_000 {
                    break;
                }
            }
        }
    }

    pub fn set_body_mode(&mut self, mode: BodyMode, cx: &mut Context<Self>) {
        let Some(id) = self.scope.tabs.active_id() else {
            return;
        };
        if let Some(r) = self.scope.responses.get_mut(&id) {
            r.mode = mode;
        }
        self.refresh_shown(id);
        cx.notify();
    }

    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.scope.tabs.active_id() else {
            return;
        };
        if let Some(r) = self.scope.responses.get_mut(&id) {
            r.wrap = !r.wrap;
        }
        self.refresh_shown(id);
        cx.notify();
    }

    fn toggle_fold(&mut self, tab_id: u64, line: usize, cx: &mut Context<Self>) {
        if let Some(r) = self.scope.responses.get_mut(&tab_id)
            && !r.collapsed.remove(&line)
        {
            r.collapsed.insert(line);
        }
        self.refresh_shown(tab_id);
        cx.notify();
    }

    fn jump_to_match(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self
            .scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get_mut(&id))
        else {
            return;
        };
        if r.matches.is_empty() {
            return;
        }
        r.match_cursor = (r.match_cursor + 1) % r.matches.len();
        let line = r.matches[r.match_cursor];
        let row = r
            .visible
            .as_ref()
            .and_then(|v| v.iter().position(|l| *l == line))
            .unwrap_or(line as usize);
        r.scroll.scroll_to_item(row, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    /// What a change in the filter or find box does (typing is handled by the input itself).
    pub(crate) fn viewer_on_change(
        id: u64,
        which: ViewerInput,
    ) -> impl Fn(&mut Self, String, &mut Context<Self>) + 'static {
        move |this, text, cx| {
            let Some(r) = this.scope.responses.get_mut(&id) else {
                return;
            };
            match which {
                ViewerInput::Find => {
                    r.find = text;
                    this.refresh_find(id);
                }
                ViewerInput::Filter => {
                    let empty = text.is_empty();
                    r.filter = text;
                    if empty {
                        this.apply_filter(id, cx);
                    }
                }
            }
            cx.notify();
        }
    }

    /// The tab's filter or find input, created on first use.
    pub fn viewer_input(
        &mut self,
        id: u64,
        which: ViewerInput,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<crate::text_input::TextInput>> {
        let r = self.scope.responses.get(&id)?;
        let (slot, placeholder) = match which {
            ViewerInput::Filter => (&r.filter_input, "JSONPath  $.items[*].id"),
            ViewerInput::Find => (&r.find_input, "Find"),
        };
        Some(crate::text_input::lazy_input(
            slot,
            placeholder,
            cx,
            Self::viewer_on_change(id, which),
        ))
    }

    /// Enter and Escape in the filter or find box.
    pub fn on_viewer_key(
        &mut self,
        which: ViewerInput,
        ev: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.scope.tabs.active_id() else {
            return;
        };
        match ev.keystroke.key.as_str() {
            "enter" => match which {
                ViewerInput::Find => self.jump_to_match(cx),
                ViewerInput::Filter => self.apply_filter(id, cx),
            },
            "escape" => {
                let Some(r) = self.scope.responses.get_mut(&id) else {
                    return;
                };
                let input = match which {
                    ViewerInput::Filter => {
                        r.filter.clear();
                        r.filtered = None;
                        r.filter_error = None;
                        r.filter_input.borrow().clone()
                    }
                    ViewerInput::Find => {
                        r.find.clear();
                        r.find_input.borrow().clone()
                    }
                };
                if let Some(i) = input {
                    i.update(cx, |i, cx| i.set_text("", cx));
                }
                self.refresh_shown(id);
                cx.notify();
            }
            _ => {}
        }
    }

    fn apply_filter(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(r) = self.scope.responses.get_mut(&tab_id) else {
            return;
        };
        let path = r.filter.trim().to_string();
        r.filter_error = None;
        if path.is_empty() {
            r.filtered = None;
            self.refresh_shown(tab_id);
            return;
        }
        let Some(raw) = r.view.as_ref().and_then(|v| v.raw.clone()) else {
            return;
        };
        let seq = r.seq;
        let task = cx
            .background_executor()
            .spawn(async move { filter_json(&raw.text, &path) });
        cx.spawn(async move |this, cx| {
            let out = task.await;
            this.update(cx, |this, cx| {
                if let Some(r) = this
                    .scope
                    .responses
                    .get_mut(&tab_id)
                    .filter(|r| r.seq == seq)
                {
                    match out {
                        Ok(text) => {
                            r.filtered = Some(Arc::new(LineText::new(text)));
                            r.filter_error = None;
                        }
                        Err(e) => {
                            r.filtered = None;
                            r.filter_error = Some(e);
                        }
                    }
                    this.refresh_shown(tab_id);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- copy / save ------------------------------------------------------------------------

    /// `Copy as cURL`; secrets are masked unless `with_secrets` (SPEC §9.5).
    pub fn copy_curl(&mut self, with_secrets: bool, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        let Some(doc) = ws.doc(&key) else { return };
        let parsed = doc.parsed.clone();
        let Some(scope) = self.scope_for(&key) else {
            return;
        };
        match runner().export_curl(&parsed, &scope, !with_secrets) {
            Ok(cmd) => {
                cx.write_to_clipboard(ClipboardItem::new_string(cmd));
                self.notice = Some(if with_secrets {
                    "Copied as cURL (with secrets).".into()
                } else {
                    "Copied as cURL (secrets masked).".into()
                });
            }
            Err(e) => self.notice = Some(e.describe().0),
        }
        cx.notify();
    }

    pub fn copy_response_body(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self
            .scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get(&id))
        else {
            return;
        };
        let text = match (&r.shown, r.result.as_ref().and_then(|x| x.outcome.as_ref())) {
            (Some(s), _) => Some(s.text.clone()),
            (None, Some(o)) => o
                .exchange
                .as_ref()
                .and_then(|x| x.body.as_memory())
                .map(|b| String::from_utf8_lossy(b).into_owned()),
            _ => None,
        };
        match text {
            Some(t) => {
                cx.write_to_clipboard(ClipboardItem::new_string(t));
                self.notice = Some("Copied response body.".into());
            }
            None => self.notice = Some("Nothing to copy.".into()),
        }
        cx.notify();
    }

    pub fn save_response_body(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self
            .scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get(&id))
        else {
            return;
        };
        let Some(x) = r
            .result
            .as_ref()
            .and_then(|x| x.outcome.as_ref())
            .and_then(|o| o.exchange.as_ref())
        else {
            self.notice = Some("There is no response to save.".into());
            return;
        };
        let body = x.body.clone();
        let name = format!(
            "response.{}",
            extension_for(r.view.as_ref().map_or(BodyKind::Binary, |v| v.kind))
        );
        let dir = self
            .scope
            .workspace
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let res = body.save_to(&path);
            this.update(cx, |this, cx| {
                this.notice = Some(match res {
                    Ok(()) => format!("Saved {}.", path.display()),
                    Err(e) => format!("Could not save: {e}."),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- rendering --------------------------------------------------------------------------

    pub(crate) fn render_response(
        &self,
        t: &Theme,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab_id) = self.scope.tabs.active_id() else {
            return div().into_any_element();
        };
        let r = self.scope.responses.get(&tab_id);
        let strip = |content: AnyElement| {
            row()
                .h(px(30.))
                .flex_none()
                .px_3()
                .gap_3()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .border_b_1()
                .border_color(t.border_variant)
                .child(content)
        };
        let shell = |children: Vec<AnyElement>| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .children(children)
                .into_any_element()
        };
        let Some(r) = r else {
            return shell(vec![
                strip(
                    row()
                        .flex_1()
                        .child("Response")
                        .child(div().flex_1())
                        .child(self.render_history_dropdown(t, cx))
                        .into_any_element(),
                )
                .into_any_element(),
                div()
                    .p_4()
                    .text_color(t.text_placeholder)
                    .child("Send the request to see the response (Cmd/Ctrl+Enter).")
                    .into_any_element(),
            ]);
        };

        if let Some(s) = &r.sending {
            let elapsed = s.started.elapsed().as_secs_f64() * 1000.0;
            let phase = s.phases.last().map_or("queued", |p| p.name.as_str());
            return shell(vec![
                strip(
                    row()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.accent)
                                .child("Sending…"),
                        )
                        .child(fmt_ms(elapsed))
                        .child(format!("after {phase}"))
                        .child(div().text_color(t.text_placeholder).child("Esc to cancel"))
                        .into_any_element(),
                )
                .into_any_element(),
                div()
                    .id("resp-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.timeline(t, &s.phases, elapsed.max(1.0), &[]))
                    .into_any_element(),
            ]);
        }

        let Some(res) = r.result.clone() else {
            return shell(vec![]);
        };
        let Some(o) = res.outcome.as_ref() else {
            let (msg, fix) = res
                .error
                .as_ref()
                .map(|e| e.describe())
                .unwrap_or_else(|| match &res.skipped {
                    Some(reason) => (format!("Skipped by a script: {reason}"), None),
                    None => ("No response.".into(), None),
                });
            let cancelled = res.error.as_ref().is_some_and(|e| e.is_cancelled());
            let head = res
                .error
                .as_ref()
                .map_or("Failed".to_string(), |e| match e.phase() {
                    "cancelled" => "Cancelled".into(),
                    "plan" => "Cannot send".into(),
                    "requires" => "Cannot send".into(),
                    "trust" => "Scripts are not trusted".into(),
                    "pre script" => "A pre script failed".into(),
                    p => format!("Failed in the {p} phase"),
                });
            return shell(vec![
                strip(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if cancelled { t.text_muted } else { t.deleted })
                        .child(head)
                        .into_any_element(),
                )
                .into_any_element(),
                div()
                    .id("resp-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().child(msg))
                            .children(fix.map(|f| div().text_color(t.text_muted).child(f))),
                    )
                    .when(!res.phases.is_empty(), |d| {
                        let total = res
                            .phases
                            .iter()
                            .map(|p| p.start_ms + p.duration_ms)
                            .fold(1.0, f64::max);
                        d.child(self.timeline(t, &res.phases, total, &[]))
                    })
                    .when(!res.console.is_empty() || !res.tests.is_empty(), |d| {
                        d.child(tests_view(t, &res))
                            .child(script_console_view(t, &res))
                    })
                    .into_any_element(),
            ]);
        };
        let x = o.exchange.as_ref().expect("exchange outcome");
        let size = if x.wire_size != x.decoded_size {
            format!(
                "{} ({} on the wire)",
                fmt_size(x.decoded_size),
                fmt_size(x.wire_size)
            )
        } else {
            fmt_size(x.decoded_size)
        };
        let status_row = row()
            .gap_3()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.status_color(x.status))
                    .child(format!("{} {}", x.status, x.status_text)),
            )
            .child(
                div()
                    .dev_tag("volatile.total-time", Role::Text, "timings", true)
                    .child(fmt_ms(o.total_ms)),
            )
            .child(size)
            .child(x.version.clone())
            .when(!o.redirects.is_empty(), |d| {
                d.child(format!(
                    "{} redirect{}",
                    o.redirects.len(),
                    if o.redirects.len() == 1 { "" } else { "s" }
                ))
            })
            .into_any_element();
        let labels: Vec<(String, usize)> = RESP_TABS
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let n = match i {
                    1 => x.headers.len(),
                    2 => {
                        x.headers
                            .iter()
                            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                            .count()
                            + self.jar_rows(&o.final_url).len()
                    }
                    4 => res.tests.len(),
                    5 => res.console.len() + res.script_errors.len(),
                    _ => 0,
                };
                ((*s).to_string(), n)
            })
            .collect();
        let content: AnyElement = match ui.resp {
            0 => self.render_resp_body(t, tab_id, r, x.status, cx),
            1 => div()
                .id("resp-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(owned_table(t, x.headers.clone(), ("Name", "Value")))
                .into_any_element(),
            2 => {
                let rows: Vec<(String, String)> = x
                    .headers
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, v)| {
                        let (n, val, attrs) = parse_set_cookie(v);
                        (
                            n,
                            if attrs.is_empty() {
                                val
                            } else {
                                format!("{val}    ({attrs})")
                            },
                        )
                    })
                    .collect();
                let mut rows = rows;
                rows.extend(self.jar_rows(&o.final_url));
                div()
                    .id("resp-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(if rows.is_empty() {
                        div()
                            .p_3()
                            .text_color(t.text_placeholder)
                            .child("No cookies set.")
                            .into_any_element()
                    } else {
                        owned_table(t, rows, ("Name", "Value"))
                    })
                    .into_any_element()
            }
            3 => div()
                .id("resp-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(
                    div()
                        .dev_tag("volatile.timeline", Role::Text, "timings", true)
                        .child(self.timeline(t, &o.timings, o.total_ms.max(1.0), &o.redirects)),
                )
                .into_any_element(),
            4 => div()
                .id("resp-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(tests_view(t, &res))
                .into_any_element(),
            _ => div()
                .id("resp-scroll")
                .flex_1()
                .overflow_y_scroll()
                .child(script_console_view(t, &res))
                .into_any_element(),
        };
        shell(vec![
            strip(
                row()
                    .flex_1()
                    .gap_3()
                    .child(status_row)
                    .when(r.viewing.is_some(), |d| {
                        d.child(
                            row()
                                .gap_2()
                                .text_color(t.accent)
                                .child("Earlier response")
                                .child(
                                    div()
                                        .id("resp-latest")
                                        .cursor_pointer()
                                        .underline()
                                        .child("Back to latest")
                                        .dev_click(
                                            "resp-latest",
                                            Role::Button,
                                            "resp-latest",
                                            true,
                                            cx.listener(|this, _, _, cx| {
                                                this.history_back_to_latest(cx)
                                            }),
                                        ),
                                ),
                        )
                    })
                    .child(div().flex_1())
                    .child(self.render_history_dropdown(t, cx))
                    .into_any_element(),
            )
            .into_any_element(),
            self.tab_strip(t, "resp", labels, ui.resp, |u, i| u.resp = i, cx)
                .into_any_element(),
            content,
        ])
    }

    fn render_resp_body(
        &self,
        t: &Theme,
        tab_id: u64,
        r: &TabResponse,
        _status: u16,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = r.view.clone() else {
            return div()
                .p_3()
                .text_color(t.text_placeholder)
                .child("Preparing the body…")
                .into_any_element();
        };
        let chip = |id: &'static str, label: &'static str, on: bool, mode: BodyMode| {
            row()
                .id(id)
                .h(px(22.))
                .px_2()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(on, |d| d.bg(t.element_selected).text_color(t.text))
                .when(!on, |d| {
                    let h = t.element_hover;
                    d.text_color(t.text_muted).hover(move |s| s.bg(h))
                })
                .dev_click(
                    id,
                    Role::Button,
                    id,
                    true,
                    cx.listener(move |this, _, _, cx| this.set_body_mode(mode, cx)),
                )
                .child(label)
        };
        let has_text = view.pretty.is_some();
        let tool = |id: &'static str, label: &'static str, on: bool| {
            row()
                .id(id)
                .h(px(22.))
                .px_2()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if on { t.accent } else { t.text_muted })
                .hover({
                    let h = t.element_hover;
                    move |s| s.bg(h)
                })
                .child(label)
        };
        let input = |id: &'static str,
                     slot: &crate::text_input::LazyInput,
                     placeholder: &'static str,
                     which: ViewerInput,
                     w: f32,
                     cx: &mut Context<Self>| {
            let field = crate::text_input::lazy_input(
                slot,
                placeholder,
                cx,
                Self::viewer_on_change(tab_id, which),
            );
            row()
                .id(id)
                .on_key_down(cx.listener(move |this, ev, _, cx| this.on_viewer_key(which, ev, cx)))
                .w(px(w))
                .h(px(22.))
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
                .child(field)
        };
        let toolbar = row()
            .flex_none()
            .h(px(30.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .px_2()
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child(view.kind.label()),
            )
            .when(has_text, |d| {
                d.child(chip(
                    "m-pretty",
                    "Pretty",
                    r.mode == BodyMode::Pretty,
                    BodyMode::Pretty,
                ))
                .child(chip(
                    "m-raw",
                    "Raw",
                    r.mode == BodyMode::Raw,
                    BodyMode::Raw,
                ))
            })
            .when(view.image.is_some(), |d| {
                d.child(chip(
                    "m-preview",
                    "Preview",
                    r.mode == BodyMode::Preview,
                    BodyMode::Preview,
                ))
            })
            .child(chip("m-hex", "Hex", r.mode == BodyMode::Hex, BodyMode::Hex))
            .child(div().flex_1())
            .when(view.kind == BodyKind::Json && has_text, |d| {
                d.child(input(
                    "filter",
                    &r.filter_input,
                    "JSONPath  $.items[*].id",
                    ViewerInput::Filter,
                    190.,
                    cx,
                ))
            })
            .when(has_text || r.mode == BodyMode::Hex, |d| {
                d.child(input(
                    "find",
                    &r.find_input,
                    "Find",
                    ViewerInput::Find,
                    120.,
                    cx,
                ))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(if r.find.len() >= 2 {
                            format!("{} found", r.matches.len())
                        } else {
                            String::new()
                        }),
                )
                .child(tool("wrap", "Wrap", r.wrap).dev_click(
                    "response.render_resp_body",
                    Role::Button,
                    "response.render_resp_body",
                    true,
                    cx.listener(|this, _, _, cx| this.toggle_wrap(cx)),
                ))
            })
            .child(tool("copy", "Copy", false).dev_click(
                "response.render_resp_body",
                Role::Button,
                "response.render_resp_body",
                true,
                cx.listener(|this, _, _, cx| this.copy_response_body(cx)),
            ))
            .child(tool("save", "Save", false).dev_click(
                "response.render_resp_body",
                Role::Button,
                "response.render_resp_body",
                true,
                cx.listener(|this, _, _, cx| this.save_response_body(cx)),
            ));

        let body: AnyElement = if r.mode == BodyMode::Preview
            && let Some((fmt, bytes)) = &view.image
        {
            let format = match fmt {
                ImageFormat::Png => gpui::ImageFormat::Png,
                ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
                ImageFormat::Gif => gpui::ImageFormat::Gif,
                ImageFormat::Webp => gpui::ImageFormat::Webp,
                ImageFormat::Svg => gpui::ImageFormat::Svg,
            };
            div()
                .id("resp-image")
                .flex_1()
                .overflow_y_scroll()
                .p_3()
                .child(img(Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))).max_w_full())
                .into_any_element()
        } else if let Some(shown) = r.shown.clone() {
            let visible = r.visible.clone();
            let n = visible.as_ref().map_or(shown.len(), |v| v.len());
            let json = view.kind == BodyKind::Json && r.mode != BodyMode::Hex;
            let folds = view.folds.clone();
            let collapsed = r.collapsed.clone();
            let matches: Arc<BTreeSet<u32>> = Arc::new(r.matches.iter().copied().collect());
            let foldable = r.mode == BodyMode::Pretty && !r.wrap && r.filtered.is_none();
            let list = uniform_list(
                "resp-body",
                n,
                cx.processor(
                    move |_this: &mut WorkspaceView, range: Range<usize>, _w, cx| {
                        let t = crate::theme::theme(cx).clone();
                        range
                            .map(|row_ix| {
                                let line = visible.as_ref().map_or(row_ix, |v| v[row_ix] as usize);
                                let fold = (foldable
                                    && folds.get(line).is_some_and(|e| *e != u32::MAX))
                                .then(|| collapsed.contains(&line));
                                body_row(
                                    &t,
                                    line,
                                    shown.line(line),
                                    json,
                                    fold,
                                    matches.contains(&(line as u32)),
                                    tab_id,
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(r.scroll.clone())
            .flex_1()
            .min_h_0()
            .font_family(BUFFER_FONT)
            .text_size(px(13.));
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .children(r.filter_error.as_ref().map(|e| {
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(t.deleted)
                        .child(format!("Filter: {e}"))
                }))
                .when(view.too_large, |d| {
                    d.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(format!(
                                "Body is {}: too large to show as text. Showing the first 4 MiB as hex; use Save for the rest.",
                                fmt_size(view.size)
                            )),
                    )
                })
                .child(list)
                .into_any_element()
        } else {
            div()
                .p_3()
                .text_color(t.text_placeholder)
                .child(if view.kind == BodyKind::Empty {
                    "The response body is empty."
                } else {
                    "Nothing to show in this mode."
                })
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }

    fn timeline(
        &self,
        t: &Theme,
        phases: &[PhaseTiming],
        total_ms: f64,
        redirects: &[fluxwire_runner::RedirectHop],
    ) -> AnyElement {
        let total = total_ms.max(0.001);
        div()
            .flex()
            .flex_col()
            .p_3()
            .children(phases.iter().map(|p| {
                let left = (p.start_ms / total).clamp(0.0, 1.0) as f32;
                let width = ((p.duration_ms / total) as f32).clamp(0.004, 1.0 - left.min(0.996));
                let bold = p.name == "total";
                row()
                    .h(px(24.))
                    .gap_3()
                    .text_size(px(12.))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .text_color(if bold { t.text } else { t.text_muted })
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(8.))
                            .relative()
                            .bg(t.element_hover)
                            .when(!p.skipped && !bold, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .h_full()
                                        .left(relative(left))
                                        .w(relative(width))
                                        .rounded(px(2.))
                                        .bg(t.accent),
                                )
                            }),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_right()
                            .font_family(BUFFER_FONT)
                            .text_color(t.text_muted)
                            .child(if p.skipped {
                                "reused".to_string()
                            } else {
                                fmt_ms(p.duration_ms)
                            }),
                    )
            }))
            .child(
                div()
                    .pt_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(if redirects.is_empty() {
                        div()
                            .text_color(t.text_placeholder)
                            .child("Redirects: none")
                    } else {
                        div().child("Redirect chain")
                    })
                    .children(redirects.iter().map(|h| {
                        div()
                            .font_family(BUFFER_FONT)
                            .child(format!("{}  {}  →  {}", h.status, h.from, h.to))
                    })),
            )
            .into_any_element()
    }

    /// Console lines shown in the bottom dock.
    pub(crate) fn console_view(&self, t: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .p_2()
            .font_family(BUFFER_FONT)
            .text_size(px(12.))
            .text_color(t.text_muted)
            .when(self.scope.console.is_empty(), |d| {
                d.child(
                    div()
                        .text_color(t.text_placeholder)
                        .child("Nothing sent yet."),
                )
            })
            .children(self.scope.console.iter().map(|l| div().child(l.clone())))
            .into_any_element()
    }
}

/// Pass/fail list for `expect` and script tests, with expected vs actual for failures.
fn tests_view(t: &Theme, res: &RunResult) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .p_2()
        .when(res.tests.is_empty(), |d| {
            d.child(div().p_1().text_color(t.text_placeholder).child(
                "No tests. Add an `http expect` block or a `ts test` block to check this response.",
            ))
        })
        .when(!res.tests.is_empty(), |d| {
            let passed = res.tests.iter().filter(|x| x.passed).count();
            d.child(
                div()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child(format!("{passed} of {} passed", res.tests.len())),
            )
        })
        .children(res.tests.iter().map(|test| {
            div()
                .flex()
                .flex_col()
                .py_1()
                .child(
                    row()
                        .gap_2()
                        .child(
                            div()
                                .text_color(if test.passed { t.created } else { t.deleted })
                                .child(if test.passed { "✓" } else { "✗" }),
                        )
                        .child(test.name.clone()),
                )
                .when(!test.passed, |d| {
                    d.children(test.message.clone().map(|m| {
                        div()
                            .pl_6()
                            .font_family(BUFFER_FONT)
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(m)
                    }))
                    .when(
                        test.expected.is_some() || test.actual.is_some(),
                        |d| {
                            d.child(
                                div()
                                    .pl_6()
                                    .font_family(BUFFER_FONT)
                                    .text_size(px(12.))
                                    .text_color(t.text_muted)
                                    .child(format!(
                                        "expected {}, got {}",
                                        test.expected.as_deref().unwrap_or("?"),
                                        test.actual.as_deref().unwrap_or("nothing")
                                    )),
                            )
                        },
                    )
                })
        }))
        .into_any_element()
}

fn log_glyph(level: fluxwire_runner::LogLevel) -> &'static str {
    use fluxwire_runner::LogLevel::*;
    match level {
        Debug => "debug",
        Log => "log",
        Info => "info",
        Warn => "warn",
        Error => "error",
    }
}

/// `console.*` output of one send, plus script errors with their `.flux.md` locations.
fn script_console_view(t: &Theme, res: &RunResult) -> AnyElement {
    let errors: Vec<_> = res
        .script_errors
        .iter()
        .chain(match &res.error {
            Some(fluxwire_runner::RunError::Script(e)) => e.iter(),
            _ => [].iter(),
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .p_2()
        .font_family(BUFFER_FONT)
        .text_size(px(12.))
        .text_color(t.text_muted)
        .when(res.console.is_empty() && errors.is_empty(), |d| {
            d.child(
                div()
                    .text_color(t.text_placeholder)
                    .child("No script output for this send."),
            )
        })
        .children(res.console.iter().map(|l| {
            let color = match l.level {
                fluxwire_runner::LogLevel::Error => t.deleted,
                fluxwire_runner::LogLevel::Warn => t.modified,
                _ => t.text_muted,
            };
            row()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .text_color(t.text_placeholder)
                        .child(clock(
                            SystemTime::UNIX_EPOCH + Duration::from_millis(l.ts_ms),
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(40.))
                        .text_color(color)
                        .child(log_glyph(l.level)),
                )
                .child(div().flex_1().child(l.text.clone()))
        }))
        .children(errors.into_iter().map(|e| {
            div().text_color(t.deleted).child(format!(
                "{} {} ({})",
                e.kind.code(),
                e.message,
                e.location()
            ))
        }))
        .into_any_element()
}

fn owned_table(
    t: &Theme,
    rows: Vec<(String, String)>,
    heads: (&'static str, &'static str),
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
    div()
        .flex()
        .flex_col()
        .child(
            row()
                .h(px(24.))
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(t.border_variant)
                .child(head(heads.0, Some(200.)))
                .child(head(heads.1, None)),
        )
        .children(rows.into_iter().map(|(k, v)| {
            row()
                .min_h(px(26.))
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(t.border_variant)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .child(
                    div()
                        .w(px(200.))
                        .flex_none()
                        .text_color(t.syntax_property)
                        .child(k),
                )
                .child(div().flex_1().min_w_0().child(v))
        }))
        .into_any_element()
}

/// One row of the virtualized body list.
#[allow(clippy::too_many_arguments)]
fn body_row(
    t: &Theme,
    line_ix: usize,
    text: &str,
    json: bool,
    fold: Option<bool>,
    matched: bool,
    tab_id: u64,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let (shown, cut) = match text.char_indices().nth(MAX_LINE_CHARS) {
        Some((i, _)) => (&text[..i], text.chars().count() - MAX_LINE_CHARS),
        None => (text, 0),
    };
    let runs: Vec<(String, Tok)> = if json && cut == 0 {
        lex_json_line(shown)
    } else {
        vec![(shown.to_string(), Tok::Plain)]
    };
    let chevron: AnyElement = match fold {
        Some(collapsed) => div()
            .id(ElementId::from(("fold", line_ix)))
            .w(px(14.))
            .flex_none()
            .cursor_pointer()
            .child(icon(
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                11.,
                t.text_placeholder,
            ))
            .dev_click(
                "response.body_row",
                Role::Button,
                "response.body_row",
                true,
                cx.listener(move |this, _, _, cx| this.toggle_fold(tab_id, line_ix, cx)),
            )
            .into_any_element(),
        None => div().w(px(14.)).flex_none().into_any_element(),
    };
    div()
        .flex()
        .flex_row()
        .h(px(ROW_HEIGHT))
        .whitespace_nowrap()
        .when(matched, |d| d.bg(t.element_selected))
        .child(
            div()
                .w(px(52.))
                .pr(px(8.))
                .flex_none()
                .text_right()
                .text_color(t.gutter)
                .child((line_ix + 1).to_string()),
        )
        .child(chevron)
        .children(
            runs.into_iter()
                .map(|(s, tok)| div().text_color(tok_color(t, tok)).child(s)),
        )
        .when(fold == Some(true), |d| {
            d.child(div().text_color(t.text_placeholder).child(" …"))
        })
        .when(cut > 0, |d| {
            d.child(
                div()
                    .text_color(t.text_placeholder)
                    .child(format!("  … {cut} more characters (enable Wrap)")),
            )
        })
        .into_any_element()
}
