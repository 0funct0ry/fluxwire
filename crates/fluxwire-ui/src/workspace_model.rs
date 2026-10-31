//! The open workspace: collections on disk, open documents, diagnostics index, session state.
//! No GPUI types here so the logic is unit-testable and survives the M1 view rewrite.

use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
};

use fluxwire_format::{
    Collection, OpenError, file_kind_for, find_collections, is_request_file, parse_with, rel_key,
    scan,
};
use fluxwire_model::{
    BlockKind, Diagnostic, Edit, FileKind, FluxDoc, RequestKind, Severity, Span, TreeNode,
    apply_edits, line_col,
};
use fluxwire_protocol::ProtocolRegistry;
use serde::{Deserialize, Serialize};

use crate::{
    editor::EditorState,
    highlight::{Spans, highlight_flux},
};

/// What an editor surface edits inside a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The whole file ("Open as text").
    Text,
    /// The request body.
    Body,
    /// Content of block `n`.
    Block(usize),
}

pub struct OpenDoc {
    pub abs: PathBuf,
    pub key: String,
    pub text: String,
    /// Last text known to be on disk.
    pub saved: String,
    pub parsed: FluxDoc,
    pub kind: FileKind,
    /// Disk text when the file changed underneath unsaved edits.
    pub conflict: Option<String>,
    pub missing: bool,
    pub text_mode: bool,
    pub highlight: Spans,
    pub editors: HashMap<Target, EditorState>,
}

impl OpenDoc {
    pub fn load(abs: &Path, key: &str) -> io::Result<OpenDoc> {
        let text = std::fs::read_to_string(abs)?;
        Ok(OpenDoc::from_text(abs, key, text.clone(), text))
    }

    pub fn from_text(abs: &Path, key: &str, text: String, saved: String) -> OpenDoc {
        let kind = file_kind_for(abs);
        let mut d = OpenDoc {
            abs: abs.to_path_buf(),
            key: key.to_string(),
            parsed: parse_with(&text, kind, &ProtocolRegistry::builtin()),
            highlight: Vec::new(),
            text,
            saved,
            kind,
            conflict: None,
            missing: false,
            text_mode: false,
            editors: HashMap::new(),
        };
        d.highlight = highlight_flux(&d.parsed);
        d
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.parsed = parse_with(&self.text, self.kind, &ProtocolRegistry::builtin());
        self.highlight = highlight_flux(&self.parsed);
        let t = &self.text;
        for e in self.editors.values_mut() {
            e.clamp(t);
        }
    }

    /// Apply span edits to the document. Returns false if the edits were invalid.
    pub fn apply(&mut self, edits: &[Edit]) -> bool {
        match apply_edits(&self.text, edits) {
            Some(new) => {
                self.set_text(new);
                true
            }
            None => false,
        }
    }

    pub fn title(&self) -> String {
        self.parsed.name().map_or_else(
            || {
                let f = self
                    .abs
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("untitled");
                f.trim_end_matches(".flux.md").to_string()
            },
            str::to_string,
        )
    }

    pub fn request_kind(&self) -> RequestKind {
        if !self.parsed.unsupported.is_empty() {
            RequestKind::Other
        } else if self.parsed.is_websocket() {
            RequestKind::WebSocket
        } else if self.parsed.request().is_some() {
            RequestKind::Http
        } else {
            RequestKind::Other
        }
    }

    pub fn method(&self) -> String {
        match self.request_kind() {
            RequestKind::WebSocket => "WS".into(),
            RequestKind::Http => self
                .parsed
                .request()
                .map_or("GET".into(), |r| r.method.to_ascii_uppercase()),
            RequestKind::Other => self
                .parsed
                .request()
                .map_or("?".into(), |r| r.method.to_ascii_uppercase()),
        }
    }

