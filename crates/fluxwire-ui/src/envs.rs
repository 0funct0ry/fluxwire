//! Environments, variables, secrets and cookies in the UI (SPEC §6, §8, §9.3).
//!
//! State lives in `EnvStore` on the workspace; scope assembly itself is
//! `fluxwire_format::build_scope`, shared with the CLI later.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use fluxwire_format::{
    EnvRow, Environment, ScopeInputs, UserSettings, build_scope, create_environment,
    delete_environment, duplicate_environment, list_environments, load_environment,
    local_env_is_tracked, rename_environment, save_environment,
};
use fluxwire_protocol::EnvState;
use fluxwire_vars::{VarScope, refs};
use gpui::{Context, Window};

use crate::{
    ui::TextEdit,
    workspace::{ModalKind, PromptPurpose, WorkspaceView},
};

/// How long a revealed secret stays visible (SPEC §8.3).
pub const REVEAL_FOR: Duration = Duration::from_secs(10);

/// User config directory (`settings.toml`). Unset in tests.
#[derive(Clone)]
pub struct ConfigDir(pub PathBuf);
impl gpui::Global for ConfigDir {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvCol {
    Name,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// A cell of the Environment tab; every keystroke is saved to the `.env` files.
    Env {
        env: String,
        row: usize,
        col: EnvCol,
    },
    /// A field of the `auth` front matter map of a request.
    Auth { key: String, field: String },
    /// The value of a cookie in the active environment's jar (index into `CookieJar::list`).
    Cookie { idx: usize },
}

/// The single-line field being typed into on the Environment and Auth surfaces.
#[derive(Clone, Debug)]
pub struct EnvEdit {
    pub target: EditTarget,
    pub text: TextEdit,
}

/// One row of the Variables list in the Environments panel.
#[derive(Clone, Debug)]
pub struct EnvVarView {
    pub name: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompleteTarget {
    Url,
    Cell,
}

/// `{{` completion state: the names offered and the highlighted one.
#[derive(Clone, Debug)]
pub struct Completion {
    pub target: CompleteTarget,
    pub names: Vec<String>,
    pub selected: usize,
}

pub enum CompleteKey {
    /// The key was used up (navigation, dismiss).
    Consumed,
    /// Replace the edited text with this.
    Accept(String),
    /// Not a completion key: handle it as typing.
    Pass,
}

/// Text after the last unterminated `{{`, if the caret (end of text) is inside one.
pub fn open_var_prefix(text: &str) -> Option<&str> {
    let i = text.rfind("{{")?;
    let rest = &text[i + 2..];
    let escaped = i > 0 && text.as_bytes()[i - 1] == b'\\';
    (!rest.contains("}}") && !escaped).then_some(rest.trim_start())
}

/// `text` with the open `{{prefix` replaced by `{{name}}`.
pub fn complete_text(text: &str, name: &str) -> String {
    match text.rfind("{{") {
        Some(i) => format!("{}{{{{{name}}}}}", &text[..i]),
        None => text.to_string(),
    }
}

/// How a variable reference looks in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarInfo {
    pub resolved: bool,
    /// Hover text: value (masked when secret) and where it came from.
    pub tip: String,
}

type CacheKey = (String, Option<String>, u64, String);

#[derive(Default)]
pub struct EnvStore {
    /// Active environment name, remembered per workspace in the session file.
    pub active: Option<String>,
    pub settings: UserSettings,
    /// One jar and token cache per (collection, environment).
    pub states: HashMap<(PathBuf, String), EnvState>,
    /// Runtime variables (highest precedence); name → (value, secret).
    pub runtime: BTreeMap<String, (String, bool)>,
    /// Secret reveal deadlines, keyed by variable name.
    pub reveal: HashMap<String, Instant>,
    /// Tabs that already got the unresolved-variable warning.
    pub warned: HashSet<u64>,
    pub loaded: HashMap<String, Environment>,
    pub tracked: HashMap<String, bool>,
    pub edit: Option<EnvEdit>,
    pub completion: Option<Completion>,
    /// Requests whose OAuth2 token is being fetched (tab ids).
    pub fetching: HashSet<u64>,
    pub names_cache: RefCell<Option<(PathBuf, Vec<String>)>>,
    scope_cache: RefCell<Option<(CacheKey, Rc<VarScope>)>>,
    /// Bumped when anything a scope depends on may have changed on disk or in memory.
    pub generation: u64,
}

impl EnvStore {
    pub fn invalidate(&mut self) {
        self.generation += 1;
        *self.names_cache.borrow_mut() = None;
    }

