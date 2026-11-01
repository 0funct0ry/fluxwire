//! Folder and collection runs (SPEC §12).
//!
//! A run selects the requests under a folder (tree order: `seq`, then name), then runs them
//! once per iteration. Requests run concurrently up to `concurrency`, each with an isolated
//! runtime-variable scope seeded from the run's starting scope. A folder whose
//! `settings.run_mode` is `sequential` (or the whole run, with [`RunMode::Sequential`]) forms a
//! *unit*: its requests run in order and share runtime variables, while units still run
//! concurrently with each other.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use fluxwire_model::{FluxDoc, FmValue, RequestKind, TreeNode};
use fluxwire_protocol::{CancellationToken, EnvState, ExecCtx, ExecSettings, NetConfig};
use fluxwire_vars::{Layer, Source, Var};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::{Job, RunResult, Runner, ScriptOptions, exec, pick_protocol};

/// How requests are scheduled. Folders can still force `sequential` themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunMode {
    #[default]
    Concurrent,
    Sequential,
}

pub const DEFAULT_CONCURRENCY: u32 = 8;

/// Everything a folder run needs.
#[derive(Clone, Default)]
pub struct RunPlan {
    pub collection_root: PathBuf,
    /// Workspace-relative folder key under the collection root; empty for the collection.
    pub folder: String,
    pub mode: RunMode,
    /// Concurrent mode only; `None` means [`DEFAULT_CONCURRENCY`].
    pub concurrency: Option<u32>,
    /// Repeat count when there is no data file (at least 1).
    pub iterations: u32,
    /// One iteration per row; row values are variables just below `--var` (SPEC §12).
    pub data: Vec<BTreeMap<String, String>>,
    /// Comma-separated: `tag:smoke`, `name:*refund*`, or a name glob (`*`, `?`) matched on name and path.
    pub filter: Option<String>,
    /// Run only these request keys (used by "Re-run failed").
    pub only: Option<Vec<String>>,
    /// Stop starting new requests after the first failure.
    pub bail: bool,
    /// Pause between requests of a sequential unit.
    pub delay_ms: u64,
    /// Whole-run timeout; unfinished requests are cancelled.
    pub timeout_ms: Option<u64>,
    pub env: Option<String>,
    pub env_state: Option<EnvState>,
    pub settings: ExecSettings,
    pub net: NetConfig,
    pub scripts: ScriptOptions,
    /// Runtime variables the run starts with: name → (value, secret).
    pub runtime: BTreeMap<String, (String, bool)>,
}

/// A request selected for the run, announced before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRequest {
    pub key: String,
    pub name: String,
    pub method: String,
    /// Folder key, empty for requests at the collection root.
    pub folder: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Passed,
    Failed,
    Skipped,
}

/// One request in one iteration.
#[derive(Debug, Clone)]
pub struct RequestRun {
    pub iteration: u32,
    pub request: PlannedRequest,
    /// Milliseconds from the start of the run to the start of this request.
    pub started_ms: u64,
    pub duration_ms: u64,
    pub result: RunResult,
}

