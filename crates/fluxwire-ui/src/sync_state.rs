//! Pure model behind the Sync tab, panel and status bar item (SPEC §15.3): one row per Spec
//! link with its status, changes, conflicts and "needs manual edit" list, plus the user's
//! include/exclude marks and conflict resolutions. No GPUI; the views only read and call this.

use fluxwire_model::OpenApiLink;
use fluxwire_openapi::link::{self, Resolution, SyncStatus};
use fluxwire_openapi::push::{Deletion, NeedsManual};
use fluxwire_openapi::{Change, ChangeKind, Conflict, Direction};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// One Spec link of one collection.
#[derive(Debug, Clone)]
pub struct LinkRow {
    pub collection_root: PathBuf,
    pub link: OpenApiLink,
    pub status: SyncStatus,
    pub changes: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    pub manual: Vec<NeedsManual>,
    /// Why the link could not be loaded (missing spec, parse error, URL source).
    pub error: Option<String>,
}

pub type ConflictKey = (usize, String, Option<String>);

/// Stable identity of a change, for include/exclude marks across refreshes.
pub fn change_key(c: &Change) -> String {
    match &c.kind {
        ChangeKind::Field { field, .. } => format!("{}|{field}", c.operation),
        ChangeKind::Add(_) => format!("{}|+", c.operation),
        ChangeKind::Delete => format!("{}|-", c.operation),
    }
}

pub fn describe(c: &Change) -> (&'static str, String, String) {
    let arrow = if c.direction == Direction::Incoming {
        "↓"
    } else {
        "↑"
    };
    let what = match &c.kind {
        ChangeKind::Field { field, .. } => field.clone(),
        ChangeKind::Add(_) => "added".to_string(),
        ChangeKind::Delete => "deleted".to_string(),
    };
    (arrow, c.operation.clone(), what)
}

#[derive(Debug, Default)]
pub struct SyncState {
    pub rows: Vec<LinkRow>,
    /// `(link index, change key)` marked excluded.
    pub excluded: HashSet<(usize, String)>,
    pub resolutions: BTreeMap<ConflictKey, Resolution>,
    pub deletion: Option<Deletion>,
    pub notice: Option<String>,
}

impl SyncState {
    pub fn has_links(&self) -> bool {
        !self.rows.is_empty()
    }

    pub fn deletion(&self) -> Deletion {
        self.deletion.unwrap_or(Deletion::Deprecate)
    }

    /// Reload every link. Marks and resolutions for things that no longer exist are dropped.
    pub fn refresh(&mut self, collections: &[(PathBuf, Vec<OpenApiLink>)]) {
        let mut rows = Vec::new();
        for (root, links) in collections {
            for l in links {
                rows.push(match link::load(root, l, None) {
                    Ok(st) => {
                        let status = st.status();
                        let manual = fluxwire_openapi::push::plan_push(
                            &st.spec_root,
                            &st.spec,
                            &st.result.changes,
                            Deletion::Deprecate,
                        )
                        .manual;
                        LinkRow {
                            collection_root: root.clone(),
                            link: l.clone(),
                            status,
                            changes: st.result.changes.clone(),
                            conflicts: st.result.conflicts.clone(),
                            manual,
                            error: None,
                        }
                    }
                    Err(e) => LinkRow {
                        collection_root: root.clone(),
                        link: l.clone(),
                        status: SyncStatus::default(),
                        changes: vec![],
                        conflicts: vec![],
                        manual: vec![],
                        error: Some(e.to_string()),
                    },
                });
            }
        }
        let live: HashSet<(usize, String)> = rows
            .iter()
            .enumerate()
            .flat_map(|(i, r)| r.changes.iter().map(move |c| (i, change_key(c))))
            .collect();
        self.excluded.retain(|k| live.contains(k));
        let open: HashSet<ConflictKey> = rows
            .iter()
            .enumerate()
            .flat_map(|(i, r)| {
                r.conflicts
                    .iter()
                    .map(move |c| (i, c.operation.clone(), c.field.clone()))
            })
            .collect();
        self.resolutions.retain(|k, _| open.contains(k));
        self.rows = rows;
    }

    /// (incoming, outgoing, conflicts) over all links, for the status bar item.
    pub fn totals(&self) -> (usize, usize, usize) {
        self.rows.iter().fold((0, 0, 0), |a, r| {
            (
                a.0 + r.status.incoming,
                a.1 + r.status.outgoing,
                a.2 + r.status.conflicts,
            )
        })
    }

    /// Conflicts of link `i` that have no resolution yet.
    pub fn open_conflicts(&self, i: usize) -> usize {
        self.rows.get(i).map_or(0, |r| {
            r.conflicts
                .iter()
                .filter(|c| {
                    !self
                        .resolutions
                        .contains_key(&(i, c.operation.clone(), c.field.clone()))
                })
                .count()
        })
    }

    pub fn toggle(&mut self, link: usize, key: &str) {
        let k = (link, key.to_string());
        if !self.excluded.remove(&k) {
            self.excluded.insert(k);
        }
    }

    pub fn is_excluded(&self, link: usize, c: &Change) -> bool {
        self.excluded.contains(&(link, change_key(c)))
    }

    pub fn resolve(&mut self, link: usize, op: &str, field: Option<&str>, r: Resolution) {
        self.resolutions
            .insert((link, op.to_string(), field.map(String::from)), r);
    }