    pub fn is_revealed(&self, var: &str) -> bool {
        self.reveal.get(var).is_some_and(|t| *t > Instant::now())
    }
}

fn hash(s: &str) -> u64 {
    // FNV-1a, enough to name files.
    s.bytes().fold(14695981039346656037, |h, b| {
        (h ^ b as u64).wrapping_mul(1099511628211)
    })
}

impl WorkspaceView {
    /// Root of the collection whose environments the picker shows: the active request's
    /// collection, else the first one.
    pub fn env_root(&self) -> Option<PathBuf> {
        let p = self.scope.workspace.as_ref()?;
        if let Some(key) = self.active_key() {
            let abs = p.abs_of(&key);
            if let Some(c) = p
                .collections
                .iter()
                .filter(|c| abs.starts_with(&c.root))
                .max_by_key(|c| c.root.components().count())
            {
                return Some(c.root.clone());
            }
        }
        p.collections.first().map(|c| c.root.clone())
    }

    pub fn env_names(&self) -> Vec<String> {
        let Some(root) = self.env_root() else {
            return Vec::new();
        };
        let mut cache = self.scope.envs.names_cache.borrow_mut();
        if let Some((r, n)) = &*cache
            && *r == root
        {
            return n.clone();
        }
        let names = list_environments(&root);
        *cache = Some((root, names.clone()));
        names
    }

    /// Name shown in the title bar.
    pub fn env_label(&self) -> String {
        self.scope
            .envs
            .active
            .clone()
            .unwrap_or_else(|| "No environment".into())
    }

    /// Pick the active environment (`None` clears it). Remembered in the session.
    pub fn set_active_env(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.scope.envs.active = name;
        self.scope.envs.invalidate();
        self.session_dirty = true;
        cx.notify();
    }

    /// After opening a workspace: keep the remembered environment if it still exists, else
    /// fall back to the first one.
    pub fn init_active_env(&mut self, remembered: Option<String>) {
        self.scope.envs.invalidate();
        self.scope.envs.loaded.clear();
        let names = self.env_names();
        self.scope.envs.active = remembered
            .filter(|n| names.contains(n))
            .or_else(|| names.first().cloned());
    }

    fn secret_names(&self, root: &Path) -> Vec<String> {
        self.scope
            .workspace
            .as_ref()
            .and_then(|p| p.collections.iter().find(|c| c.root == root))
            .map(|c| c.collection.config.secrets.names.clone())
            .unwrap_or_default()
    }

    /// Cookie jar and token cache for the active environment of `root`.
    pub fn env_state(&mut self, root: &Path) -> Option<EnvState> {
        let name = self.scope.envs.active.clone()?;
        let persist = self.scope.envs.persist_enabled();
        let dir = self.state_dir.clone();
        let st = self
            .scope
            .envs
            .states
            .entry((root.to_path_buf(), name.clone()))
            .or_insert_with(|| {
                let s = EnvState::new(name.clone());
                if persist
                    && let Some(d) = &dir
                    && let Ok(t) = std::fs::read_to_string(cookie_file(d, root, &name))
                {
                    s.cookies.load_json(&t);
                }
                s
            })
            .clone();
        Some(st)
    }

