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
    ScopeInputs, UserSettings, build_scope, delete_environment, duplicate_environment,
    list_environments, local_env_is_tracked, rename_environment,
};
use fluxwire_protocol::EnvState;
use fluxwire_vars::{VarScope, refs};
use gpui::{Context, Window};

use crate::workspace::{ItemRun, ModalItem, ModalKind, PromptPurpose, WorkspaceView, fuzzy};

/// How long a revealed secret stays visible (SPEC §8.3).
pub const REVEAL_FOR: Duration = Duration::from_secs(10);

/// User config directory (`settings.toml`). Unset in tests.
#[derive(Clone)]
pub struct ConfigDir(pub PathBuf);
impl gpui::Global for ConfigDir {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// A field of the `auth` front matter map of a request.
    Auth { key: String, field: String },
    /// The value of a cookie in the active environment's jar (index into `CookieJar::list`).
    Cookie { idx: usize },
}

/// The single-line field being typed into on the Environment and Auth surfaces.
#[derive(Clone, Debug)]
pub struct EnvEdit {
    pub target: EditTarget,
    pub text: gpui::Entity<crate::text_input::TextInput>,
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

/// (collection, environment, row id, is value) of an Environment editor cell input.
pub type CellKey = (PathBuf, String, u64, bool);

type CacheKey = (String, Option<String>, u64, String);

#[derive(Default)]
pub struct EnvStore {
    /// Active environment of the Workspace, remembered in the session.
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
    /// Cached E106 check (`*.local.env` is git-ignored); cleared by `invalidate`.
    pub ignored: std::cell::Cell<Option<bool>>,
    /// Open environment documents (M9.02): in-memory rows, dirty state, undo.
    pub docs: HashMap<(PathBuf, String), crate::env_doc::EnvDoc>,
    pub tracked: HashMap<(PathBuf, String), bool>,
    /// Name/Value inputs of the editor tabs, keyed by (collection, environment, row id, is value).
    pub cells: RefCell<HashMap<CellKey, gpui::Entity<crate::text_input::TextInput>>>,
    /// Filter inputs of the editor tabs.
    pub filters: RefCell<HashMap<(PathBuf, String), gpui::Entity<crate::text_input::TextInput>>>,
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
        self.ignored.set(None);
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
    /// Directory that holds `environments/`: the Workspace root. Environments are
    /// workspace-level (ADR-012).
    pub fn env_root(&self) -> Option<PathBuf> {
        self.scope.workspace.as_ref().map(|p| p.root.clone())
    }