    /// Text of an editable surface.
    pub fn target_text(&self, t: Target) -> String {
        match t {
            Target::Text => self.text.clone(),
            Target::Body => match self.parsed.request().map(|r| &r.body) {
                Some(fluxwire_model::Body::Raw(s)) => strip_one_eol(s.slice(&self.text)),
                Some(fluxwire_model::Body::File { span, .. }) => {
                    strip_one_eol(span.slice(&self.text))
                }
                _ => String::new(),
            },
            Target::Block(i) => self
                .parsed
                .blocks
                .get(i)
                .map(|b| strip_one_eol(b.content.slice(&self.text)))
                .unwrap_or_default(),
        }
    }

    /// Replace an editable surface's text through the span editor.
    pub fn set_target_text(&mut self, t: Target, new: &str) {
        match t {
            Target::Text => self.set_text(new.to_string()),
            Target::Body => {
                let edits = fluxwire_format::set_body(&self.parsed, new);
                self.apply(&edits);
            }
            Target::Block(i) => {
                if i < self.parsed.blocks.len() {
                    let edits = fluxwire_format::set_block_content(&self.parsed, i, new);
                    self.apply(&edits);
                }
            }
        }
    }

    pub fn save(&mut self) -> io::Result<()> {
        std::fs::write(&self.abs, &self.text)?;
        self.saved = self.text.clone();
        self.conflict = None;
        self.missing = false;
        Ok(())
    }

    pub fn reload_from(&mut self, disk: String) {
        self.saved = disk.clone();
        self.conflict = None;
        self.missing = false;
        self.set_text(disk);
    }

    /// First block index with this kind and language, for the Scripts/Tests tabs.
    pub fn find_block(&self, kind: BlockKind, lang: Option<&str>) -> Option<usize> {
        self.parsed
            .blocks
            .iter()
            .position(|b| b.kind == kind && lang.is_none_or(|l| b.info.language == l))
    }
}

fn strip_one_eol(s: &str) -> String {
    s.strip_suffix('\n')
        .map(|x| x.strip_suffix('\r').unwrap_or(x))
        .unwrap_or(s)
        .to_string()
}

/// One row of the Problems panel.
#[derive(Clone, Debug)]
pub struct Problem {
    pub key: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub span: Span,
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// What quick search needs from a file beyond the tree: the URL, front matter tags and headings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SearchRow {
    url: String,
    tags: Vec<String>,
    headings: Vec<String>,
}

/// ATX headings of the prose, skipping front matter and fenced blocks.
fn headings_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim_end() == "---" {
                break;
            }
        }
    }
    for l in lines {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && t.starts_with('#') {
            let h = t.trim_start_matches('#');
            if h.starts_with(' ') && !h.trim().is_empty() {
                out.push(h.trim().to_string());
            }
        }
    }
    out
}

#[derive(Clone, Debug)]
struct IndexRow {
    span: Span,
    line: usize,
    col: usize,
    code: &'static str,
    severity: Severity,
    message: String,
}

fn rows(text: &str, diags: &[Diagnostic]) -> Vec<IndexRow> {
    diags
        .iter()
        .map(|d| {
            let (l, c) = line_col(text, d.span.start);
            IndexRow {
                span: d.span,
                line: l + 1,
                col: c + 1,
                code: d.code,
                severity: d.severity,
                message: d.message.clone(),
            }
        })
        .collect()
}

pub struct Workspace {
    pub root: PathBuf,
    pub name: String,
    pub collections: Vec<LoadedCollection>,
    /// Collections that could not be opened, with the reason (e.g. format too new).
    pub errors: Vec<(PathBuf, OpenError)>,
    /// Keys of collections removed from this Workspace. Collections are discovered on disk, so
    /// "remove" hides them (remembered in the session) instead of touching any file.
    pub hidden: std::collections::HashSet<String>,
    pub docs: HashMap<String, OpenDoc>,
    index: BTreeMap<String, Vec<IndexRow>>,
    /// Quick-search data per indexed file (URL, tags, doc headings).
    search_rows: BTreeMap<String, SearchRow>,
    pub branch: Option<String>,
}

pub struct LoadedCollection {
    pub root: PathBuf,
    /// Workspace-relative key of the collection directory ("" if it is the workspace root).
    pub key: String,
    pub collection: Collection,
    /// Tree with workspace-relative paths.
    pub tree: Vec<TreeNode>,
}