    /// Write the jar of the active environment when `persist_cookies` is on.
    pub fn persist_cookies(&self, root: &Path) {
        let (Some(name), Some(dir)) = (self.scope.envs.active.clone(), self.state_dir.clone())
        else {
            return;
        };
        if !self.scope.envs.persist_enabled() {
            return;
        }
        if let Some(st) = self
            .scope
            .envs
            .states
            .get(&(root.to_path_buf(), name.clone()))
        {
            let f = cookie_file(&dir, root, &name);
            if let Some(p) = f.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let _ = std::fs::write(f, st.cookies.to_json());
        }
    }

    /// Root of the collection that contains `key`.
    pub fn collection_root_of(&self, key: &str) -> Option<PathBuf> {
        let p = self.scope.workspace.as_ref()?;
        let abs = p.abs_of(key);
        p.collections
            .iter()
            .filter(|c| abs.starts_with(&c.root))
            .max_by_key(|c| c.root.components().count())
            .map(|c| c.root.clone())
    }

    fn make_scope(&self, key: &str) -> Option<VarScope> {
        let p = self.scope.workspace.as_ref()?;
        let doc = p.doc(key)?;
        let root = self.collection_root_of(key)?;
        let abs = p.abs_of(key);
        let globals: Vec<(String, String)> = self
            .scope
            .envs
            .settings
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Some(build_scope(&ScopeInputs {
            collection_root: &root,
            file: &abs,
            doc: &doc.parsed,
            env: self.scope.envs.active.as_deref(),
            runtime: &self.scope.envs.runtime,
            globals: &globals,
            allow_process_env: self.scope.envs.settings.allow_process_env,
        }))
    }

    /// URL a new request in `dir` starts with: `{{baseUrl}}/` (`wss://{{host}}/` for WebSocket)
    /// only when that variable is defined for the active environment, otherwise empty.
    pub fn default_url_for(&self, dir: &Path, websocket: bool) -> String {
        let var = if websocket { "host" } else { "baseUrl" };
        let Some(ws) = self.scope.workspace.as_ref() else {
            return String::new();
        };
        let Some(root) = ws
            .collections
            .iter()
            .filter(|c| dir.starts_with(&c.root))
            .max_by_key(|c| c.root.components().count())
            .map(|c| c.root.clone())
        else {
            return String::new();
        };
        let probe = dir.join("new.flux.md");
        let doc = fluxwire_format::parse(&fluxwire_format::new_request("x", "", "GET", ""));
        let globals: Vec<(String, String)> = self
            .scope
            .envs
            .settings
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let scope = build_scope(&ScopeInputs {
            collection_root: &root,
            file: &probe,
            doc: &doc,
            env: self.scope.envs.active.as_deref(),
            runtime: &self.scope.envs.runtime,
            globals: &globals,
            allow_process_env: self.scope.envs.settings.allow_process_env,
        });
        if scope.var(var).is_some() {
            fluxwire_format::default_request_url(websocket).to_string()
        } else {
            String::new()
        }
    }

    /// The variable scope for `key`, cached until something it depends on changes.
    pub fn scope_for(&self, key: &str) -> Option<Rc<VarScope>> {
        let doc = self.scope.workspace.as_ref()?.doc(key)?;
        let vars_sig = format!("{:?}", doc.parsed.front.get("vars").map(|e| &e.value));
        let ck: CacheKey = (
            key.to_string(),
            self.scope.envs.active.clone(),
            self.scope.envs.generation,
            vars_sig,
        );
        if let Some((k, s)) = &*self.scope.envs.scope_cache.borrow()
            && *k == ck
        {
            return Some(s.clone());
        }
        let s = Rc::new(self.make_scope(key)?);
        *self.scope.envs.scope_cache.borrow_mut() = Some((ck, s.clone()));
        Some(s)
    }