impl RequestRun {
    pub fn status(&self) -> RequestStatus {
        if self.result.skipped.is_some()
            || self.result.error.as_ref().is_some_and(|e| e.is_cancelled())
        {
            RequestStatus::Skipped
        } else if self.result.passed() {
            RequestStatus::Passed
        } else {
            RequestStatus::Failed
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub requests: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub tests: usize,
    pub tests_failed: usize,
}

#[derive(Debug, Clone)]
pub struct FolderRunResult {
    pub run_id: String,
    pub folder: String,
    pub started_at: SystemTime,
    pub total_ms: u64,
    pub iterations: u32,
    /// Row values per iteration (empty maps when there is no data file).
    pub rows: Vec<BTreeMap<String, String>>,
    /// Iteration-major, then selection order.
    pub requests: Vec<Arc<RequestRun>>,
    pub cancelled: bool,
    pub timed_out: bool,
    pub bailed: bool,
    /// The run could not start (folder not found, bad filter…).
    pub error: Option<String>,
}

impl FolderRunResult {
    pub fn summary(&self) -> RunSummary {
        let mut s = RunSummary::default();
        for r in &self.requests {
            s.requests += 1;
            match r.status() {
                RequestStatus::Passed => s.passed += 1,
                RequestStatus::Failed => s.failed += 1,
                RequestStatus::Skipped => s.skipped += 1,
            }
            s.tests += r.result.tests.len();
            s.tests_failed += r.result.tests.iter().filter(|t| !t.passed).count();
        }
        s
    }

    /// Keys of requests that failed in any iteration, in selection order.
    pub fn failed_keys(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.requests {
            if r.status() == RequestStatus::Failed && !out.contains(&r.request.key) {
                out.push(r.request.key.clone());
            }
        }
        out
    }

    pub fn passed(&self) -> bool {
        self.error.is_none() && self.summary().failed == 0
    }
}

#[derive(Debug, Clone)]
pub enum FolderRunEvent {
    Started {
        iterations: u32,
        requests: Vec<PlannedRequest>,
    },
    RequestStarted {
        iteration: u32,
        key: String,
    },
    RequestFinished(Arc<RequestRun>),
    Finished(Box<FolderRunResult>),
}

pub struct FolderRunHandle {
    pub run_id: String,
    pub events: UnboundedReceiver<FolderRunEvent>,
    cancel: CancellationToken,
}

impl FolderRunHandle {
    /// Cancel the whole run: in-flight requests are cancelled, unstarted ones skipped.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn canceller(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

fn new_run_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

impl Runner {
    /// Start a folder or collection run (SPEC §12).
    pub fn run_folder(&self, plan: RunPlan) -> FolderRunHandle {
        let (tx, rx) = unbounded_channel();
        let cancel = CancellationToken::new();
        let run_id = new_run_id();
        let shared = self.protocols.clone();
        let handle = FolderRunHandle {
            run_id: run_id.clone(),
            events: rx,
            cancel: cancel.clone(),
        };
        self.rt.spawn(async move {
            let result = drive(shared, Arc::new(plan), run_id, cancel, &tx).await;
            let _ = tx.send(FolderRunEvent::Finished(Box::new(result)));
        });
        handle
    }
}

/// A selected request with everything resolved once per run.
struct Item {
    planned: PlannedRequest,
    doc: FluxDoc,
    file: PathBuf,
    ancestors: Arc<Vec<(String, FluxDoc)>>,
    /// The outermost folder forcing `run_mode: sequential`, if any.
    group: Option<String>,
}

fn setting<'a>(doc: &'a FluxDoc, key: &str) -> Option<&'a str> {
    match doc.front.get("settings").map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()),
        _ => None,
    }
}

fn find_folder<'a>(tree: &'a [TreeNode], key: &str) -> Option<&'a [TreeNode]> {
    if key.is_empty() {
        return Some(tree);
    }
    for n in tree {
        if let TreeNode::Folder { path, children, .. } = n {
            if path == key {
                return Some(children);
            }
            if key.starts_with(&format!("{path}/"))
                && let Some(c) = find_folder(children, key)
            {
                return Some(c);
            }
        }
    }
    None
}

fn flatten<'a>(nodes: &'a [TreeNode], out: &mut Vec<(&'a str, &'a str, &'a str)>) {
    for n in nodes {
        match n {
            TreeNode::Folder { children, .. } => flatten(children, out),
            TreeNode::Request {
                name,
                path,
                method,
                kind: RequestKind::Http,
            } => out.push((name, path, method)),
            _ => {}
        }
    }
}

/// `*` and `?` glob, case-insensitive.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    go(&p, &t)
}

fn filter_matches(filter: &str, item: &Item) -> bool {
    let mut tags: Vec<String> = item.doc.front.list("tags");
    for (_, d) in item.ancestors.iter() {
        tags.extend(d.front.list("tags"));
    }
    filter
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .any(|f| {
            if let Some(tag) = f.strip_prefix("tag:") {
                tags.iter().any(|t| t.eq_ignore_ascii_case(tag.trim()))
            } else if let Some(g) = f.strip_prefix("name:") {
                glob_match(g.trim(), &item.planned.name)
            } else {
                glob_match(f, &item.planned.name)
                    || glob_match(f, &item.planned.key)
                    || glob_match(&format!("*{f}*"), &item.planned.name)
            }
        })
}