    /// The changes of link `i` to apply in `dir`: marked-in, with settled conflicts folded in.
    fn selected(
        &self,
        i: usize,
        st: &link::LinkState,
        dir: Direction,
    ) -> Result<Vec<Change>, String> {
        let res: BTreeMap<(String, Option<String>), Resolution> = self
            .resolutions
            .iter()
            .filter(|(k, _)| k.0 == i)
            .map(|(k, v)| ((k.1.clone(), k.2.clone()), v.clone()))
            .collect();
        let (changes, open) = st.resolved(&res);
        if !open.is_empty() {
            return Err(format!("{} unresolved conflict(s)", open.len()));
        }
        Ok(changes
            .into_iter()
            .filter(|c| c.direction == dir && !self.excluded.contains(&(i, change_key(c))))
            .collect())
    }

    fn load(&self, i: usize) -> Result<link::LinkState, String> {
        let r = self.rows.get(i).ok_or("no such link")?;
        link::load(&r.collection_root, &r.link, None).map_err(|e| e.to_string())
    }

    /// Pull the selected incoming changes of link `i`. Returns a one-line summary.
    pub fn pull(&mut self, i: usize) -> Result<String, String> {
        let st = self.load(i)?;
        let changes = self.selected(i, &st, Direction::Incoming)?;
        let r = link::pull(&st, &changes).map_err(|e| e.to_string())?;
        Ok(format!(
            "Pulled: {} file(s) updated, {} created, {} removed",
            r.files_changed.len(),
            r.files_created.len(),
            r.files_trashed.len()
        ))
    }

    /// Push the selected outgoing changes of link `i`.
    pub fn push(&mut self, i: usize) -> Result<String, String> {
        let st = self.load(i)?;
        let changes = self.selected(i, &st, Direction::Outgoing)?;
        let r = link::push(&st, &changes, self.deletion()).map_err(|e| e.to_string())?;
        let mut s = if r.written {
            "Pushed: spec updated".to_string()
        } else {
            "Spec unchanged".to_string()
        };
        if !r.manual.is_empty() {
            s.push_str(&format!(", {} need a manual edit", r.manual.len()));
        }
        if !r.patch.is_empty() {
            s.push_str(&format!(", {} shown as a patch", r.patch.len()));
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = "openapi: 3.0.3\npaths:\n  /a:\n    get:\n      operationId: getA\n      summary: A\n      responses:\n        '200': {description: ok}\n";
    const REQ: &str = "---\nname: A\noperation: getA\n---\n\n# A\n\n```http\nGET {{baseUrl}}/a\n```\n\n```http expect\nHTTP 200\n```\n";

    fn setup() -> (tempfile::TempDir, Vec<(PathBuf, Vec<OpenApiLink>)>) {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("api")).unwrap();
        std::fs::write(r.join("s.yaml"), SPEC).unwrap();
        std::fs::write(r.join("api/a.flux.md"), REQ).unwrap();
        let link = OpenApiLink {
            name: "s".into(),
            source: "s.yaml".into(),
            folder: Some("api".into()),
            mode: None,
            base_url_variable: None,
        };
        let st = link::load(r, &link, None).unwrap();
        std::fs::create_dir_all(r.join(".fluxwire/sync")).unwrap();
        std::fs::write(r.join(".fluxwire/sync/s.base.json"), st.spec.to_base_json()).unwrap();
        let cols = vec![(r.to_path_buf(), vec![link])];
        (d, cols)
    }

    #[test]
    fn clean_link_shows_zero_and_totals_add_up() {
        let (_d, cols) = setup();
        let mut s = SyncState::default();
        s.refresh(&cols);
        assert_eq!(s.totals(), (0, 0, 0));
        assert!(s.has_links());
    }

    #[test]
    fn excluded_changes_are_not_pulled_and_marks_survive_refresh() {
        let (d, cols) = setup();
        std::fs::write(
            d.path().join("s.yaml"),
            SPEC.replace("summary: A", "summary: B"),
        )
        .unwrap();
        let mut s = SyncState::default();
        s.refresh(&cols);
        assert_eq!(s.totals(), (1, 0, 0));
        let key = change_key(&s.rows[0].changes[0]);
        s.toggle(0, &key);
        s.refresh(&cols);
        assert!(s.is_excluded(0, &s.rows[0].changes[0].clone()));
        s.pull(0).unwrap();
        assert!(
            std::fs::read_to_string(d.path().join("api/a.flux.md"))
                .unwrap()
                .contains("name: A")
        );
        s.toggle(0, &key);
        s.pull(0).unwrap();
        s.refresh(&cols);
        assert_eq!(s.totals(), (0, 0, 0));
        assert!(
            std::fs::read_to_string(d.path().join("api/a.flux.md"))
                .unwrap()
                .contains("name: B")
        );
    }

    #[test]
    fn conflicts_block_until_resolved() {
        let (d, cols) = setup();
        std::fs::write(
            d.path().join("s.yaml"),
            SPEC.replace("summary: A", "summary: S"),
        )
        .unwrap();
        std::fs::write(
            d.path().join("api/a.flux.md"),
            REQ.replace("name: A", "name: C"),
        )
        .unwrap();
        let mut s = SyncState::default();
        s.refresh(&cols);
        assert_eq!(s.totals().2, 1);
        assert_eq!(s.open_conflicts(0), 1);
        assert!(s.pull(0).is_err() && s.push(0).is_err());
        s.resolve(0, "getA", Some("name"), Resolution::TakeCollection);
        assert_eq!(s.open_conflicts(0), 0);
        s.push(0).unwrap();
        assert!(
            std::fs::read_to_string(d.path().join("s.yaml"))
                .unwrap()
                .contains("summary: C")
        );
    }

    #[test]
    fn broken_link_reports_error_row() {
        let (d, cols) = setup();
        std::fs::remove_file(d.path().join("s.yaml")).unwrap();
        let mut s = SyncState::default();
        s.refresh(&cols);
        assert!(s.rows[0].error.is_some());
        assert_eq!(s.totals(), (0, 0, 0));
    }
}