    /// Lookup used to colour and explain `{{name}}` references in the editor.
    pub fn var_info(&self, scope: &VarScope, name: &str) -> VarInfo {
        use fluxwire_vars::VarRefKind::*;
        let kind = refs(&format!("{{{{{name}}}}}"))
            .first()
            .map(|r| r.kind.clone());
        match kind {
            Some(Dynamic) => VarInfo {
                resolved: fluxwire_vars::interpolate(&format!("{{{{{name}}}}}"), scope)
                    .missing
                    .is_empty(),
                tip: format!("{{{{{name}}}}} · dynamic value, new on every send"),
            },
            Some(ProcessEnv) => {
                let ok = fluxwire_vars::interpolate(&format!("{{{{{name}}}}}"), scope)
                    .missing
                    .is_empty();
                VarInfo {
                    resolved: ok,
                    tip: if ok {
                        format!("{{{{{name}}}}} · process environment")
                    } else if !scope.allow_process_env {
                        format!(
                            "{{{{{name}}}}} · process.env is off; set allow_process_env = true in settings.toml"
                        )
                    } else {
                        format!("{{{{{name}}}}} · not set in the process environment")
                    },
                }
            }
            Some(Scheme) => VarInfo {
                resolved: false,
                tip: format!("{{{{{name}}}}} · no secret provider for this scheme in this version"),
            },
            _ => match scope.var(name) {
                Some(v) => {
                    let shown = if v.secret && !self.scope.envs.is_revealed(name) {
                        "••••••••  (secret)".to_string()
                    } else {
                        v.value.clone()
                    };
                    VarInfo {
                        resolved: true,
                        tip: format!("{{{{{name}}}}} = {shown}\n{}", v.source.describe()),
                    }
                }
                None => VarInfo {
                    resolved: false,
                    tip: format!(
                        "{{{{{name}}}}} is not defined{}",
                        self.scope
                            .envs
                            .active
                            .as_ref()
                            .map(|e| format!(" in {e} or its parents"))
                            .unwrap_or_else(|| " (no environment selected)".into())
                    ),
                },
            },
        }
    }