fn select(plan: &RunPlan) -> Result<Vec<Item>, String> {
    let col = fluxwire_format::scan(&plan.collection_root).map_err(|e| format!("{e:?}"))?;
    let nodes = find_folder(&col.tree, &plan.folder)
        .ok_or_else(|| format!("folder `{}` not found", plan.folder))?;
    let mut flat = Vec::new();
    flatten(nodes, &mut flat);
    let reg = fluxwire_protocol::ProtocolRegistry::builtin();
    let mut cache: BTreeMap<String, Arc<Vec<(String, FluxDoc)>>> = BTreeMap::new();
    let mut items = Vec::new();
    for (name, key, method) in flat {
        if let Some(only) = &plan.only
            && !only.iter().any(|k| k == key)
        {
            continue;
        }
        let file = plan.collection_root.join(key);
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let doc = fluxwire_format::parse_with(&text, fluxwire_model::FileKind::Request, &reg);
        let folder = key
            .rsplit_once('/')
            .map(|(f, _)| f)
            .unwrap_or("")
            .to_string();
        let ancestors = cache
            .entry(folder.clone())
            .or_insert_with(|| {
                Arc::new(fluxwire_format::ancestor_docs(&plan.collection_root, &file))
            })
            .clone();
        let group = ancestors
            .iter()
            .find(|(_, d)| setting(d, "run_mode") == Some("sequential"))
            .map(|(label, _)| {
                label
                    .strip_suffix("folder.flux.md")
                    .or_else(|| label.strip_suffix("collection.flux.md"))
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .to_string()
            });
        let item = Item {
            planned: PlannedRequest {
                key: key.to_string(),
                name: name.to_string(),
                method: method.to_string(),
                folder,
            },
            doc,
            file,
            ancestors,
            group,
        };
        if let Some(f) = plan.filter.as_deref()
            && !f.trim().is_empty()
            && !filter_matches(f, &item)
        {
            continue;
        }
        items.push(item);
    }
    Ok(items)
}

/// Indexes into `items`, grouped into units that run internally in order.
fn units(items: &[Item], mode: RunMode) -> Vec<Vec<usize>> {
    if mode == RunMode::Sequential {
        return vec![(0..items.len()).collect()];
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut group_at: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        match it.group.as_deref() {
            Some(g) => match group_at.get(g) {
                Some(&u) => out[u].push(i),
                None => {
                    group_at.insert(g, out.len());
                    out.push(vec![i]);
                }
            },
            None => out.push(vec![i]),
        }
    }
    out
}

struct Shared {
    plan: Arc<RunPlan>,
    protocols: Arc<Vec<Arc<dyn fluxwire_protocol::Protocol>>>,
    cancel: CancellationToken,
    stop: AtomicBool,
    timed_out: AtomicBool,
    start: Instant,
    tx: UnboundedSender<FolderRunEvent>,
}

fn skipped_run(sh: &Shared, iteration: u32, it: &Item, reason: &str) -> Arc<RequestRun> {
    Arc::new(RequestRun {
        iteration,
        request: it.planned.clone(),
        started_ms: sh.start.elapsed().as_millis() as u64,
        duration_ms: 0,
        result: RunResult::skipped(it.planned.key.clone(), reason),
    })
}

async fn run_one(
    sh: &Shared,
    it: &Item,
    row: &BTreeMap<String, String>,
    runtime: &BTreeMap<String, (String, bool)>,
) -> RunResult {
    let plan = &sh.plan;
    let mut scope = fluxwire_format::build_scope(&fluxwire_format::ScopeInputs {
        collection_root: &plan.collection_root,
        file: &it.file,
        doc: &it.doc,
        env: plan.env.as_deref(),
        env_root: plan.scripts.env_root.as_deref(),
        env_overlay: plan.scripts.env_overlay.as_ref(),
        runtime,
        globals: &plan.scripts.globals,
        allow_process_env: plan.scripts.allow_process_env,
    });
    for (k, v) in row {
        scope.set(
            Layer::Cli,
            k.clone(),
            Var {
                value: v.clone(),
                secret: false,
                source: Source::new("data file", None),
            },
        );
    }
    let mut scripts = plan.scripts.clone();
    scripts.collection_root = Some(plan.collection_root.clone());
    scripts.file = Some(it.file.clone());
    let job = Job {
        shared: sh.protocols.clone(),
        proto: pick_protocol(&sh.protocols, &it.doc),
        doc: it.doc.clone(),
        scope,
        ctx: ExecCtx {
            cancel: sh.cancel.clone(),
            settings: plan.settings.clone(),
            net: plan.net.clone(),
            temp_dir: None,
            env: plan.env_state.clone(),
            session: None,
        },
        scripts,
        depth: 0,
        started_at: SystemTime::now(),
        ancestors: Some(it.ancestors.clone()),
    };
    exec(job, None).await
}

