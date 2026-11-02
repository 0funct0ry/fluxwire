//! History in the UI (SPEC §13): recording every send, the store plumbing behind the History
//! tab and navigator (`history_tab.rs`, `history_nav.rs`), the read-only History entry and diff
//! tabs, restore, copy as cURL, and the response-pane dropdown.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fluxwire_format as fmt;
use fluxwire_protocol::HistoryRecord;
use fluxwire_runner::{RunResult, runner};
use fluxwire_store::{
    Counts, DiffKind, DiffRow, Entry, Filter, HistorySettings, NewEntry, Redactor, Scope, Stats,
    Store, Summary, data_dir, diff_headers, diff_json_or_text, workspace_key,
};
use gpui::{
    AnyElement, ClipboardItem, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div, prelude::*, px,
};
use serde_json::Value;

use crate::{
    assets::BUFFER_FONT,
    center::button,
    editor::{Lang, code_view},
    history_query::{DEFAULT_WIDTHS, HistRow, PAGE, Query, build_rows},
    tabs::{Tab, TabKind},
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
};

const RECENT: usize = 10;
const DIFF_ROW_CAP: usize = 2000;
const REDACTED: &str = "‹redacted›";

pub const METHOD_CHIPS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// `0` is "failed before a response".
pub const CLASS_CHIPS: [(u8, &str); 5] =
    [(2, "2xx"), (3, "3xx"), (4, "4xx"), (5, "5xx"), (0, "err")];

/// Four side-by-side comparisons of two entries.
#[derive(Debug, Default)]
pub struct DiffSet {
    pub req_headers: Vec<DiffRow>,
    pub req_body: Vec<DiffRow>,
    pub res_headers: Vec<DiffRow>,
    pub res_body: Vec<DiffRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClearScope {
    Request,
    Collection,
    All,
}

/// A pointer drag in the History tab.
#[derive(Clone, Copy, Debug)]
pub enum HistResize {
    /// Column index, pointer x and width at the start.
    Col(usize, f32, f32),
    /// Pointer x and detail width at the start.
    Divider(f32, f32),
}

pub struct HistoryState {
    pub store: Option<Store>,
    pub ws_key: Option<String>,
    pub entries: Vec<Summary>,
    pub rows: Arc<Vec<HistRow>>,
    pub all_loaded: bool,
    pub query: Query,
    pub input: Option<gpui::Entity<crate::text_input::TextInput>>,
    pub stats: Stats,
    pub counts: Counts,
    /// The row shown in the detail pane.
    pub selected: Option<i64>,
    /// Rows ticked for diff, pin and delete.
    pub ticked: BTreeSet<i64>,
    /// Folder runs whose sends are shown under their group row.
    pub expanded: HashSet<String>,
    pub widths: [f32; 7],
    /// Width of the detail pane.
    pub divider: f32,
    pub resizing: Option<HistResize>,
    pub note_input: Option<gpui::Entity<crate::text_input::TextInput>>,
    /// The entry whose note the note field holds.
    pub note_for: Option<i64>,
    pub menu: Option<(i64, gpui::Point<gpui::Pixels>)>,
    pub nav_menu: Option<(crate::history_nav::NavTarget, gpui::Point<gpui::Pixels>)>,
    pub clear_menu: bool,
    pub export_menu: bool,
    pub include_pinned: bool,
    pub focus: Option<gpui::FocusHandle>,
    pub cache: HashMap<i64, Arc<Entry>>,
    pub diffs: HashMap<(i64, i64), Arc<DiffSet>>,
    /// First entry picked for "Diff with…".
    pub pick: Option<i64>,
    pub scroll: UniformListScrollHandle,
    pub nav_scroll: UniformListScrollHandle,
    /// Recent sends of the active request, for the response-pane dropdown.
    pub recent: Vec<Summary>,
    pub dropdown: bool,
    pub generation: u64,
}

impl HistoryState {
    /// `persist` opens `history.sqlite` in the user data dir and starts retention; otherwise
    /// the store lives in memory (tests, no state dir).
    pub fn new(persist: bool) -> Self {
        let store = if persist {
            data_dir().and_then(|d| Store::open(&d.join("history.sqlite")).ok())
        } else {
            Store::open_in_memory().ok()
        };
        if persist && let Some(s) = store.clone() {
            // retention on startup and every hour, off the UI thread
            std::thread::spawn(move || {
                loop {
                    let _ = s.enforce_retention(&HistorySettings::load_default(), now_ms());
                    std::thread::sleep(Duration::from_secs(3600));
                }
            });
        }
        Self {
            store,
            ws_key: None,
            entries: Vec::new(),
            rows: Arc::default(),
            all_loaded: true,
            query: Query::default(),
            input: None,
            stats: Stats::default(),
            counts: Counts::default(),
            selected: None,
            ticked: BTreeSet::new(),
            expanded: HashSet::new(),
            widths: DEFAULT_WIDTHS,
            divider: 420.0,
            resizing: None,
            note_input: None,
            note_for: None,
            menu: None,
            nav_menu: None,
            clear_menu: false,
            export_menu: false,
            include_pinned: false,
            focus: None,
            cache: HashMap::new(),
            diffs: HashMap::new(),
            pick: None,
            scroll: UniformListScrollHandle::new(),
            nav_scroll: UniformListScrollHandle::new(),
            recent: Vec::new(),
            dropdown: false,
            generation: 0,
        }
    }