fn prefix_tree(nodes: &[TreeNode], prefix: &str) -> Vec<TreeNode> {
    let join = |p: &str| {
        if prefix.is_empty() {
            p.to_string()
        } else {
            format!("{prefix}/{p}")
        }
    };
    nodes
        .iter()
        .map(|n| match n {
            TreeNode::Folder {
                name,
                path,
                children,
            } => TreeNode::Folder {
                name: name.clone(),
                path: join(path),
                children: prefix_tree(children, prefix),
            },
            TreeNode::Request {
                name,
                path,
                method,
                kind,
            } => TreeNode::Request {
                name: name.clone(),
                path: join(path),
                method: method.clone(),
                kind: *kind,
            },
            TreeNode::Doc { name, path } => TreeNode::Doc {
                name: name.clone(),
                path: join(path),
            },
        })
        .collect()
}

pub fn read_git_branch(root: &Path) -> Option<String> {
    for dir in root.ancestors() {
        let head = dir.join(".git").join("HEAD");
        if let Ok(t) = std::fs::read_to_string(&head) {
            let t = t.trim();
            return Some(match t.strip_prefix("ref: refs/heads/") {
                Some(b) => b.to_string(),
                None => t.chars().take(7).collect(),
            });
        }
    }
    None
}

impl Workspace {
    /// Open a folder. Errors only if it is missing; a Collection that fails to load is kept in
    /// `errors` and the rest of the Workspace still opens.
    pub fn open(root: &Path) -> Result<Workspace, OpenError> {
        if !root.is_dir() {
            return Err(OpenError::NotFound(root.to_path_buf()));
        }
        let mut p = Workspace {
            root: root.to_path_buf(),
            name: root.file_name().map_or_else(
                || root.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            collections: vec![],
            errors: vec![],
            hidden: Default::default(),
            docs: HashMap::new(),
            index: BTreeMap::new(),
            search_rows: BTreeMap::new(),
            branch: read_git_branch(root),
        };
        p.rescan();
        Ok(p)
    }

    pub fn key_of(&self, abs: &Path) -> String {
        rel_key(&self.root, abs)
    }

    pub fn abs_of(&self, key: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in key.split('/').filter(|s| !s.is_empty()) {
            p.push(part);
        }
        p
    }

    /// Re-read collections and the diagnostics index from disk.
    pub fn rescan(&mut self) {
        self.index.clear();
        self.search_rows.clear();
        self.load_tree();
        let mut files = Vec::new();
        for c in &self.collections {
            collect_md(&c.root, &mut files);
        }
        for f in files {
            self.index_file(&f);
        }
    }

    fn load_tree(&mut self) {
        self.collections.clear();
        self.errors.clear();
        for dir in find_collections(&self.root) {
            if self.hidden.contains(&self.key_of(&dir)) {
                continue;
            }
            match scan(&dir) {
                Ok(c) => {
                    let key = self.key_of(&dir);
                    let tree = prefix_tree(&c.tree, &key);
                    self.collections.push(LoadedCollection {
                        root: dir,
                        key,
                        collection: c,
                        tree,
                    });
                }
                Err(e) => self.errors.push((dir, e)),
            }
        }
    }

    fn index_file(&mut self, abs: &Path) {
        let key = self.key_of(abs);
        match std::fs::read_to_string(abs) {
            Ok(text) => {
                let d = parse_with(&text, file_kind_for(abs), &ProtocolRegistry::builtin());
                let r = rows(&text, &d.diagnostics);
                self.search_rows.insert(
                    key.clone(),
                    SearchRow {
                        url: d.request().map(|r| r.target.clone()).unwrap_or_default(),
                        tags: d.front.list("tags"),
                        headings: headings_of(&text),
                    },
                );
                if r.is_empty() {
                    self.index.remove(&key);
                } else {
                    self.index.insert(key, r);
                }
            }
            Err(_) => {
                self.index.remove(&key);
                self.search_rows.remove(&key);
            }
        }
    }

    /// After file-system changes: reload the tree, but re-parse only the changed files and files
    /// not seen before; forget files that are gone.
    fn rescan_incremental(&mut self, changed: &[PathBuf]) {
        self.load_tree();
        let mut files = Vec::new();
        for c in &self.collections {
            collect_md(&c.root, &mut files);
        }
        let present: std::collections::HashSet<String> =
            files.iter().map(|f| self.key_of(f)).collect();
        let gone: Vec<String> = self
            .search_rows
            .keys()
            .filter(|k| !present.contains(*k))
            .cloned()
            .collect();
        for k in gone {
            self.search_rows.remove(&k);
            self.index.remove(&k);
        }
        for f in files {
            let key = self.key_of(&f);
            if !self.search_rows.contains_key(&key) || changed.contains(&f) {
                self.index_file(&f);
            }
        }
    }

    pub fn doc(&self, key: &str) -> Option<&OpenDoc> {
        self.docs.get(key)
    }

    pub fn doc_mut(&mut self, key: &str) -> Option<&mut OpenDoc> {
        self.docs.get_mut(key)
    }

    /// Load (or return the already-open) document for `key`.
    pub fn open_doc(&mut self, key: &str) -> io::Result<&mut OpenDoc> {
        if !self.docs.contains_key(key) {
            let abs = self.abs_of(key);
            let d = OpenDoc::load(&abs, key)?;
            self.docs.insert(key.to_string(), d);
        }
        Ok(self.docs.get_mut(key).expect("just inserted"))
    }

    pub fn close_doc(&mut self, key: &str) {
        self.docs.remove(key);
    }

    pub fn dirty_keys(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .docs
            .values()
            .filter(|d| d.dirty())
            .map(|d| d.key.clone())
            .collect();
        v.sort();
        v
    }

    /// Diagnostics for open files (live) and indexed files (from disk).
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        let mut push = |key: &str, r: &IndexRow| {
            out.push(Problem {
                key: key.to_string(),
                file: key.rsplit('/').next().unwrap_or(key).to_string(),
                line: r.line,
                col: r.col,
                span: r.span,
                code: r.code,
                severity: r.severity,
                message: r.message.clone(),
            })
        };
        for (key, rs) in &self.index {
            if self.docs.contains_key(key) {
                continue;
            }
            for r in rs {
                push(key, r);
            }
        }
        let mut open: Vec<_> = self.docs.values().collect();
        open.sort_by(|a, b| a.key.cmp(&b.key));
        for d in open {
            for r in rows(&d.text, &d.parsed.diagnostics) {
                push(&d.key, &r);
            }
        }
        out.sort_by(|a, b| (a.key.as_str(), a.line).cmp(&(b.key.as_str(), b.line)));
        out
    }

