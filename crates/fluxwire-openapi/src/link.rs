//! A Spec link in use (SPEC §15.3): load both sides, compute status, resolve conflicts, then
//! pull or push. Nothing is applied automatically; the base snapshot advances only by the
//! changes that were actually applied.

use crate::folder::{FolderModel, from_folder};
use crate::merge::{Change, ChangeKind, Conflict, Direction, MergeResult, merge};
use crate::pull::pull_op;
use crate::push::{Deletion, NeedsManual, Pushed, SpecEdit, apply, plan_push};
use crate::spec::{from_spec, parse_spec};
use crate::sync_model::{Op, SyncModel};
use fluxwire_model::OpenApiLink;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncStatus {
    pub incoming: usize,
    pub outgoing: usize,
    pub conflicts: usize,
    pub needs_manual: usize,
}

/// How a conflict is settled (SPEC §15.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    TakeSpec,
    TakeCollection,
    Edit(Value),
}

#[derive(Debug, PartialEq)]
pub enum SyncError {
    Io(String),
    Parse(String),
    /// Unresolved conflicts block pull and push.
    Conflicts(usize),
    /// URL sources are pull-only and must be fetched by the caller.
    Remote,
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) | Self::Parse(e) => f.write_str(e),
            Self::Conflicts(n) => write!(f, "{n} unresolved conflict(s)"),
            Self::Remote => f.write_str("URL sources must be fetched before syncing"),
        }
    }
}

pub struct LinkState {
    pub root: PathBuf,
    pub link: OpenApiLink,
    pub spec_path: PathBuf,
    pub spec_text: String,
    pub spec_root: Value,
    pub spec: SyncModel,
    pub coll: FolderModel,
    pub base: SyncModel,
    pub result: MergeResult,
}

pub fn base_path(root: &Path, link: &OpenApiLink) -> PathBuf {
    root.join(".fluxwire/sync")
        .join(format!("{}.base.json", link.name))
}

pub fn is_remote(link: &OpenApiLink) -> bool {
    link.source.starts_with("http://") || link.source.starts_with("https://")
}

/// Load a link from disk. `remote_text` supplies the fetched document for URL sources.
pub fn load(
    root: &Path,
    link: &OpenApiLink,
    remote_text: Option<String>,
) -> Result<LinkState, SyncError> {
    let spec_path = root.join(&link.source);
    let spec_text = match (is_remote(link), remote_text) {
        (true, Some(t)) => t,
        (true, None) => return Err(SyncError::Remote),
        (false, _) => std::fs::read_to_string(&spec_path)
            .map_err(|e| SyncError::Io(format!("{}: {e}", link.source)))?,
    };
    let spec_root = parse_spec(&spec_text).map_err(SyncError::Parse)?;
    let spec = from_spec(&spec_root).map_err(SyncError::Parse)?;
    let base = match std::fs::read_to_string(base_path(root, link)) {
        Ok(t) => SyncModel::from_base_json(&t).map_err(SyncError::Parse)?,
        // No snapshot yet: treat the spec as the base so the first status shows collection drift.
        Err(_) => spec.clone(),
    };
    let folder = root.join(link.folder.as_deref().unwrap_or(""));
    // Fields a request cannot express are copied from the base, so only real edits differ;
    // operations new to the base fall back to the spec.
    let reference = SyncModel {
        operations: spec
            .operations
            .iter()
            .chain(base.operations.iter())
            .map(|(k, v)| (k.clone(), base.operations.get(k).unwrap_or(v).clone()))
            .collect(),
    };
    let coll = from_folder(&folder, &reference);
    let result = merge(&base, &spec, &coll.model);
    Ok(LinkState {
        root: root.to_path_buf(),
        link: link.clone(),
        spec_path,
        spec_text,
        spec_root,
        spec,
        coll,
        base,
        result,
    })
}

impl LinkState {
    pub fn is_json(&self) -> bool {
        self.spec_text.trim_start().starts_with('{')
    }

    pub fn status(&self) -> SyncStatus {
        let plan = plan_push(
            &self.spec_root,
            &self.spec,
            &self.result.changes,
            Deletion::Deprecate,
        );
        SyncStatus {
            incoming: self.result.incoming().count(),
            outgoing: self.result.outgoing().count(),
            conflicts: self.result.conflicts.len(),
            needs_manual: plan.manual.len(),
        }
    }