    pub(crate) fn filter(&self, before: Option<i64>, offset: usize) -> Filter {
        let now = now_ms();
        self.query.to_filter(now, day_start_ms(now), before, offset)
    }

    pub fn rebuild_rows_pub(&mut self) {
        self.rebuild_rows()
    }

    pub(crate) fn rebuild_rows(&mut self) {
        self.rows = Arc::new(build_rows(
            &self.entries,
            &self.expanded,
            self.all_loaded,
            day_label,
        ));
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Start of the user's current local day, in ms.
pub fn day_start_ms(now: i64) -> i64 {
    use chrono::TimeZone;
    local(now)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| chrono::Local.from_local_datetime(&d).earliest())
        .map_or(now, |d| d.timestamp_millis())
}

pub(crate) fn local(ts_ms: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .unwrap_or_default()
}

pub fn day_label(ts_ms: i64) -> String {
    let d = local(ts_ms).date_naive();
    let today = chrono::Local::now().date_naive();
    if d == today {
        "Today".into()
    } else if today.pred_opt() == Some(d) {
        "Yesterday".into()
    } else {
        d.format("%a %d %b %Y").to_string()
    }
}

pub fn time_label(ts_ms: i64) -> String {
    local(ts_ms).format("%H:%M:%S").to_string()
}

pub(crate) fn short_method(m: &str) -> &str {
    if m == "DELETE" { "DEL" } else { m }
}

pub(crate) fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> {
    let h = runner().handle().spawn_blocking(f);
    async move { h.await.ok() }
}

/// Text of a stored `body` object (`text`, or a placeholder for binary).
pub(crate) fn body_text(v: Option<&Value>) -> String {
    let Some(b) = v else { return String::new() };
    if let Some(t) = b.get("text").and_then(Value::as_str) {
        t.to_string()
    } else if b.get("b64").is_some() {
        format!(
            "<binary, {} bytes>",
            b.get("size").and_then(Value::as_u64).unwrap_or(0)
        )
    } else {
        String::new()
    }
}

pub(crate) fn headers_of(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(|h| serde_json::from_value(h.clone()).ok())
        .unwrap_or_default()
}

pub(crate) fn lang_for(text: &str) -> Lang {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        Lang::Json
    } else if t.starts_with('<') {
        Lang::Xml
    } else {
        Lang::Plain
    }
}

pub(crate) fn pretty(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_string())
}

fn compute_diff(a: &Entry, b: &Entry) -> DiffSet {
    let get = |e: &Entry, side: &str| e.payload.get(side).cloned().unwrap_or(Value::Null);
    let (ra, rb) = (get(a, "request"), get(b, "request"));
    let (sa, sb) = (get(a, "response"), get(b, "response"));
    DiffSet {
        req_headers: diff_headers(
            &headers_of(ra.get("headers")),
            &headers_of(rb.get("headers")),
        ),
        req_body: diff_json_or_text(&body_text(ra.get("body")), &body_text(rb.get("body"))),
        res_headers: diff_headers(
            &headers_of(sa.get("headers")),
            &headers_of(sb.get("headers")),
        ),
        res_body: diff_json_or_text(&body_text(sa.get("body")), &body_text(sb.get("body"))),
    }
}