async fn run_unit(
    sh: Arc<Shared>,
    items: Arc<Vec<Item>>,
    unit: Vec<usize>,
    iteration: u32,
    row: Arc<BTreeMap<String, String>>,
    sem: Arc<Semaphore>,
) -> Vec<(usize, Arc<RequestRun>)> {
    let mut out = Vec::new();
    let mut runtime = sh.plan.runtime.clone();
    let multi = unit.len() > 1;
    for (n, &i) in unit.iter().enumerate() {
        let it = &items[i];
        let skip = |sh: &Shared| {
            if sh.cancel.is_cancelled() {
                Some("run cancelled")
            } else if sh.stop.load(Ordering::SeqCst) {
                Some("skipped after a failure (bail)")
            } else {
                None
            }
        };
        let finish = |run: Arc<RequestRun>, out: &mut Vec<(usize, Arc<RequestRun>)>| {
            let _ = sh.tx.send(FolderRunEvent::RequestFinished(run.clone()));
            out.push((i, run));
        };
        if let Some(reason) = skip(&sh) {
            finish(skipped_run(&sh, iteration, it, reason), &mut out);
            continue;
        }
        if multi && n > 0 && sh.plan.delay_ms > 0 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(sh.plan.delay_ms)) => {}
                _ = sh.cancel.cancelled() => {}
            }
        }
        let Ok(_permit) = sem.acquire().await else {
            break;
        };
        if let Some(reason) = skip(&sh) {
            finish(skipped_run(&sh, iteration, it, reason), &mut out);
            continue;
        }
        let _ = sh.tx.send(FolderRunEvent::RequestStarted {
            iteration,
            key: it.planned.key.clone(),
        });
        let started_ms = sh.start.elapsed().as_millis() as u64;
        let t = Instant::now();
        let result = run_one(&sh, it, &row, &runtime).await;
        let duration_ms = t.elapsed().as_millis() as u64;
        for c in &result.var_changes {
            match &c.value {
                Some(v) => {
                    runtime.insert(c.name.clone(), (v.clone(), c.secret));
                }
                None => {
                    runtime.remove(&c.name);
                }
            }
        }
        let run = Arc::new(RequestRun {
            iteration,
            request: it.planned.clone(),
            started_ms,
            duration_ms,
            result,
        });
        if sh.plan.bail && run.status() == RequestStatus::Failed {
            sh.stop.store(true, Ordering::SeqCst);
        }
        finish(run, &mut out);
    }
    out
}