    pub fn conflicts(&self) -> &[Conflict] {
        &self.result.conflicts
    }

    /// Changes after settling conflicts with `res` (keyed by operation and field).
    /// Returns the changes and the conflicts still open.
    pub fn resolved(
        &self,
        res: &BTreeMap<(String, Option<String>), Resolution>,
    ) -> (Vec<Change>, Vec<Conflict>) {
        let mut changes = self.result.changes.clone();
        let mut open = Vec::new();
        for c in &self.result.conflicts {
            let Some(r) = res.get(&(c.operation.clone(), c.field.clone())) else {
                open.push(c.clone());
                continue;
            };
            let mut add = |d: Direction, kind: ChangeKind| {
                changes.push(Change {
                    operation: c.operation.clone(),
                    direction: d,
                    kind,
                })
            };
            match (&c.field, r) {
                (Some(f), Resolution::TakeSpec) => add(
                    Direction::Incoming,
                    ChangeKind::Field {
                        field: f.clone(),
                        value: c.spec.clone(),
                    },
                ),
                (Some(f), Resolution::TakeCollection) => add(
                    Direction::Outgoing,
                    ChangeKind::Field {
                        field: f.clone(),
                        value: c.collection.clone(),
                    },
                ),
                (Some(f), Resolution::Edit(v)) => {
                    add(
                        Direction::Incoming,
                        ChangeKind::Field {
                            field: f.clone(),
                            value: v.clone(),
                        },
                    );
                    add(
                        Direction::Outgoing,
                        ChangeKind::Field {
                            field: f.clone(),
                            value: v.clone(),
                        },
                    );
                }
                (None, Resolution::TakeSpec) => add(Direction::Incoming, whole(&c.spec)),
                (None, Resolution::TakeCollection) => {
                    add(Direction::Outgoing, whole(&c.collection))
                }
                (None, Resolution::Edit(_)) => open.push(c.clone()),
            }
        }
        (changes, open)
    }
}