pub(crate) fn record_of(e: &Entry) -> HistoryRecord {
    HistoryRecord {
        protocol: e.summary.protocol.clone(),
        summary: e.summary.summary.clone(),
        payload: e.payload.clone(),
    }
}

impl WorkspaceView {
    // ---- store plumbing ---------------------------------------------------------------------

    /// The workspace changed (opened or closed): reload the panel for its history.
    pub fn history_workspace_changed(&mut self, cx: &mut Context<Self>) {
        self.history.ws_key = self
            .scope
            .workspace
            .as_ref()
            .map(|p| workspace_key(&p.root));
        self.history.entries.clear();
        self.history.cache.clear();
        self.history.diffs.clear();
        self.history.pick = None;
        self.history.selected = None;
        self.history.ticked.clear();
        self.history.expanded.clear();
        self.history.note_for = None;
        self.history.menu = None;
        self.history.nav_menu = None;
        self.history.query.clear();
        self.history.query.nav = None;
        if let Some(i) = &self.history.input {
            i.update(cx, |i, cx| i.set_text("", cx));
        }
        self.history.stats = Stats::default();
        self.history.counts = Counts::default();
        self.history.rebuild_rows();
        self.history_reload(cx);
    }

    pub fn history_reload(&mut self, cx: &mut Context<Self>) {
        let (Some(store), Some(ws)) = (self.history.store.clone(), self.history.ws_key.clone())
        else {
            cx.notify();
            return;
        };
        self.history.generation += 1;
        let gen_ = self.history.generation;
        let filter = self.history.filter(None, 0);
        let recent_path = self.active_key();
        let day_start = day_start_ms(now_ms());
        let task = blocking(move || {
            let list = store.list(&ws, &filter).unwrap_or_default();
            let stats = store.stats(&ws, &filter).unwrap_or_default();
            let counts = store.counts(&ws, day_start).unwrap_or_default();
            let recent = recent_path
                .map(|p| {
                    store
                        .list(
                            &ws,
                            &Filter {
                                request_path: Some(p),
                                limit: Some(RECENT),
                                ..Default::default()
                            },
                        )
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            (list, recent, stats, counts)
        });
        cx.spawn(async move |this, cx| {
            let Some((list, recent, stats, counts)) = task.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.history.generation != gen_ {
                    return;
                }
                this.history.all_loaded = list.len() < PAGE;
                this.history.entries = list;
                this.history.recent = recent;
                this.history.stats = stats;
                this.history.counts = counts;
                this.history
                    .ticked
                    .retain(|id| this.history.entries.iter().any(|e| e.id == *id));
                this.history.rebuild_rows();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn history_more(&mut self, cx: &mut Context<Self>) {
        let (Some(store), Some(ws), Some(last)) = (
            self.history.store.clone(),
            self.history.ws_key.clone(),
            self.history.entries.last().map(|e| e.id),
        ) else {
            return;
        };
        if self.history.all_loaded {
            return;
        }
        let gen_ = self.history.generation;
        let filter = self.history.filter(Some(last), self.history.entries.len());
        let task = blocking(move || store.list(&ws, &filter).unwrap_or_default());
        cx.spawn(async move |this, cx| {
            let Some(list) = task.await else { return };
            this.update(cx, |this, cx| {
                if this.history.generation != gen_ {
                    return;
                }
                this.history.all_loaded = list.len() < PAGE;
                this.history.entries.extend(list);
                this.history.rebuild_rows();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Record a finished send (called from `finish_run`); the write happens off the UI thread.
    pub fn history_record(&mut self, tab_id: u64, res: &RunResult, cx: &mut Context<Self>) {
        let (Some(store), Some(wskey), Some(ws), Some(rec)) = (
            self.history.store.clone(),
            self.history.ws_key.clone(),
            self.scope.workspace.as_ref(),
            res.history.as_ref(),
        ) else {
            return;
        };
        if res.error.as_ref().is_some_and(|e| e.is_cancelled()) {
            return;
        }
        let Some(key) = self.scope.tabs.get(tab_id).map(|t| t.key.clone()) else {
            return;
        };
        let Some(doc) = ws.doc(&key) else { return };
        let abs = ws.abs_of(&key);
        let collection = ws
            .collections
            .iter()
            .filter(|c| abs.starts_with(&c.root))
            .max_by_key(|c| c.root.components().count())
            .map(|c| c.key.clone())
            .unwrap_or_default();
        let x = res.outcome.as_ref().and_then(|o| o.exchange.as_ref());
        let req = rec.payload.get("request");
        let str_of = |v: Option<&Value>, k: &str| {
            v.and_then(|v| v.get(k))
                .and_then(Value::as_str)
                .map(String::from)
        };
        let entry = NewEntry {
            ts_ms: res
                .started_at
                .duration_since(UNIX_EPOCH)
                .map_or_else(|_| now_ms(), |d| d.as_millis() as i64),
            collection,
            request_path: key,
            request_name: doc.title(),
            protocol: rec.protocol.clone(),
            method: str_of(req, "method").unwrap_or_else(|| doc.method()),
            url: res
                .outcome
                .as_ref()
                .map(|o| o.final_url.clone())
                .or_else(|| str_of(req, "url"))
                .unwrap_or_else(|| res.label.clone()),
            status: x.map(|x| i64::from(x.status)),
            total_ms: res.outcome.as_ref().map_or_else(
                || res.phases.iter().map(|p| p.duration_ms).sum(),
                |o| o.total_ms,
            ),
            env: self.active_env_name(),
            dirty: doc.dirty(),
            run_id: None,
            origin: None,
            summary: rec.summary.clone(),
            payload: rec.payload.clone(),
        };
        let red = Redactor::with_secrets(res.secrets.clone());
        let task = blocking(move || store.insert(&wskey, entry, &red));
        cx.spawn(async move |this, cx| {
            task.await;
            this.update(cx, |this, cx| this.history_reload(cx)).ok();
        })
        .detach();
    }

    /// Fetch entries into the cache, then run `then`.
    pub(crate) fn history_fetch(
        &mut self,
        ids: Vec<i64>,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let Some(store) = self.history.store.clone() else {
            return;
        };
        let missing: Vec<i64> = ids
            .into_iter()
            .filter(|i| !self.history.cache.contains_key(i))
            .collect();
        if missing.is_empty() {
            then(self, cx);
            return;
        }
        let task = blocking(move || {
            missing
                .into_iter()
                .filter_map(|i| store.get(i).ok().flatten())
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let Some(found) = task.await else { return };
            this.update(cx, |this, cx| {
                for e in found {
                    if e.summary.protocol == "ws"
                        && let Some(o) = runner().history_outcome(&record_of(&e))
                    {
                        let frames = o.session.map(|s| s.frames).unwrap_or_default();
                        this.ensure_history_log(e.summary.id, frames);
                    }
                    this.history.cache.insert(e.summary.id, Arc::new(e));
                }
                then(this, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- actions ----------------------------------------------------------------------------

    pub fn open_history_entry(&mut self, id: i64, permanent: bool, cx: &mut Context<Self>) {
        self.open_history_entry_titled(id, None, permanent, cx);
    }

    /// `title` is the saved tab title when restoring a session, before the list has loaded.
    pub fn open_history_entry_titled(
        &mut self,
        id: i64,
        title: Option<&str>,
        permanent: bool,
        cx: &mut Context<Self>,
    ) {
        let title = self
            .history
            .entries
            .iter()
            .chain(&self.history.recent)
            .find(|e| e.id == id)
            .map(|e| e.request_name.clone())
            .or_else(|| title.filter(|t| !t.is_empty()).map(String::from))
            .unwrap_or_else(|| format!("History {id}"));
        self.open_tab(
            &format!("history/{id}"),
            &title,
            TabKind::History,
            permanent,
        );
        self.history_fetch(vec![id], cx, |_, _| {});
        cx.notify();
    }

    /// Pick a first entry, or diff the first pick against `id`.
    pub fn history_pick_diff(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.history.pick {
            Some(a) if a != id => {
                self.history.pick = None;
                self.open_history_diff(a, id, cx);
            }
            Some(_) => self.history.pick = None,
            None => {
                self.history.pick = Some(id);
                self.notice = Some("Pick another history entry to diff with.".into());
            }
        }
        cx.notify();
    }

    pub fn open_history_diff(&mut self, a: i64, b: i64, cx: &mut Context<Self>) {
        self.open_tab(
            &format!("historydiff/{a}/{b}"),
            &format!("Diff #{a} ↔ #{b}"),
            TabKind::History,
            true,
        );
        self.history_fetch(vec![a, b], cx, move |this, cx| {
            let (Some(x), Some(y)) = (
                this.history.cache.get(&a).cloned(),
                this.history.cache.get(&b).cloned(),
            ) else {
                return;
            };
            let task = cx
                .background_executor()
                .spawn(async move { compute_diff(&x, &y) });
            cx.spawn(async move |this, cx| {
                let set = task.await;
                this.update(cx, |this, cx| {
                    this.history.diffs.insert((a, b), Arc::new(set));
                    cx.notify();
                })
                .ok();
            })
            .detach();
        });
    }

    pub fn history_copy_curl(&mut self, id: i64, cx: &mut Context<Self>) {
        self.history_fetch(vec![id], cx, move |this, cx| {
            let curl = this
                .history
                .cache
                .get(&id)
                .and_then(|e| runner().history_curl(&record_of(e)));
            match curl {
                Some(c) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(c));
                    this.notice = Some("Copied as cURL (secrets are redacted in history).".into());
                }
                None => this.notice = Some("This entry cannot be exported as cURL.".into()),
            }
        });
    }

    /// Put an entry's request into an unsaved tab of its request file (never written to disk
    /// until the user saves).
    pub fn history_restore(
        &mut self,
        id: i64,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_request(&key, true, window, cx);
        self.history_fetch(vec![id], cx, move |this, cx| {
            let Some(e) = this.history.cache.get(&id).cloned() else {
                return;
            };
            let Some(p) = this.scope.workspace.as_mut() else {
                return;
            };
            let Ok(doc) = p.open_doc(&key) else {
                this.notice = Some(format!("{key} no longer exists; cannot restore."));
                return;
            };
            let req = e.payload.get("request").cloned().unwrap_or(Value::Null);
            let method = req
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or(&e.summary.method)
                .to_string();
            let url = req
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or(&e.summary.url)
                .to_string();
            let edits = fmt::set_request_line(&doc.parsed, &method, &url);
            doc.apply(&edits);
            let mut n = doc.parsed.request().map_or(0, |r| r.headers.len());
            while n > 0 {
                n -= 1;
                let edits = fmt::remove_header(&doc.parsed, n);
                doc.apply(&edits);
            }
            for (k, v) in headers_of(req.get("headers")) {
                if v != REDACTED {
                    let edits = fmt::add_header(&doc.parsed, &k, &v);
                    doc.apply(&edits);
                }
            }
            let body = body_text(req.get("body"));
            let edits = fmt::set_body(&doc.parsed, &body);
            doc.apply(&edits);
            this.after_edit_pub(&key, cx);
        });
    }

    pub fn history_clear(&mut self, scope: ClearScope, cx: &mut Context<Self>) {
        if self.history.store.is_none() {
            return;
        }
        let active = self.active_key();
        let collection = active.as_ref().and_then(|k| {
            let p = self.scope.workspace.as_ref()?;
            let abs = p.abs_of(k);
            p.collections
                .iter()
                .filter(|c| abs.starts_with(&c.root))
                .max_by_key(|c| c.root.components().count())
                .map(|c| c.key.clone())
        });
        let scope = match (scope, active, collection) {
            (ClearScope::All, _, _) => Scope::All,
            (ClearScope::Collection, _, Some(c)) => Scope::Collection(c),
            (ClearScope::Request, Some(path), Some(collection)) => {
                Scope::Request { collection, path }
            }
            _ => {
                self.notice = Some("Open a request first.".into());
                cx.notify();
                return;
            }
        };
        let include = self.history.include_pinned;
        self.history_clear_scope(scope, include, cx);
    }

    /// Delete a scope of history; pinned entries stay unless `include_pinned`.
    pub fn history_clear_scope(
        &mut self,
        scope: Scope,
        include_pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let (Some(store), Some(ws)) = (self.history.store.clone(), self.history.ws_key.clone())
        else {
            return;
        };
        let task = blocking(move || store.clear(&ws, &scope, include_pinned));
        cx.spawn(async move |this, cx| {
            let n = task.await.and_then(Result::ok).unwrap_or(0);
            this.update(cx, |this, cx| {
                this.history.cache.clear();
                this.history.selected = None;
                this.history.ticked.clear();
                this.notice = Some(format!("Cleared {n} history entries."));
                this.history_reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Show an earlier stored response in the active request's response pane.
    pub fn show_history_response(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(tab_id) = self.scope.tabs.active_id() else {
            return;
        };
        self.history.dropdown = false;
        self.history_fetch(vec![id], cx, move |this, cx| {
            let Some(e) = this.history.cache.get(&id).cloned() else {
                return;
            };
            this.show_stored_result(tab_id, &e, cx);
        });
    }

    pub fn toggle_history_dropdown(&mut self, cx: &mut Context<Self>) {
        self.history.dropdown = !self.history.dropdown;
        if self.history.dropdown {
            self.history_reload(cx);
        }
        cx.notify();
    }

    // ---- tabs -------------------------------------------------------------------------------

    pub(crate) fn render_history_tab(
        &self,
        t: &Theme,
        tab: &Tab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if tab.key == "history" {
            return self.render_history_viewer(t, cx);
        }
        if let Some(rest) = tab.key.strip_prefix("historydiff/") {
            let mut it = rest.split('/').filter_map(|s| s.parse::<i64>().ok());
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                return self.render_history_diff(t, a, b);
            }
        }
        let id: i64 = tab
            .key
            .trim_start_matches("history/")
            .parse()
            .unwrap_or_default();
        let Some(e) = self.history.cache.get(&id) else {
            return self
                .page(t, &tab.title, "History entry · read-only")
                .child(div().text_color(t.text_placeholder).child("Loading…"))
                .into_any_element();
        };
        if e.summary.protocol == "ws" {
            return self.render_ws_history(t, id, e, cx);
        }
        let s = &e.summary;
        let key = s.request_path.clone();
        let req = e.payload.get("request").cloned().unwrap_or(Value::Null);
        let res = e.payload.get("response").cloned().unwrap_or(Value::Null);
        let headers = |v: Option<&Value>| {
            headers_of(v)
                .into_iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let section = |title: &str| {
            div()
                .mt_2()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(title.to_string())
        };
        let block = |text: String, lang: Lang| {
            div()
                .p_2()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border_variant)
                .bg(t.editor)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .child(code_view(&text, lang, t))
        };
        let req_body = body_text(req.get("body"));
        let res_body = pretty(&body_text(res.get("body")));
        let mut meta = vec![time_label(s.ts_ms), format!("{:.0} ms", s.total_ms)];
        if let Some(env) = &s.env {
            meta.push(format!("env {env}"));
        }
        if s.dirty {
            meta.push("unsaved at send".into());
        }
        if e.truncated {
            meta.push("body truncated at 1 MiB".into());
        }
        let err = e
            .payload
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str);
        let diff_label = if self.history.pick.is_some_and(|p| p != id) {
            "Diff with picked"
        } else {
            "Diff with…"
        };
        self.page(t, &s.request_name, "History entry · read-only")
            .child(
                row()
                    .gap_3()
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_color(t.method_color(&s.method))
                            .child(s.method.clone()),
                    )
                    .child(div().font_family(BUFFER_FONT).child(s.url.clone()))
                    .child(
                        div()
                            .text_color(match s.status {
                                Some(c) => t.status_color(c as u16),
                                None => t.deleted,
                            })
                            .child(s.status.map_or("failed".to_string(), |c| c.to_string())),
                    ),
            )
            .child(div().text_color(t.text_muted).child(meta.join(" · ")))
            .child(
                row()
                    .gap_2()
                    .child(
                        button(t, "h-restore", "Restore", true).on_click(cx.listener(
                            move |this, _, w, cx| this.history_restore(id, key.clone(), w, cx),
                        )),
                    )
                    .child(button(t, "h-curl", "Copy as cURL", false).on_click(
                        cx.listener(move |this, _, _, cx| this.history_copy_curl(id, cx)),
                    ))
                    .child(button(t, "h-diff", diff_label, false).on_click(
                        cx.listener(move |this, _, _, cx| this.history_pick_diff(id, cx)),
                    )),
            )
            .children(err.map(|m| div().text_color(t.deleted).child(m.to_string())))
            .child(section("Request headers"))
            .child(block(headers(req.get("headers")), Lang::Plain))
            .child(section("Request body"))
            .child(block(req_body.clone(), lang_for(&req_body)))
            .child(section("Response headers"))
            .child(block(headers(res.get("headers")), Lang::Plain))
            .child(section("Response body"))
            .child(block(res_body.clone(), lang_for(&res_body)))
            .into_any_element()
    }

    /// A stored WebSocket session: handshake, then the transcript in the session log.
    fn render_ws_history(
        &self,
        t: &Theme,
        id: i64,
        e: &Entry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = &e.summary;
        let key = s.request_path.clone();
        let req = e.payload.get("request").cloned().unwrap_or(Value::Null);
        let res = e.payload.get("response").cloned().unwrap_or(Value::Null);
        let sess = e.payload.get("session").cloned().unwrap_or(Value::Null);
        let headers = |v: Option<&Value>| {
            headers_of(v)
                .into_iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let section = |title: &str| {
            div()
                .mt_2()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(title.to_string())
        };
        let block = |text: String| {
            div()
                .p_2()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border_variant)
                .bg(t.editor)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .child(code_view(&text, Lang::Plain, t))
        };
        let mut meta = vec![time_label(s.ts_ms), format!("{:.0} ms", s.total_ms)];
        if let Some(env) = &s.env {
            meta.push(format!("env {env}"));
        }
        if s.dirty {
            meta.push("unsaved at send".into());
        }
        let total = sess
            .get("total_frames")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let kept = sess
            .get("frames")
            .and_then(Value::as_array)
            .map_or(0, Vec::len) as u64;
        meta.push(format!("{total} frames"));
        if total > kept {
            meta.push(format!("transcript keeps the first {kept}"));
        }
        if let Some(c) = sess.get("close_code").and_then(Value::as_u64) {
            meta.push(format!("closed with code {c}"));
        }
        let err = e
            .payload
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
            .or_else(|| sess.get("error").and_then(Value::as_str));
        let status = div()
            .flex_none()
            .px_3()
            .py_1()
            .text_size(px(12.))
            .text_color(t.text_muted)
            .child("Transcript")
            .into_any_element();
        let log = self.render_session_log(t, crate::session::LogRef::History(id), status, cx);
        self.page(t, &s.request_name, "History entry · read-only")
            .child(
                row()
                    .gap_3()
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_color(t.method_color("WS"))
                            .child("WS"),
                    )
                    .child(div().font_family(BUFFER_FONT).child(s.url.clone()))
                    .child(
                        div()
                            .text_color(match s.status {
                                Some(c) => t.status_color(c as u16),
                                None => t.deleted,
                            })
                            .child(s.status.map_or("failed".to_string(), |c| c.to_string())),
                    ),
            )
            .child(div().text_color(t.text_muted).child(meta.join(" · ")))
            .child(
                row()
                    .gap_2()
                    .child(
                        button(t, "h-restore", "Restore", true).on_click(cx.listener(
                            move |this, _, w, cx| this.history_restore(id, key.clone(), w, cx),
                        )),
                    ),
            )
            .children(err.map(|m| div().text_color(t.deleted).child(m.to_string())))
            .child(section("Handshake request headers"))
            .child(block(headers(req.get("headers"))))
            .child(section("Handshake response headers"))
            .child(block(headers(res.get("headers"))))
            .child(section("Transcript"))
            .child(
                div()
                    .h(px(420.))
                    .flex()
                    .flex_col()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border_variant)
                    .child(log),
            )
            .into_any_element()
    }

    fn render_history_diff(&self, t: &Theme, a: i64, b: i64) -> AnyElement {
        let page = self.page(t, &format!("Diff #{a} ↔ #{b}"), "History diff · read-only");
        let Some(set) = self.history.diffs.get(&(a, b)) else {
            return page
                .child(div().text_color(t.text_placeholder).child("Comparing…"))
                .into_any_element();
        };
        let changed = |rows: &[DiffRow]| rows.iter().filter(|r| r.kind != DiffKind::Same).count();
        let section = |title: &str, rows: &[DiffRow]| {
            let shown = &rows[..rows.len().min(DIFF_ROW_CAP)];
            let side = |text: Option<&String>, kind: DiffKind, left: bool| {
                let bg = match (kind, left) {
                    (DiffKind::Same, _) => None,
                    (DiffKind::Added, false) | (DiffKind::Changed, false) => Some(t.created),
                    (DiffKind::Removed, true) | (DiffKind::Changed, true) => Some(t.deleted),
                    _ => None,
                };
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .h(px(18.))
                    .overflow_hidden()
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .when_some(bg, |d, c| d.bg(c.opacity(0.18)))
                    .child(text.cloned().unwrap_or_default())
            };
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .mt_2()
                        .mb_1()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(t.text_muted)
                        .child(format!("{title} · {} changed", changed(rows))),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .border_1()
                        .border_color(t.border_variant)
                        .rounded(px(4.))
                        .children(shown.iter().map(|r| {
                            row()
                                .child(side(r.left.as_ref(), r.kind, true))
                                .child(div().w(px(1.)).h(px(18.)).bg(t.border_variant))
                                .child(side(r.right.as_ref(), r.kind, false))
                        }))
                        .when(rows.len() > shown.len(), |d| {
                            d.child(
                                div()
                                    .px_2()
                                    .text_color(t.text_placeholder)
                                    .child(format!("… {} more rows", rows.len() - shown.len())),
                            )
                        }),
                )
        };
        page.child(section("Request headers", &set.req_headers))
            .child(section("Request body", &set.req_body))
            .child(section("Response headers", &set.res_headers))
            .child(section("Response body", &set.res_body))
            .into_any_element()
    }

    // ---- response-pane dropdown -------------------------------------------------------------

    /// The "History" button and its dropdown in the response header strip.
    pub(crate) fn render_history_dropdown(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.history.dropdown;
        let btn = div()
            .id("resp-history")
            .px_2()
            .h(px(20.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .cursor_pointer()
            .text_size(px(11.))
            .when(open, |d| d.bg(t.element_selected))
            .child("History ▾")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_history_dropdown(cx)));
        div()
            .relative()
            .child(btn)
            .when(open, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(22.))
                        .right_0()
                        .w(px(300.))
                        .p_1()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.elevated)
                        .occlude()
                        .children(if self.history.recent.is_empty() {
                            vec![
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_color(t.text_placeholder)
                                    .child("No earlier sends of this request.")
                                    .into_any_element(),
                            ]
                        } else {
                            self.history
                                .recent
                                .iter()
                                .map(|e| {
                                    let id = e.id;
                                    row()
                                        .id(("hr", id as usize))
                                        .px_2()
                                        .h(px(24.))
                                        .gap_2()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .hover({
                                            let h = t.element_hover;
                                            move |s| s.bg(h)
                                        })
                                        .child(
                                            div()
                                                .font_family(BUFFER_FONT)
                                                .text_color(match e.status {
                                                    Some(s) => t.status_color(s as u16),
                                                    None => t.deleted,
                                                })
                                                .child(
                                                    e.status
                                                        .map_or("err".into(), |s| s.to_string()),
                                                ),
                                        )
                                        .child(div().flex_1().child(format!(
                                            "{} · {:.0} ms",
                                            time_label(e.ts_ms),
                                            e.total_ms
                                        )))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.show_history_response(id, cx)
                                        }))
                                        .into_any_element()
                                })
                                .collect()
                        }),
                )
            })
            .into_any_element()
    }
}