    pub fn problem_count(&self) -> usize {
        self.problems()
            .iter()
            .filter(|p| p.severity != Severity::Info)
            .count()
    }

    /// React to external file changes. Returns keys of open documents whose state changed.
    pub fn handle_fs(&mut self, paths: &[PathBuf]) -> Vec<String> {
        let mut changed = Vec::new();
        let mut tree_dirty = false;
        for p in paths {
            let is_md = p.extension().is_some_and(|e| e == "md");
            if p.file_name().is_some_and(|n| n == "fluxwire.toml")
                || p.is_dir()
                || !p.exists()
                || is_md
            {
                tree_dirty = true;
            }
            if !is_md || !p.starts_with(&self.root) {
                continue;
            }
            let key = self.key_of(p);
            if let Some(doc) = self.docs.get_mut(&key) {
                match std::fs::read_to_string(p) {
                    Ok(disk) => {
                        if disk == doc.text {
                            if doc.saved != disk || doc.conflict.is_some() || doc.missing {
                                doc.saved = disk;
                                doc.conflict = None;
                                doc.missing = false;
                                changed.push(key.clone());
                            }
                        } else if !doc.dirty() {
                            doc.reload_from(disk);
                            changed.push(key.clone());
                        } else if doc.conflict.as_deref() != Some(disk.as_str())
                            && disk != doc.saved
                        {
                            doc.conflict = Some(disk);
                            changed.push(key.clone());
                        }
                    }
                    Err(_) => {
                        if !doc.missing {
                            doc.missing = true;
                            changed.push(key.clone());
                        }
                    }
                }
            }
        }
        if tree_dirty {
            self.rescan_incremental(paths);
        } else {
            for p in paths {
                if p.starts_with(&self.root) && is_request_file(p) {
                    self.index_file(p);
                }
            }
        }
        self.branch = read_git_branch(&self.root);
        changed
    }