    /// Why `name` cannot name an environment of `root` (None when it can).
    pub fn env_name_error(&self, root: &Path, name: &str, except: Option<&str>) -> Option<String> {
        let existing: Vec<String> = list_environments(root)
            .into_iter()
            .filter(|n| Some(n.as_str()) != except)
            .collect();
        fluxwire_format::validate_environment_name(
            name,
            &existing,
            cfg!(any(target_os = "macos", target_os = "windows")),
        )
        .err()
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

    /// The environment an Environment action applies to: the open Environment tab's, else the
    /// active environment of the workspace.
    pub fn env_target(&self) -> Option<(PathBuf, String)> {
        if let Some(t) = self.scope.tabs.active()
            && let Some(k) = self.parse_env_tab_key(&t.key)
        {
            return Some(k);
        }
        Some((self.env_root()?, self.active_env_name()?))
    }

    /// Whether `*.local.env` is git-ignored for the workspace (cached until something changes).
    pub fn ignores_local_env_files(&self) -> bool {
        if let Some(v) = self.scope.envs.ignored.get() {
            return v;
        }
        let v = self
            .env_root()
            .is_none_or(|r| fluxwire_format::gitignore_ignores_local_env(&r));
        self.scope.envs.ignored.set(Some(v));
        v
    }

    /// Name shown in the title bar.
    pub fn env_label(&self) -> String {
        self.active_env_name()
            .unwrap_or_else(|| "No environment".into())
    }

    /// Header of the switcher.
    pub fn env_modal_title(&self) -> Option<String> {
        (self.modal.as_ref()?.kind == ModalKind::Environment).then(|| "Switch environment".into())
    }

    /// Detail text of an environment row: the `baseUrl` host, else the variable count.
    fn env_detail(&self, root: &Path, name: &str) -> String {
        let rows = self
            .scope
            .envs
            .docs
            .get(&(root.to_path_buf(), name.to_string()))
            .map(|d| d.env.rows.clone())
            .unwrap_or_else(|| {
                fluxwire_format::load_environment(root, name, &self.secret_names(root)).rows
            });
        let host = rows
            .iter()
            .find(|r| r.name == "baseUrl")
            .map(|r| {
                let v = r.value.split("://").nth(1).unwrap_or(&r.value);
                v.split('/').next().unwrap_or(v).to_string()
            })
            .filter(|h| !h.is_empty());
        host.unwrap_or_else(|| {
            let n = rows.len();
            format!("{n} variable{}", if n == 1 { "" } else { "s" })
        })
    }

    /// Rows of the environment switcher (B1).
    pub(crate) fn env_switcher_items(&self, q: &str) -> Vec<ModalItem> {
        let item = |label: String, detail: String, run: ItemRun| ModalItem {
            id: String::new(),
            label,
            detail,
            indices: Vec::new(),
            run,
        };
        let Some(root) = self.env_root() else {
            return Vec::new();
        };
        let active = self.active_env_name();
        let names = list_environments(&root);
        let matching: Vec<&String> = names.iter().filter(|n| fuzzy(q, n)).collect();
        let mut v = Vec::new();
        if !q.is_empty() && matching.is_empty() && self.env_name_error(&root, q, None).is_none() {
            v.push(item(
                format!("Create environment “{q}”"),
                String::new(),
                ItemRun::EnvCreate {
                    root: root.clone(),
                    name: q.to_string(),
                },
            ));
        }
        v.push(item(
            "New environment…".into(),
            String::new(),
            ItemRun::EnvNew(root.clone()),
        ));
        for n in matching {
            let mut detail = self.env_detail(&root, n);
            if self
                .scope
                .envs
                .tracked
                .get(&(root.clone(), n.clone()))
                .copied()
                .unwrap_or_else(|| local_env_is_tracked(&root, n))
            {
                detail = format!("⚠ .local.env tracked by git · {detail}");
            }
            if active.as_deref() == Some(n.as_str()) {
                detail = format!("active · {detail}");
            }
            let label = if active.as_deref() == Some(n.as_str()) {
                format!("✓ {n}")
            } else {
                n.clone()
            };
            v.push(item(
                label,
                detail,
                ItemRun::EnvActivate {
                    root: root.clone(),
                    name: Some(n.clone()),
                },
            ));
        }
        if fuzzy(q, "No environment") {
            v.push(item(
                "No environment".into(),
                if active.is_none() {
                    "active".into()
                } else {
                    String::new()
                },
                ItemRun::EnvActivate {
                    root: root.clone(),
                    name: None,
                },
            ));
        }
        v.push(item(
            "Manage environments…".into(),
            String::new(),
            ItemRun::EnvManage(root),
        ));
        v
    }

    /// Pick the active environment (`None` clears it). Remembered in the session.
    pub fn set_active_env(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.set_active_cur(name);
        self.scope.envs.invalidate();
        self.session_dirty = true;
        cx.notify();
    }

    /// After opening a workspace: keep the remembered environment if it still exists (or the
    /// explicit "No environment"), else fall back to the first one.
    pub fn init_active_env(&mut self, remembered: Option<&str>, none: bool) {
        self.scope.envs.invalidate();
        self.scope.envs.docs.clear();
        self.scope.envs.cells.borrow_mut().clear();
        self.scope.envs.filters.borrow_mut().clear();
        let names = self
            .env_root()
            .map(|r| list_environments(&r))
            .unwrap_or_default();
        self.scope.envs.active = if none {
            None
        } else {
            remembered
                .filter(|n| names.iter().any(|x| x == n))
                .map(str::to_string)
                .or_else(|| names.first().cloned())
        };
    }

    /// Active environment of the workspace.
    pub fn active_env_name(&self) -> Option<String> {
        self.scope.envs.active.clone()
    }

    fn set_active_cur(&mut self, name: Option<String>) {
        self.scope.envs.active = name;
    }

    /// An active environment that no longer exists falls back to the first one.
    fn repair_active_envs(&mut self) {
        let names = self
            .env_root()
            .map(|r| list_environments(&r))
            .unwrap_or_default();
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

    /// `[secrets]` names of the collections (environments are workspace-level, so a name that
    /// any collection marks secret is secret).
    pub(crate) fn secret_names(&self, _root: &Path) -> Vec<String> {
        let mut v: Vec<String> = self
            .scope
            .workspace
            .as_ref()
            .map(|p| {
                p.collections
                    .iter()
                    .flat_map(|c| c.collection.config.secrets.names.clone())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v.dedup();
        v
    }

    /// Cookie jar and token cache for the active environment of `root`.
    pub fn env_state(&mut self, root: &Path) -> Option<EnvState> {
        let name = self.active_env_name()?;
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
        let (Some(name), Some(dir)) = (self.active_env_name(), self.state_dir.clone()) else {
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
        let env = self.active_env_name();
        let overlay = self.dirty_env_overlay();
        let env_root = self.env_root();
        Some(build_scope(&ScopeInputs {
            collection_root: &root,
            file: &abs,
            doc: &doc.parsed,
            env: env.as_deref(),
            env_root: env_root.as_deref(),
            env_overlay: overlay.as_ref(),
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
        let env = self.active_env_name();
        let env_root = self.env_root();
        let scope = build_scope(&ScopeInputs {
            collection_root: &root,
            file: &probe,
            doc: &doc,
            env: env.as_deref(),
            env_root: env_root.as_deref(),
            env_overlay: None,
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
            self.active_env_name(),
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
                        self.active_env_name()
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
                    let (input, text) = (c.input.clone(), c.text.clone());
                    input.update(cx, |i, cx| i.set_text(&text, cx));
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
        // Open environment documents: E101–E104 per row, E105/E106 per environment.
        let mut docs: Vec<_> = self.scope.envs.docs.iter().collect();
        docs.sort_by(|a, b| a.0.cmp(b.0));
        for ((root, name), d) in docs {
            let tab = self.env_tab_key(root, name);
            let secrets = self.secret_names(root);
            let mut push = |line: usize, code: &'static str, sev, message: String| {
                out.push(crate::workspace_model::Problem {
                    key: tab.clone(),
                    file: format!("environments/{name}.env"),
                    line,
                    col: 1,
                    span: fluxwire_model::Span { start: 0, end: 0 },
                    code,
                    severity: sev,
                    message,
                });
            };
            for dg in d.diagnostics(&secrets) {
                let sev = match dg.severity {
                    fluxwire_format::EnvSeverity::Error => fluxwire_model::Severity::Error,
                    fluxwire_format::EnvSeverity::Warning => fluxwire_model::Severity::Warning,
                    fluxwire_format::EnvSeverity::Info => fluxwire_model::Severity::Info,
                };
                let line = d.env.rows.get(dg.row).map_or(1, |r| r.line.max(dg.row + 1));
                push(line, dg.code, sev, dg.message);
            }
            if self
                .scope
                .envs
                .tracked
                .get(&(root.clone(), name.clone()))
                .copied()
                .unwrap_or(false)
            {
                push(
                    1,
                    "E105",
                    fluxwire_model::Severity::Warning,
                    format!(
                        "{name}.local.env is tracked by git; its secrets may be published. Run: git rm --cached environments/{name}.local.env"
                    ),
                );
            }
            if !self.ignores_local_env_files() {
                push(
                    1,
                    "E106",
                    fluxwire_model::Severity::Warning,
                    "The workspace's .gitignore does not ignore *.local.env.".into(),
                );
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

    pub fn start_field_edit(
        &mut self,
        target: EditTarget,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = crate::text_input::bound_input("", cx, |this: &mut Self, value, cx| {
            this.apply_env_value(value, cx)
        });
        input.update(cx, |i, cx| i.set_text(&text, cx));
        window.focus(&gpui::Focusable::focus_handle(input.read(cx), cx));
        self.scope.envs.edit = Some(EnvEdit {
            target,
            text: input,
        });
        cx.notify();
    }

    /// Typing in the edited field; Enter/Escape finish it, Tab moves along an Environment row.
    pub fn on_env_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scope.envs.edit.is_none() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "escape" | "enter" => {
                self.scope.envs.edit = None;
                window.focus(&self.root_focus);
                cx.notify();
            }
            _ => {}
        }
    }

    /// The edited field's text changed: write it to its target.
    fn apply_env_value(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(edit) = self.scope.envs.edit.clone() else {
            return;
        };
        match edit.target.clone() {
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
        self.scope.envs.edit = Some(edit);
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

    pub fn start_rename_env(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root() {
            self.start_rename_env_in(&root, name, window, cx);
        }
    }

    pub fn start_rename_env_in(
        &mut self,
        root: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt = Some(PromptPurpose::RenameEnvironment {
            root: root.to_path_buf(),
            name: name.to_string(),
        });
        self.open_modal_keep_prompt(window, cx);
        self.prefill_modal(name, cx);
    }

    pub fn duplicate_env(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root() {
            self.duplicate_env_in(&root, name, cx);
        }
    }

    pub fn duplicate_env_in(&mut self, root: &Path, name: &str, cx: &mut Context<Self>) {
        match duplicate_environment(root, name) {
            Ok(n) => {
                self.after_env_op(None);
                self.open_env_doc(root, &n, true);
            }
            Err(e) => self.notice = Some(format!("Could not duplicate {name}: {e}")),
        }
        cx.notify();
    }

    /// Ask before deleting: the files that go are listed.
    pub fn confirm_delete_env(
        &mut self,
        root: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let files: Vec<String> = [
            fluxwire_format::env_path(root, name),
            fluxwire_format::local_env_path(root, name),
        ]
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| {
            format!(
                "environments/{}",
                p.file_name().unwrap_or_default().to_string_lossy()
            )
        })
        .collect();
        self.dialog = Some(crate::dialogs::Dialog::Confirm(crate::dialogs::Confirm {
            title: format!("Delete environment {name}?"),
            lines: vec![format!(
                "These files will be moved to the trash: {}.",
                files.join(", ")
            )],
            buttons: vec![
                (
                    "Move to trash".into(),
                    crate::dialogs::ConfirmAction::DeleteEnvironment {
                        root: root.to_path_buf(),
                        name: name.to_string(),
                    },
                ),
                ("Cancel".into(), crate::dialogs::ConfirmAction::Cancel),
            ],
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    pub fn delete_env(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root() {
            self.delete_env_in(&root, name, cx);
        }
    }

    pub fn delete_env_in(&mut self, root: &Path, name: &str, cx: &mut Context<Self>) {
        match delete_environment(root, name) {
            Ok(()) => {
                if self.active_env_name().as_deref() == Some(name) {
                    self.scope.envs.active = None;
                }
                let tab = self.env_tab_key(root, name);
                self.scope.tabs.close_key(&tab);
                self.discard_env_doc(&(root.to_path_buf(), name.to_string()));
                self.after_env_op(None);
            }
            Err(e) => self.notice = Some(format!("Could not delete {name}: {e}")),
        }
        cx.notify();
    }

    /// Run after the rename prompt closes. Returns true when handled.
    pub fn finish_env_prompt(&mut self, purpose: PromptPurpose, name: &str) -> bool {
        let PromptPurpose::RenameEnvironment { root, name: old } = purpose else {
            return false;
        };
        let name = name.trim();
        if name == old {
            return true;
        }
        if let Some(e) = self.env_name_error(&root, name, Some(&old)) {
            self.notice = Some(e);
            return true;
        }
        match rename_environment(&root, &old, name) {
            Ok(()) => {
                if self.active_env_name().as_deref() == Some(old.as_str()) {
                    self.scope.envs.active = Some(name.to_string());
                }
                let was_open = self
                    .scope
                    .tabs
                    .id_for_key(&self.env_tab_key(&root, &old))
                    .is_some();
                let tab = self.env_tab_key(&root, &old);
                self.scope.tabs.close_key(&tab);
                self.discard_env_doc(&(root.clone(), old));
                self.after_env_op(None);
                if was_open {
                    self.open_env_doc(&root, name, true);
                }
            }
            Err(e) => self.notice = Some(format!("Operation failed: {e}")),
        }
        true
    }

    fn after_env_op(&mut self, select: Option<String>) {
        self.scope.envs.invalidate();
        self.refresh_env_docs();
        if let Some(n) = select {
            self.set_active_cur(Some(n));
        }
        self.repair_active_envs();
        self.session_dirty = true;
    }

    /// File-system batch touched `environments/` or a `.flux.md`: drop caches.
    pub fn on_env_fs_change(&mut self, batch: &[PathBuf]) {
        let touched = batch.iter().any(|p| {
            p.components().any(|c| c.as_os_str() == "environments")
                || p.file_name().is_some_and(|n| {
                    n.to_string_lossy().ends_with(".flux.md")
                        || n == "fluxwire.toml"
                        || n == ".gitignore"
                })
        });
        if touched {
            self.scope.envs.invalidate();
            self.refresh_env_docs();
            // The active environment may have been deleted or renamed outside the app.
            self.repair_active_envs();
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