fn whole(v: &Value) -> ChangeKind {
    match v {
        Value::Object(o) => {
            ChangeKind::Add(o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        _ => ChangeKind::Delete,
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct PullReport {
    pub files_changed: Vec<PathBuf>,
    pub files_created: Vec<PathBuf>,
    pub files_trashed: Vec<PathBuf>,
}

fn apply_to_base(base: &SyncModel, changes: &[Change], dir: Direction) -> SyncModel {
    let mut out = base.clone();
    for c in changes.iter().filter(|c| c.direction == dir) {
        match &c.kind {
            ChangeKind::Add(op) => {
                out.operations.insert(c.operation.clone(), op.clone());
            }
            ChangeKind::Delete => {
                out.operations.remove(&c.operation);
            }
            ChangeKind::Field { field, value } => {
                out.operations
                    .entry(c.operation.clone())
                    .or_default()
                    .insert(field.clone(), value.clone());
            }
        }
    }
    out
}

fn write_base(state: &LinkState, model: &SyncModel) -> Result<(), SyncError> {
    let p = base_path(&state.root, &state.link);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| SyncError::Io(e.to_string()))?;
    }
    std::fs::write(p, model.to_base_json()).map_err(|e| SyncError::Io(e.to_string()))
}

fn new_request_text(base_url_var: &str, op: &Op) -> String {
    let s = |k: &str| op.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (method, path) = (s("method"), s("path"));
    let name = if s("name").is_empty() {
        format!("{method} {path}")
    } else {
        s("name")
    };
    let mut fm = format!("---\nname: {}\n", fluxwire_format::encode_scalar(&name));
    let tags: Vec<String> = op
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(String::from))
        .collect();
    if !tags.is_empty() {
        fm.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    }
    let key = op
        .get("operation_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    let _ = key;
    let mut text = String::new();
    text.push_str(&fm);
    text.push_str(&format!("---\n\n# {name}\n\n"));
    if !s("description").is_empty() {
        text.push_str(&format!("{}\n\n", s("description")));
    }
    let target: String = path
        .split('/')
        .map(
            |seg| match seg.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                Some(n) => format!(":{n}"),
                None => seg.to_string(),
            },
        )
        .collect::<Vec<_>>()
        .join("/");
    text.push_str(&format!(
        "```http\n{method} {{{{{base_url_var}}}}}{target}\n```\n"
    ));
    if let Some(c) = op.get("expected_status").and_then(Value::as_u64) {
        text.push_str(&format!("\n```http expect\nHTTP {c}\n```\n"));
    }
    text
}

/// Apply incoming changes to the collection and advance the base by them.
pub fn pull(state: &LinkState, changes: &[Change]) -> Result<PullReport, SyncError> {
    let mut report = PullReport::default();
    let mut by_op: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut incoming_ops: BTreeMap<&str, Op> = BTreeMap::new();
    for c in changes
        .iter()
        .filter(|c| c.direction == Direction::Incoming)
    {
        match &c.kind {
            ChangeKind::Field { field, value } => {
                by_op.entry(&c.operation).or_default().push(field);
                incoming_ops
                    .entry(&c.operation)
                    .or_default()
                    .insert(field.clone(), value.clone());
            }
            ChangeKind::Add(op) => {
                let dir = state.root.join(state.link.folder.as_deref().unwrap_or(""));
                let var = state.link.base_url_variable.as_deref().unwrap_or("baseUrl");
                let mut text = new_request_text(var, op);
                // The operation key goes into the front matter so the new file is linked.
                text = text.replacen(
                    "---\n",
                    &format!(
                        "---\noperation: {}\n",
                        fluxwire_format::encode_scalar(&c.operation)
                    ),
                    1,
                );
                let file = dir.join(format!("{}.flux.md", fluxwire_format::slug(&c.operation)));
                std::fs::create_dir_all(&dir).map_err(|e| SyncError::Io(e.to_string()))?;
                let text = fluxwire_format::format(&text, fluxwire_model::FileKind::Request);
                std::fs::write(&file, text).map_err(|e| SyncError::Io(e.to_string()))?;
                report.files_created.push(file);
            }
            ChangeKind::Delete => {
                if let Some(f) = state.coll.files.get(&c.operation) {
                    fluxwire_format::delete(f).map_err(|e| SyncError::Io(e.to_string()))?;
                    report.files_trashed.push(f.clone());
                }
            }
        }
    }
    for (op, fields) in by_op {
        let Some(file) = state.coll.files.get(op) else {
            continue;
        };
        let src = std::fs::read_to_string(file).map_err(|e| SyncError::Io(e.to_string()))?;
        let new = pull_op(&src, &incoming_ops[op], &fields);
        if new != src {
            std::fs::write(file, new).map_err(|e| SyncError::Io(e.to_string()))?;
            report.files_changed.push(file.clone());
        }
    }
    write_base(
        state,
        &apply_to_base(&state.base, changes, Direction::Incoming),
    )?;
    Ok(report)
}

#[derive(Debug, PartialEq)]
pub struct PushReport {
    pub written: bool,
    pub patch: Vec<(SpecEdit, String)>,
    pub manual: Vec<NeedsManual>,
}

/// Compute the pushed spec text without writing (for `diff` and "show patch").
pub fn push_preview(
    state: &LinkState,
    changes: &[Change],
    deletion: Deletion,
) -> Result<(Pushed, Vec<NeedsManual>), SyncError> {
    if is_remote(&state.link) {
        return Err(SyncError::Remote);
    }
    let plan = plan_push(&state.spec_root, &state.spec, changes, deletion);
    let pushed = apply(&state.spec_text, state.is_json(), &plan.edits).map_err(SyncError::Parse)?;
    Ok((pushed, plan.manual))
}

/// Apply outgoing changes to the spec file and advance the base by what was applied.
pub fn push(
    state: &LinkState,
    changes: &[Change],
    deletion: Deletion,
) -> Result<PushReport, SyncError> {
    let (pushed, manual) = push_preview(state, changes, deletion)?;
    let written = pushed.text != state.spec_text;
    if written {
        std::fs::write(&state.spec_path, &pushed.text).map_err(|e| SyncError::Io(e.to_string()))?;
    }
    // Only changes that did not need manual work or fall back to a patch advance the base.
    let skip: Vec<&str> = manual
        .iter()
        .map(|m| m.operation.as_str())
        .chain(pushed.patch.iter().map(|_| ""))
        .collect();
    let applied: Vec<Change> = changes
        .iter()
        .filter(|c| !skip.contains(&c.operation.as_str()))
        .cloned()
        .collect();
    let applied = if pushed.patch.is_empty() {
        applied
    } else {
        Vec::new()
    };
    write_base(
        state,
        &apply_to_base(&state.base, &applied, Direction::Outgoing),
    )?;
    Ok(PushReport {
        written,
        patch: pushed.patch,
        manual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "openapi: 3.0.3\npaths:\n  /pets/{id}:\n    get:\n      operationId: getPet\n      summary: Get a pet\n      parameters:\n        - name: id\n          in: path\n          required: true\n          description: Pet id\n          example: p1\n      responses:\n        '200': {description: ok}\n";
    const REQ: &str = "---\nname: Get a pet\noperation: getPet\npath:\n  id: p1\n---\n\n# Get a pet\n\n```http\nGET {{baseUrl}}/pets/:id\n```\n\n```http expect\nHTTP 200\n```\n";

    fn setup() -> (tempfile::TempDir, OpenApiLink) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("openapi")).unwrap();
        std::fs::create_dir_all(d.path().join("pets")).unwrap();
        std::fs::write(d.path().join("openapi/p.yaml"), SPEC).unwrap();
        std::fs::write(d.path().join("pets/get-pet.flux.md"), REQ).unwrap();
        let link = OpenApiLink {
            name: "p".into(),
            source: "openapi/p.yaml".into(),
            folder: Some("pets".into()),
            mode: Some("two-way".into()),
            base_url_variable: Some("baseUrl".into()),
        };
        // Initial base = spec mapping, as the importer writes it.
        let st = load(d.path(), &link, None).unwrap();
        write_base(&st, &st.spec).unwrap();
        (d, link)
    }

    #[test]
    fn clean_link_has_empty_status_and_noop_push() {
        let (d, link) = setup();
        let st = load(d.path(), &link, None).unwrap();
        assert_eq!(st.status(), SyncStatus::default());
        let r = push(&st, &st.result.changes, Deletion::Deprecate).unwrap();
        assert!(!r.written);
        assert_eq!(std::fs::read_to_string(&st.spec_path).unwrap(), SPEC);
    }

    #[test]
    fn one_pull_and_one_push_converge() {
        let (d, link) = setup();
        // Spec edits a parameter description; the app renames the request.
        std::fs::write(
            d.path().join("openapi/p.yaml"),
            SPEC.replace("Pet id", "The pet id"),
        )
        .unwrap();
        std::fs::write(
            d.path().join("pets/get-pet.flux.md"),
            REQ.replace("name: Get a pet", "name: Fetch pet"),
        )
        .unwrap();
        let st = load(d.path(), &link, None).unwrap();
        let s = st.status();
        assert_eq!((s.incoming, s.outgoing, s.conflicts), (1, 1, 0));
        pull(&st, &st.result.changes).unwrap();
        let st = load(d.path(), &link, None).unwrap();
        push(&st, &st.result.changes, Deletion::Deprecate).unwrap();
        let spec = std::fs::read_to_string(d.path().join("openapi/p.yaml")).unwrap();
        assert_eq!(
            spec,
            SPEC.replace("Pet id", "The pet id")
                .replace("Get a pet", "Fetch pet")
        );
        let st = load(d.path(), &link, None).unwrap();
        assert_eq!(st.status(), SyncStatus::default(), "both sides converged");
    }

    #[test]
    fn same_field_on_both_sides_conflicts_until_resolved() {
        let (d, link) = setup();
        std::fs::write(
            d.path().join("openapi/p.yaml"),
            SPEC.replace("summary: Get a pet", "summary: Spec name"),
        )
        .unwrap();
        std::fs::write(
            d.path().join("pets/get-pet.flux.md"),
            REQ.replace("name: Get a pet", "name: App name"),
        )
        .unwrap();
        let st = load(d.path(), &link, None).unwrap();
        assert_eq!(st.status().conflicts, 1);
        let mut res = BTreeMap::new();
        let (_, open) = st.resolved(&res);
        assert_eq!(open.len(), 1);
        res.insert(
            ("getPet".to_string(), Some("name".to_string())),
            Resolution::TakeSpec,
        );
        let (changes, open) = st.resolved(&res);
        assert!(open.is_empty());
        pull(&st, &changes).unwrap();
        assert!(
            std::fs::read_to_string(d.path().join("pets/get-pet.flux.md"))
                .unwrap()
                .contains("name: Spec name")
        );
    }
}