    /// Request nodes in tree order, for quick search.
    pub fn all_requests(&self) -> Vec<(String, String, String, RequestKind)> {
        fn walk(nodes: &[TreeNode], out: &mut Vec<(String, String, String, RequestKind)>) {
            for n in nodes {
                match n {
                    TreeNode::Folder { children, .. } => walk(children, out),
                    TreeNode::Request {
                        name,
                        path,
                        method,
                        kind,
                    } => out.push((path.clone(), name.clone(), method.clone(), *kind)),
                    TreeNode::Doc { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        for c in &self.collections {
            walk(&c.tree, &mut out);
        }
        out
    }

    /// Requests, folders and docs headings as search items (no parsing; reads cached rows).
    pub fn search_items(&self) -> Vec<fluxwire_search::Item> {
        use fluxwire_search::{Item, Kind};
        fn walk(nodes: &[TreeNode], rows: &BTreeMap<String, SearchRow>, out: &mut Vec<Item>) {
            for n in nodes {
                match n {
                    TreeNode::Folder {
                        name,
                        path,
                        children,
                    } => {
                        out.push(
                            Item::new(format!("folder:{path}"), Kind::Folder, name.clone())
                                .detail(path.clone()),
                        );
                        walk(children, rows, out);
                    }
                    TreeNode::Request {
                        name, path, method, ..
                    } => {
                        let row = rows.get(path);
                        let url = row.map(|r| r.url.as_str()).unwrap_or_default();
                        out.push(
                            Item::new(format!("req:{path}"), Kind::Request, name.clone())
                                .detail(path.clone())
                                .extra(format!("{method} {url}"))
                                .tags(row.map(|r| r.tags.clone()).unwrap_or_default()),
                        );
                    }
                    TreeNode::Doc { name, path } => {
                        out.push(
                            Item::new(format!("doc:{path}"), Kind::Doc, name.clone())
                                .detail(path.clone()),
                        );
                    }
                }
            }
        }
        let mut out = Vec::new();
        for c in &self.collections {
            walk(&c.tree, &self.search_rows, &mut out);
        }
        for (key, row) in &self.search_rows {
            for (i, h) in row.headings.iter().enumerate() {
                out.push(
                    Item::new(format!("head:{key}#{i}"), Kind::Doc, h.clone()).detail(key.clone()),
                );
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.all_requests().is_empty()
    }

    /// The collection directory that contains `key`, as an absolute path.
    pub fn collection_dir_for(&self, key: &str) -> Option<PathBuf> {
        let abs = self.abs_of(key);
        self.collections
            .iter()
            .filter(|c| abs.starts_with(&c.root))
            .map(|c| c.root.clone())
            .max_by_key(|p| p.components().count())
    }
}

fn collect_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "environments" | "lib" | "node_modules" | "target"
                )
            {
                continue;
            }
            if p.join("fluxwire.toml").is_file() {
                continue;
            }
            collect_md(&p, out);
        } else if name.ends_with(".flux.md") {
            out.push(p);
        }
    }
}

// ---- persistence -------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub workspaces: Vec<String>,
    pub last: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionTab {
    pub key: String,
    pub preview: bool,
    pub pinned: bool,
    /// `""` for request tabs; otherwise `docs`, `run`, `env`, `history`, `sync` or `keys`.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// Vertical scroll offset of the editor area (pixels, positive = scrolled down).
    #[serde(default)]
    pub scroll: f32,
}

/// Window layout remembered per workspace (SPEC §17.1).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub left_open: bool,
    pub left_width: f32,
    /// `workspace`, `history`, `environments` or `sync`.
    pub panel: String,
    pub bottom_open: bool,
    /// `console` or `problems`.
    pub bottom_tab: String,
    pub bottom_height: f32,
    pub split_vertical: bool,
    pub group_by_kind: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            left_open: true,
            left_width: 260.0,
            panel: "workspace".into(),
            bottom_open: false,
            bottom_tab: "console".into(),
            bottom_height: 200.0,
            split_vertical: true,
            group_by_kind: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    pub active: Option<String>,
    /// Unsaved buffers: key -> text.
    pub unsaved: BTreeMap<String, String>,
    /// Active environment, remembered per workspace.
    #[serde(default)]
    pub active_env: Option<String>,
    #[serde(default)]
    pub layout: Layout,
    /// Recently closed tabs, oldest first, for "reopen closed tab" after a restart.
    #[serde(default)]
    pub closed: Vec<SessionTab>,
    /// Collections removed from the Workspace (keys), so they stay hidden on reopen.
    #[serde(default)]
    pub hidden_collections: Vec<String>,
}

pub fn load_recent(dir: &Path) -> Recent {
    std::fs::read_to_string(dir.join("recent.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_recent(dir: &Path, r: &Recent) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(t) = serde_json::to_string_pretty(r) {
        let _ = std::fs::write(dir.join("recent.json"), t);
    }
}

pub fn remember_workspace(r: &mut Recent, root: &Path) {
    let s = root.to_string_lossy().into_owned();
    r.workspaces.retain(|w| *w != s);
    r.workspaces.insert(0, s.clone());
    r.workspaces.truncate(10);
    r.last = Some(s);
}

fn session_path(dir: &Path, root: &Path) -> PathBuf {
    let mut h: u64 = 14_695_981_039_346_656_037;
    for b in root.to_string_lossy().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211);
    }
    dir.join("sessions").join(format!("{h:016x}.json"))
}