async fn drive(
    protocols: Arc<Vec<Arc<dyn fluxwire_protocol::Protocol>>>,
    plan: Arc<RunPlan>,
    run_id: String,
    cancel: CancellationToken,
    tx: &UnboundedSender<FolderRunEvent>,
) -> FolderRunResult {
    let started_at = SystemTime::now();
    let start = Instant::now();
    let mut result = FolderRunResult {
        run_id,
        folder: plan.folder.clone(),
        started_at,
        total_ms: 0,
        iterations: 0,
        rows: Vec::new(),
        requests: Vec::new(),
        cancelled: false,
        timed_out: false,
        bailed: false,
        error: None,
    };
    let items = match select(&plan) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };
    let rows: Vec<BTreeMap<String, String>> = if plan.data.is_empty() {
        vec![BTreeMap::new(); plan.iterations.max(1) as usize]
    } else {
        plan.data.clone()
    };
    result.iterations = rows.len() as u32;
    result.rows = rows.clone();
    let _ = tx.send(FolderRunEvent::Started {
        iterations: result.iterations,
        requests: items.iter().map(|i| i.planned.clone()).collect(),
    });

    let sh = Arc::new(Shared {
        plan: plan.clone(),
        protocols,
        cancel: cancel.clone(),
        stop: AtomicBool::new(false),
        timed_out: AtomicBool::new(false),
        start,
        tx: tx.clone(),
    });
    let timer = plan.timeout_ms.filter(|ms| *ms > 0).map(|ms| {
        let sh = sh.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            sh.timed_out.store(true, Ordering::SeqCst);
            sh.cancel.cancel();
        })
    });

    let unit_list = units(&items, plan.mode);
    let permits = plan.concurrency.unwrap_or(DEFAULT_CONCURRENCY).max(1) as usize;
    let sem = Arc::new(Semaphore::new(permits));
    for (iteration, row) in rows.into_iter().enumerate() {
        let row = Arc::new(row);
        let slots: Arc<Mutex<Vec<Option<Arc<RequestRun>>>>> =
            Arc::new(Mutex::new(vec![None; items.len()]));
        let mut set = tokio::task::JoinSet::new();
        for unit in &unit_list {
            set.spawn(run_unit(
                sh.clone(),
                items.clone(),
                unit.clone(),
                iteration as u32,
                row.clone(),
                sem.clone(),
            ));
        }
        while let Some(done) = set.join_next().await {
            for (i, run) in done.unwrap_or_default() {
                slots.lock().unwrap()[i] = Some(run);
            }
        }
        let slots = std::mem::take(&mut *slots.lock().unwrap());
        result.requests.extend(slots.into_iter().flatten());
        if sh.stop.load(Ordering::SeqCst) || cancel.is_cancelled() {
            break;
        }
    }
    if let Some(t) = timer {
        t.abort();
    }
    result.total_ms = start.elapsed().as_millis() as u64;
    result.timed_out = sh.timed_out.load(Ordering::SeqCst);
    result.cancelled = cancel.is_cancelled() && !result.timed_out;
    result.bailed = sh.stop.load(Ordering::SeqCst);
    result
}

/// Rows from a `.csv` (header row names the variables) or `.json` (array of objects) file.
pub fn load_data_file(path: &Path) -> Result<Vec<BTreeMap<String, String>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "json" => parse_json_rows(&text),
        "csv" => Ok(parse_csv_rows(&text)),
        _ => Err("data files must be .csv or .json".into()),
    }
}

pub fn parse_json_rows(text: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let arr = v
        .as_array()
        .ok_or("a JSON data file must be an array of objects")?;
    arr.iter()
        .map(|row| {
            let o = row
                .as_object()
                .ok_or("every JSON data row must be an object")?;
            Ok(o.iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect())
        })
        .collect()
}

fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut row));
                any = false;
            }
            c => field.push(c),
        }
    }
    if any {
        row.push(field);
        records.push(row);
    }
    records
        .into_iter()
        .filter(|r| !(r.len() == 1 && r[0].is_empty()))
        .collect()
}

pub fn parse_csv_rows(text: &str) -> Vec<BTreeMap<String, String>> {
    let mut recs = csv_records(text).into_iter();
    let Some(header) = recs.next() else {
        return Vec::new();
    };
    recs.map(|r| {
        header
            .iter()
            .enumerate()
            .map(|(i, h)| (h.trim().to_string(), r.get(i).cloned().unwrap_or_default()))
            .collect()
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("get-*", "Get-Payment"));
        assert!(glob_match("?et", "get"));
        assert!(!glob_match("get", "get-payment"));
    }

    #[test]
    fn csv_quotes_and_blank_lines() {
        let rows = parse_csv_rows("a,b\n1,\"x,\"\"y\"\"\"\n\n2,z\r\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["b"], "x,\"y\"");
        assert_eq!(rows[1]["a"], "2");
    }

    #[test]
    fn json_rows_stringify_values() {
        let rows = parse_json_rows(r#"[{"id":1,"n":"a"}]"#).unwrap();
        assert_eq!(rows[0]["id"], "1");
        assert_eq!(rows[0]["n"], "a");
        assert!(parse_json_rows("{}").is_err());
    }
}