    /// Colouring and hover lookup for the request at `key`.
    pub fn var_look<'a>(&'a self, key: &str) -> impl Fn(&str) -> VarInfo + 'a {
        let scope = self.scope_for(key);
        move |name: &str| match &scope {
            Some(s) => self.var_info(s, name),
            None => VarInfo {
                resolved: false,
                tip: format!("{{{{{name}}}}} · open a workspace to resolve variables"),
            },
        }
    }

    /// Names that resolve in `scope`, for `{{` completion.
    pub fn var_names(&self, scope: &VarScope) -> Vec<String> {
        let mut n: Vec<String> = scope.vars.keys().cloned().collect();
        n.extend(
            [
                "$uuid",
                "$timestamp",
                "$isoTimestamp",
                "$randomInt",
                "$randomString(16)",
                "$base64()",
            ]
            .map(String::from),
        );
        n
    }

    /// Recompute `{{` suggestions after the edited text of `target` changed.
    pub fn refresh_completion(&mut self, target: CompleteTarget, text: &str) {
        let prefix = open_var_prefix(text).map(str::to_lowercase);
        let names = prefix.as_ref().and_then(|p| {
            let scope = self.active_key().and_then(|k| self.scope_for(&k))?;
            let mut v: Vec<String> = self
                .var_names(&scope)
                .into_iter()
                .filter(|n| n.to_lowercase().contains(p.as_str()))
                .collect();
            v.sort_by_key(|n| (!n.to_lowercase().starts_with(p.as_str()), n.clone()));
            v.truncate(8);
            (!v.is_empty()).then_some(v)
        });
        self.scope.envs.completion = names.map(|names| Completion {
            target,
            names,
            selected: 0,
        });
    }

    /// Route a key press to the open completion list, if there is one for `target`.
    pub fn completion_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        target: CompleteTarget,
        text: &str,
    ) -> CompleteKey {
        let Some(c) = self
            .scope
            .envs
            .completion
            .as_mut()
            .filter(|c| c.target == target)
        else {
            return CompleteKey::Pass;
        };
        match ev.keystroke.key.as_str() {
            "down" => c.selected = (c.selected + 1).min(c.names.len() - 1),
            "up" => c.selected = c.selected.saturating_sub(1),
            "escape" => self.scope.envs.completion = None,
            "tab" | "enter" => {
                let name = c.names[c.selected].clone();
                self.scope.envs.completion = None;
                return CompleteKey::Accept(complete_text(text, &name));
            }
            _ => return CompleteKey::Pass,
        }
        CompleteKey::Consumed
    }

    /// Clicking a suggestion: finish the open `{{prefix` in the URL bar or the edited cell.
    pub fn accept_completion_click(
        &mut self,
        target: CompleteTarget,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        self.scope.envs.completion = None;
        match target {
            CompleteTarget::Url => {
                let Some(key) = self.active_key() else { return };
                let cur = self
                    .scope
                    .workspace
                    .as_ref()
                    .and_then(|p| p.doc(&key))
                    .and_then(|d| d.parsed.request().map(|r| r.target.clone()))
                    .unwrap_or_default();
                self.set_url_text(&key, &complete_text(&cur, name), cx);
            }
            CompleteTarget::Cell => {
                if let Some(c) = self.scope.cell.as_mut() {
                    c.text = complete_text(&c.text, name);
                    self.apply_cell_text(cx);
                }
            }
        }
        cx.notify();
    }

    /// Workspace diagnostics plus a warning per unresolved variable in each open request.
    pub fn all_problems(&self) -> Vec<crate::workspace_model::Problem> {
        let Some(p) = self.scope.workspace.as_ref() else {
            return Vec::new();
        };
        let mut out = p.problems();
        for t in self.scope.tabs.all() {
            if !matches!(t.kind, crate::tabs::TabKind::Http { .. }) {
                continue;
            }
            let Some(doc) = p.doc(&t.key) else { continue };
            for name in self.unresolved_in(&t.key) {
                let needle = format!("{{{{{name}}}}}");
                let start = doc.text.find(&needle).unwrap_or(0);
                let before = &doc.text[..start];
                let line = before.matches('\n').count() + 1;
                let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                out.push(crate::workspace_model::Problem {
                    key: t.key.clone(),
                    file: t.key.clone(),
                    line,
                    col,
                    span: fluxwire_model::Span { start, end: start + needle.len() },
                    code: "V101",
                    severity: fluxwire_model::Severity::Warning,
                    message: format!("Unresolved variable {needle}: not defined in the active environment or its parents."),
                });
            }
        }
        // Script failures of each open request's latest send (S001…), at their `.flux.md` line.
        for t in self.scope.tabs.all() {
            let Some(res) = self
                .scope
                .responses
                .get(&t.id)
                .and_then(|r| r.result.as_ref())
            else {
                continue;
            };
            let Some(doc) = p.doc(&t.key) else { continue };
            let pre = match &res.error {
                Some(fluxwire_runner::RunError::Script(es)) => es.as_slice(),
                _ => &[],
            };
            for e in res.script_errors.iter().chain(pre) {
                let own = t.key.ends_with(e.file.as_str());
                let (start, end, line) = if own && e.line > 0 {
                    let mut start = 0;
                    for _ in 1..e.line {
                        match doc.text[start..].find('\n') {
                            Some(n) => start += n + 1,
                            None => break,
                        }
                    }
                    let end = doc.text[start..]
                        .find('\n')
                        .map_or(doc.text.len(), |n| start + n);
                    (start, end, e.line as usize)
                } else {
                    (0, 0, e.line.max(1) as usize)
                };
                out.push(crate::workspace_model::Problem {
                    key: if own { t.key.clone() } else { e.file.clone() },
                    file: e.file.clone(),
                    line,
                    col: 1,
                    span: fluxwire_model::Span { start, end },
                    code: e.kind.code(),
                    severity: fluxwire_model::Severity::Error,
                    message: e.message.clone(),
                });
            }
        }
        for i in &self.keymap_issues {
            out.push(crate::workspace_model::Problem {
                key: "keymap.toml".into(),
                file: "keymap.toml".into(),
                line: i.line.max(1),
                col: 1,
                span: fluxwire_model::Span { start: 0, end: 0 },
                code: i.code,
                severity: fluxwire_model::Severity::Warning,
                message: i.message.clone(),
            });
        }
        out.sort_by(|a, b| (a.key.as_str(), a.line).cmp(&(b.key.as_str(), b.line)));
        out
    }

    pub fn problem_total(&self) -> usize {
        self.all_problems()
            .iter()
            .filter(|p| p.severity != fluxwire_model::Severity::Info)
            .count()
    }

    /// Unresolved variables in the request as currently edited.
    pub fn unresolved_in(&self, key: &str) -> Vec<String> {
        let (Some(doc), Some(scope)) = (
            self.scope.workspace.as_ref().and_then(|p| p.doc(key)),
            self.scope_for(key),
        ) else {
            return Vec::new();
        };
        let mut texts: Vec<String> = Vec::new();
        if let Some(r) = doc.parsed.request() {
            texts.push(r.target.clone());
            texts.extend(
                r.query
                    .iter()
                    .filter(|q| q.enabled)
                    .flat_map(|q| [q.name.clone()].into_iter().chain(q.value.clone())),
            );
            texts.extend(
                r.headers
                    .iter()
                    .filter(|h| h.enabled)
                    .map(|h| h.value.clone()),
            );
            if let fluxwire_model::Body::Raw(span) = &r.body {
                texts.push(span.slice(&doc.parsed.source).to_string());
            }
        }
        for k in ["auth", "path"] {
            if let Some(fluxwire_model::FmValue::Map(m)) = doc.parsed.front.get(k).map(|e| &e.value)
            {
                texts.extend(m.iter().map(|(_, v)| v.clone()));
            }
        }
        let mut out: Vec<String> = texts
            .iter()
            .flat_map(|t| fluxwire_vars::interpolate(t, &scope).missing)
            .collect();
        out.sort();
        out.dedup();
        out
    }

    // ---- environment files ------------------------------------------------------------------

    /// Load (or reload) the rows of `name` for the Environment tab.
    pub fn load_env(&mut self, name: &str) {
        let Some(root) = self.env_root() else { return };
        let secrets = self.secret_names(&root);
        let env = load_environment(&root, name, &secrets);
        self.scope
            .envs
            .tracked
            .insert(name.to_string(), local_env_is_tracked(&root, name));
        self.scope.envs.loaded.insert(name.to_string(), env);
    }

    /// Variables of `name` for the panel, from the loaded rows or straight from disk.
    pub fn env_var_views(&self, name: &str) -> Vec<EnvVarView> {
        let rows = match self.scope.envs.loaded.get(name) {
            Some(e) => e.rows.clone(),
            None => self
                .env_root()
                .map(|r| load_environment(&r, name, &self.secret_names(&r)).rows)
                .unwrap_or_default(),
        };
        rows.into_iter()
            .map(|r| EnvVarView {
                name: r.name,
                value: r.value,
                secret: r.secret,
            })
            .collect()
    }

    pub fn env_rows(&self, name: &str) -> Vec<EnvRow> {
        self.scope
            .envs
            .loaded
            .get(name)
            .map(|e| e.rows.clone())
            .unwrap_or_default()
    }

    fn save_env(&mut self, name: &str) {
        let (Some(root), Some(env)) = (self.env_root(), self.scope.envs.loaded.get(name)) else {
            return;
        };
        if let Err(e) = save_environment(&root, env) {
            self.notice = Some(format!("Could not save environment {name}: {e}"));
        }
        self.scope.envs.invalidate();
    }

    pub fn start_env_edit(
        &mut self,
        env: &str,
        row: usize,
        col: EnvCol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_if_missing(env);
        let rows = self.env_rows(env);
        let text = rows
            .get(row)
            .map(|r| {
                if col == EnvCol::Name {
                    r.name.clone()
                } else {
                    r.value.clone()
                }
            })
            .unwrap_or_default();
        self.start_field_edit(
            EditTarget::Env {
                env: env.to_string(),
                row,
                col,
            },
            text,
            window,
            cx,
        );
    }

    pub fn start_field_edit(
        &mut self,
        target: EditTarget,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scope.envs.edit = Some(EnvEdit {
            target,
            text: TextEdit::new(text),
        });
        window.focus(&self.env_focus);
        cx.notify();
    }

    fn load_if_missing(&mut self, env: &str) {
        if !self.scope.envs.loaded.contains_key(env) {
            self.load_env(env);
        }
    }

    /// Typing in the edited field; Enter/Escape finish it, Tab moves along an Environment row.
    pub fn on_env_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut edit) = self.scope.envs.edit.clone() else {
            return;
        };
        match ev.keystroke.key.as_str() {
            "escape" | "enter" => {
                self.scope.envs.edit = None;
                window.focus(&self.root_focus);
                cx.notify();
                return;
            }
            "tab" => {
                if let EditTarget::Env { env, row, col } = &edit.target {
                    let next = if *col == EnvCol::Name {
                        (*row, EnvCol::Value)
                    } else {
                        (row + 1, EnvCol::Name)
                    };
                    let env = env.clone();
                    self.start_env_edit(&env, next.0, next.1, window, cx);
                }
                return;
            }
            _ => {}
        }
        let before = edit.text.text().to_string();
        if !edit.text.handle_key(ev) {
            return;
        }
        let value = edit.text.text().to_string();
        if value == before {
            // Only the caret or selection moved.
            cx.notify();
            return;
        }
        match edit.target.clone() {
            EditTarget::Env { env, row, col } => {
                self.load_if_missing(&env);
                if let Some(e) = self.scope.envs.loaded.get_mut(&env) {
                    while e.rows.len() <= row {
                        e.rows.push(EnvRow {
                            name: String::new(),
                            value: String::new(),
                            secret: false,
                            local: false,
                            line: 0,
                        });
                    }
                    let r = &mut e.rows[row];
                    if col == EnvCol::Name {
                        r.name = value;
                    } else {
                        r.value = value;
                    }
                }
                self.save_env(&env);
            }
            EditTarget::Auth { key, field } => self.set_auth_field(&key, &field, &value, cx),
            EditTarget::Cookie { idx } => {
                if let Some(root) = self.env_root()
                    && let Some(st) = self.env_state(&root)
                    && let Some(mut c) = st.cookies.list().into_iter().nth(idx)
                {
                    c.value = value;
                    st.cookies.set(c);
                    self.persist_cookies(&root);
                }
            }
        }
        edit.text = edit.text.clone();
        self.scope.envs.edit = Some(edit);
        cx.notify();
    }

    pub fn toggle_env_secret(&mut self, env: &str, row: usize, cx: &mut Context<Self>) {
        self.load_if_missing(env);
        if let Some(r) = self
            .scope
            .envs
            .loaded
            .get_mut(env)
            .and_then(|e| e.rows.get_mut(row))
        {
            r.secret = !r.secret;
            r.local = r.secret;
        }
        self.save_env(env);
        self.load_env(env);
        cx.notify();
    }

    pub fn delete_env_row(&mut self, env: &str, row: usize, cx: &mut Context<Self>) {
        if let Some(e) = self.scope.envs.loaded.get_mut(env)
            && row < e.rows.len()
        {
            e.rows.remove(row);
        }
        self.scope.envs.edit = None;
        self.save_env(env);
        self.load_env(env);
        cx.notify();
    }

    /// Click-to-reveal for 10 s (SPEC §8.3).
    pub fn reveal_secret(&mut self, var: &str, cx: &mut Context<Self>) {
        self.scope
            .envs
            .reveal
            .insert(var.to_string(), Instant::now() + REVEAL_FOR);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(REVEAL_FOR + Duration::from_millis(100))
                .await;
            this.update(cx, |this, cx| {
                let now = Instant::now();
                this.scope.envs.reveal.retain(|_, t| *t > now);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn clear_runtime_vars(&mut self, cx: &mut Context<Self>) {
        self.scope.envs.runtime.clear();
        self.scope.envs.invalidate();
        cx.notify();
    }

    // ---- environment operations -------------------------------------------------------------

    pub fn start_new_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = Some(PromptPurpose::NewEnvironment);
        self.open_modal_keep_prompt(window, cx);
    }

    pub fn start_rename_env(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt = Some(PromptPurpose::RenameEnvironment {
            name: name.to_string(),
        });
        self.open_modal_keep_prompt(window, cx);
        if let Some(m) = self.modal.as_mut() {
            m.query.insert(name);
        }
    }

    pub fn duplicate_env(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(root) = self.env_root() else { return };
        match duplicate_environment(&root, name) {
            Ok(n) => self.after_env_op(Some(n)),
            Err(e) => self.notice = Some(format!("Could not duplicate {name}: {e}")),
        }
        cx.notify();
    }

    pub fn delete_env(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(root) = self.env_root() else { return };
        match delete_environment(&root, name) {
            Ok(()) => {
                if self.scope.envs.active.as_deref() == Some(name) {
                    self.scope.envs.active = None;
                }
                self.scope.tabs.close_key(&format!("env/{name}"));
                self.after_env_op(None);
                if self.scope.envs.active.is_none() {
                    self.scope.envs.active = self.env_names().first().cloned();
                }
            }
            Err(e) => self.notice = Some(format!("Could not delete {name}: {e}")),
        }
        cx.notify();
    }

    /// Run after the prompt modal closes: create or rename.
    pub fn finish_env_prompt(&mut self, purpose: PromptPurpose, name: &str) -> bool {
        let Some(root) = self.env_root() else {
            return false;
        };
        let name = name.trim();
        let bad = name.is_empty()
            || name.contains(['/', '\\', '.'])
            || name.contains(char::is_whitespace);
        if bad {
            self.notice = Some("Environment names are one word without dots or slashes.".into());
            return true;
        }
        let r = match purpose {
            PromptPurpose::NewEnvironment => create_environment(&root, name).map(|_| None),
            PromptPurpose::RenameEnvironment { name: old } => {
                rename_environment(&root, &old, name).map(|_| Some(old))
            }
            _ => return false,
        };
        match r {
            Ok(old) => {
                if let Some(old) = old {
                    if self.scope.envs.active.as_deref() == Some(old.as_str()) {
                        self.scope.envs.active = Some(name.to_string());
                    }
                    self.scope.tabs.close_key(&format!("env/{old}"));
                }
                self.after_env_op(Some(name.to_string()));
            }
            Err(e) => self.notice = Some(format!("Operation failed: {e}")),
        }
        true
    }

    fn after_env_op(&mut self, select: Option<String>) {
        self.scope.envs.invalidate();
        self.scope.envs.loaded.clear();
        if let Some(n) = select {
            self.scope.envs.active = Some(n);
        }
        self.session_dirty = true;
    }

    /// File-system batch touched `environments/` or a `.flux.md`: drop caches.
    pub fn on_env_fs_change(&mut self, batch: &[PathBuf]) {
        let touched = batch.iter().any(|p| {
            p.components().any(|c| c.as_os_str() == "environments")
                || p.file_name().is_some_and(|n| {
                    n.to_string_lossy().ends_with(".flux.md") || n == "fluxwire.toml"
                })
        });
        if touched {
            self.scope.envs.invalidate();
            if self.scope.envs.edit.is_none() {
                self.scope.envs.loaded.clear();
            }
            // The active environment may have been deleted or renamed outside the app.
            let names = self.env_names();
            if self
                .scope
                .envs
                .active
                .as_ref()
                .is_some_and(|a| !names.contains(a))
            {
                self.scope.envs.active = names.first().cloned();
            }
        }
    }

    pub fn open_env_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_modal(ModalKind::Environment, window, cx);
    }
}

impl EnvStore {
    pub fn persist_enabled(&self) -> bool {
        self.settings.persist_cookies
    }
}

pub fn cookie_file(state_dir: &Path, root: &Path, env: &str) -> PathBuf {
    state_dir
        .join("cookies")
        .join(format!("{:016x}-{env}.json", hash(&root.to_string_lossy())))
}