pub fn load_session(dir: &Path, root: &Path) -> Session {
    std::fs::read_to_string(session_path(dir, root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_session(dir: &Path, root: &Path, s: &Session) {
    let p = session_path(dir, root);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(t) = serde_json::to_string(s) {
        let _ = std::fs::write(p, t);
    }
}

/// Most recently run palette/quick-search item ids (recency boost), newest first.
pub fn load_recent_ids(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("palette-recent.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_recent_ids(dir: &Path, ids: &[String]) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(t) = serde_json::to_string(ids) {
        let _ = std::fs::write(dir.join("palette-recent.json"), t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("fluxwire-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/payments-api")
    }

    #[test]
    fn opens_fixture_and_prefixes_keys() {
        let p = Workspace::open(&fixture()).unwrap();
        assert_eq!(p.collections.len(), 1);
        assert!(
            p.all_requests()
                .iter()
                .any(|r| r.0 == "payments/create-payment.flux.md")
        );
        assert_eq!(p.problem_count(), 0, "{:?}", p.problems());
    }

    #[test]
    fn nested_collection_keys_include_prefix() {
        let root = tmp("nested");
        fluxwire_format::create_collection(&root.join("api"), "Api").unwrap();
        fluxwire_format::create_request(&root.join("api"), "Hello", "", false).unwrap();
        let p = Workspace::open(&root).unwrap();
        assert_eq!(p.all_requests()[0].0, "api/hello.flux.md");
        assert_eq!(
            p.collection_dir_for("api/hello.flux.md").unwrap(),
            root.join("api")
        );
    }

    #[test]
    fn format_too_new_is_an_error_row() {
        let root = tmp("toonew");
        std::fs::write(
            root.join("fluxwire.toml"),
            "[collection]\nname=\"x\"\nformat = 2\n",
        )
        .unwrap();
        let ws = Workspace::open(&root).expect("workspace opens");
        assert!(ws.collections.is_empty());
        assert!(matches!(
            ws.errors[0].1,
            OpenError::FormatTooNew { found: 2, .. }
        ));
    }

    #[test]
    fn edit_save_roundtrip_changes_only_edited_line() {
        let root = tmp("edit");
        copy_dir(&fixture(), &root);
        let mut p = Workspace::open(&root).unwrap();
        let key = "payments/get-payment.flux.md";
        let before = std::fs::read_to_string(root.join(key)).unwrap();
        let d = p.open_doc(key).unwrap();
        let edits = fluxwire_format::set_header(&d.parsed, 0, true, "Accept", "text/plain");
        assert!(d.apply(&edits));
        assert!(d.dirty());
        d.save().unwrap();
        let after = std::fs::read_to_string(root.join(key)).unwrap();
        let diff: Vec<_> = before
            .lines()
            .zip(after.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(diff.len(), 1);
        assert_eq!(before.lines().count(), after.lines().count());
    }

    #[test]
    fn unsupported_file_roundtrips_and_is_not_runnable() {
        let root = tmp("edge");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/format-edge/fluxwire.toml"),
            root.join("fluxwire.toml"),
        )
        .unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/format-edge/future-grpc.flux.md"),
            root.join("future-grpc.flux.md"),
        )
        .unwrap();
        let mut p = Workspace::open(&root).unwrap();
        let d = p.open_doc("future-grpc.flux.md").unwrap();
        assert!(!d.parsed.runnable());
        assert!(!d.parsed.other_content().is_empty());
        let before = d.text.clone();
        d.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("future-grpc.flux.md")).unwrap(),
            before
        );
    }

    #[test]
    fn external_change_reloads_clean_and_conflicts_dirty() {
        let root = tmp("watch");
        copy_dir(&fixture(), &root);
        let mut p = Workspace::open(&root).unwrap();
        let key = "payments/list-payments.flux.md";
        p.open_doc(key).unwrap();
        let path = root.join(key);
        let orig = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, orig.replace("limit=20", "limit=50")).unwrap();
        assert_eq!(
            p.handle_fs(std::slice::from_ref(&path)),
            vec![key.to_string()]
        );
        assert!(p.doc(key).unwrap().text.contains("limit=50"));
        assert!(!p.doc(key).unwrap().dirty());

        let d = p.doc_mut(key).unwrap();
        let e = fluxwire_format::set_request_line(
            &d.parsed,
            "POST",
            &d.parsed.request().unwrap().target.clone(),
        );
        d.apply(&e);
        std::fs::write(&path, orig.replace("limit=20", "limit=99")).unwrap();
        assert_eq!(
            p.handle_fs(std::slice::from_ref(&path)),
            vec![key.to_string()]
        );
        let d = p.doc(key).unwrap();
        assert!(d.conflict.is_some());
        assert!(d.text.contains("POST"), "dirty text is kept");
    }

    #[test]
    fn malformed_file_gets_problems() {
        let root = tmp("bad");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/format-edge/fluxwire.toml"),
            root.join("fluxwire.toml"),
        )
        .unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/format-edge/malformed.flux.md"),
            root.join("malformed.flux.md"),
        )
        .unwrap();
        let p = Workspace::open(&root).unwrap();
        let pr = p.problems();
        assert!(pr.iter().any(|x| x.code == "F101"), "{pr:?}");
        assert!(pr.iter().all(|x| x.line >= 1));
    }

    #[test]
    fn session_and_recent_roundtrip() {
        let dir = tmp("state");
        let mut r = load_recent(&dir);
        remember_workspace(&mut r, Path::new("/a"));
        remember_workspace(&mut r, Path::new("/b"));
        remember_workspace(&mut r, Path::new("/a"));
        save_recent(&dir, &r);
        assert_eq!(load_recent(&dir).workspaces, ["/a", "/b"]);
        let mut s = Session::default();
        s.unsaved.insert("x.flux.md".into(), "hi".into());
        save_session(&dir, Path::new("/a"), &s);
        assert_eq!(load_session(&dir, Path::new("/a")), s);
        assert_eq!(load_session(&dir, Path::new("/zzz")), Session::default());
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let t = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &t);
            } else {
                std::fs::copy(e.path(), t).unwrap();
            }
        }
    }
}
