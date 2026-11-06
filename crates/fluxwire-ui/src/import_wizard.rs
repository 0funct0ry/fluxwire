//! Import wizard model (M11, SPEC §14): source, target, preview tree, report, per-file
//! overwrite choices and the import step. Pure logic over `fluxwire-import`; the dialog in
//! `import_dialog.rs` renders it.
//!
//! The plan is re-targeted to the Workspace: environment files go to the Workspace's
//! `environments/` (ADR-012); `.gitignore` goes to the Workspace root; collection files are
//! only written at a collection root, and a plain Folder gets a `folder.flux.md` instead.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fluxwire_import::{
    ImportPlan, Options, PlannedFile, PlannedKind, Source, apply, detect, plan as plan_import,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    /// `None` for a directory row.
    pub path: Option<PathBuf>,
    pub kind: Option<PlannedKind>,
    pub exists: bool,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    /// Request files written.
    pub requests: usize,
    pub report: String,
    /// Workspace-relative path of the first imported request, to open afterwards.
    pub first_request: Option<PathBuf>,
}

pub struct ImportWizard {
    pub ws_root: PathBuf,
    pub collection_roots: Vec<PathBuf>,
    pub target: PathBuf,
    pub src: Option<PathBuf>,
    pub source: Option<Source>,
    pub detected: Option<Source>,
    pub spec_link: bool,
    pub write_report: bool,
    /// The re-targeted plan (paths relative to the Workspace root).
    pub plan: Option<ImportPlan>,
    pub error: Option<String>,
    /// Existing files the user confirmed to overwrite (Workspace-relative).
    pub overwrite: BTreeSet<PathBuf>,
}

pub const ALL_SOURCES: [Source; 5] = Source::ALL;

impl ImportWizard {
    pub fn new(ws_root: PathBuf, collection_roots: Vec<PathBuf>, target: PathBuf) -> Self {
        ImportWizard {
            ws_root,
            collection_roots,
            target,
            src: None,
            source: None,
            detected: None,
            spec_link: false,
            write_report: false,
            plan: None,
            error: None,
            overwrite: BTreeSet::new(),
        }
    }

    /// The target relative to the Workspace root, `None` when it is outside.
    fn rel_target(&self) -> Option<PathBuf> {
        self.target
            .strip_prefix(&self.ws_root)
            .ok()
            .map(Path::to_path_buf)
    }

    pub fn target_is_collection_root(&self) -> bool {
        self.collection_roots.contains(&self.target)
            || !self
                .collection_roots
                .iter()
                .any(|r| self.target.starts_with(r))
    }

    /// A Spec link needs the collection root, and an OpenAPI source.
    pub fn spec_link_available(&self) -> bool {
        self.source == Some(Source::OpenApi) && self.target_is_collection_root()
    }

    /// Choose the file to import; detects the source.
    pub fn set_src(&mut self, path: PathBuf) {
        let text = if path.is_dir() {
            String::new()
        } else {
            std::fs::read_to_string(&path).unwrap_or_default()
        };
        self.detected = if path.is_dir() {
            Some(Source::Hurl)
        } else {
            detect(&path, &text)
        };
        self.src = Some(path);
        self.source = self.detected.or(self.source);
        self.overwrite.clear();
        self.replan();
    }

    pub fn set_source(&mut self, s: Source) {
        self.source = Some(s);
        self.replan();
    }

    pub fn set_target(&mut self, dir: PathBuf) {
        self.target = dir;
        self.overwrite.clear();
        self.replan();
    }

    pub fn set_spec_link(&mut self, on: bool) {
        self.spec_link = on;
        self.replan();
    }

    pub fn replan(&mut self) {
        self.plan = None;
        self.error = None;
        let (Some(src), Some(source)) = (self.src.clone(), self.source) else {
            return;
        };
        let Some(rel) = self.rel_target() else {
            self.error = Some("That location is outside the workspace.".into());
            return;
        };
        let opts = Options {
            spec_link: self.spec_link && self.spec_link_available(),
            name: None,
        };
        match plan_import(source, &src, &opts) {
            Ok(p) => self.plan = Some(self.retarget(p, &rel)),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn retarget(&self, mut p: ImportPlan, rel: &Path) -> ImportPlan {
        let root = self.target_is_collection_root();
        // An import at the root of a Workspace that is not itself a collection makes a new
        // collection folder, as New collection does, so it can be removed like any other
        // collection and the Workspace folder does not become the collection.
        let nested;
        let rel =
            if root && rel.as_os_str().is_empty() && !self.collection_roots.contains(&self.ws_root)
            {
                let name = p
                    .files
                    .iter()
                    .find(|f| f.kind == PlannedKind::Collection)
                    .and_then(|f| {
                        fluxwire_format::parse(&f.contents)
                            .front
                            .str("name")
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "imported".into());
                nested = PathBuf::from(fluxwire_format::slug(&name));
                nested.as_path()
            } else {
                rel
            };
        let mut files = Vec::new();
        for mut f in std::mem::take(&mut p.files) {
            match f.kind {
                PlannedKind::Env | PlannedKind::LocalEnv => {}
                PlannedKind::Gitignore => f.path = PathBuf::from(".gitignore"),
                PlannedKind::Config | PlannedKind::Support if !root => continue,
                PlannedKind::Collection if !root => {
                    f.kind = PlannedKind::Folder;
                    f.contents =
                        fluxwire_format::format(&f.contents, fluxwire_model::FileKind::Folder);
                    f.path = rel.join("folder.flux.md");
                }
                _ => f.path = rel.join(&f.path),
            }
            files.push(f);
        }
        if !root {
            p.report.lossy(
                "",
                "Collection settings",
                "The target is a folder inside a collection, so `fluxwire.toml` and Spec links were not written and the collection file became `folder.flux.md`",
            );
        }
        p.files = files;
        p
    }

    /// Files that already exist and need a per-file confirmation. Merged files
    /// (`.gitignore`, `fluxwire.toml`) never need one.
    pub fn conflicts(&self) -> Vec<PathBuf> {
        let Some(p) = &self.plan else {
            return Vec::new();
        };
        p.files
            .iter()
            .filter(|f| needs_confirmation(f) && self.ws_root.join(&f.path).exists())
            .map(|f| f.path.clone())
            .collect()
    }

    pub fn toggle_overwrite(&mut self, path: &Path) {
        if !self.overwrite.remove(path) {
            self.overwrite.insert(path.to_path_buf());
        }
    }

    pub fn set_all_overwrite(&mut self, on: bool) {
        self.overwrite = if on {
            self.conflicts().into_iter().collect()
        } else {
            BTreeSet::new()
        };
    }

    pub fn can_import(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| !p.files.is_empty()) && self.error.is_none()
    }

    /// The preview: directories as rows, files under them, in path order.
    pub fn tree(&self) -> Vec<TreeRow> {
        let Some(p) = &self.plan else {
            return Vec::new();
        };
        let base = self.rel_target().unwrap_or_default();
        let mut paths: Vec<&PlannedFile> = p.files.iter().collect();
        paths.sort_by(|a, b| a.path.cmp(&b.path));
        let mut rows = Vec::new();
        let mut open: Vec<String> = Vec::new();
        for f in paths {
            // environments live at the Workspace; show them from the Workspace root
            let shown: PathBuf = f
                .path
                .strip_prefix(&base)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| f.path.clone());
            let comps: Vec<String> = shown
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let (dirs, name) = comps.split_at(comps.len().saturating_sub(1));
            let mut common = 0;
            while common < open.len() && common < dirs.len() && open[common] == dirs[common] {
                common += 1;
            }
            open.truncate(common);
            for (i, d) in dirs.iter().enumerate().skip(common) {
                rows.push(TreeRow {
                    depth: i,
                    label: format!("{d}/"),
                    path: None,
                    kind: None,
                    exists: false,
                    overwrite: false,
                });
                open.push(d.clone());
            }
            let exists = needs_confirmation(f) && self.ws_root.join(&f.path).exists();
            rows.push(TreeRow {
                depth: dirs.len(),
                label: name.first().cloned().unwrap_or_default(),
                path: Some(f.path.clone()),
                kind: Some(f.kind),
                exists,
                overwrite: self.overwrite.contains(&f.path),
            });
        }
        rows
    }

    /// Markdown of the report as it will read after the import.
    pub fn report_markdown(&self) -> String {
        self.plan
            .as_ref()
            .map(|p| p.report.to_markdown(&p.files))
            .unwrap_or_default()
    }

    pub fn lossy_count(&self) -> usize {
        self.plan.as_ref().map_or(0, |p| p.report.lossy.len())
    }

    /// Write the plan into the Workspace. Existing files are kept unless confirmed.
    pub fn import(&mut self) -> Result<Outcome, String> {
        let Some(mut p) = self.plan.clone() else {
            return Err("Choose a file to import first.".into());
        };
        let Some(rel) = self.rel_target() else {
            return Err("That location is outside the workspace.".into());
        };
        let confirmed = self.overwrite.clone();
        let written = apply(&mut p, &self.ws_root, &|path| confirmed.contains(path))
            .map_err(|e| format!("Could not write the import: {e}"))?;
        let report = p.report.to_markdown(&p.files);
        let mut written = written;
        if self.write_report {
            let path = rel.join("import-report.md");
            std::fs::write(self.ws_root.join(&path), &report)
                .map_err(|e| format!("Could not write the report: {e}"))?;
            written.push(path);
        }
        let first_request = p
            .files
            .iter()
            .filter(|f| f.kind == PlannedKind::Request && written.contains(&f.path))
            .map(|f| f.path.clone())
            .min();
        let requests = p
            .files
            .iter()
            .filter(|f| f.kind == PlannedKind::Request && written.contains(&f.path))
            .count();
        Ok(Outcome {
            requests,
            written,
            skipped: p.report.skipped.clone(),
            report,
            first_request,
        })
    }
}

/// Requests, folders, collection files and environments ask before overwriting; merged and
/// supporting files do not.
fn needs_confirmation(f: &PlannedFile) -> bool {
    !matches!(f.kind, PlannedKind::Gitignore | PlannedKind::Config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/import")
            .join(rel)
    }

    fn wizard(dir: &Path) -> ImportWizard {
        ImportWizard::new(dir.to_path_buf(), vec![], dir.to_path_buf())
    }

    /// A Workspace whose folder is itself a collection.
    fn root_collection_wizard(dir: &Path) -> ImportWizard {
        ImportWizard::new(
            dir.to_path_buf(),
            vec![dir.to_path_buf()],
            dir.to_path_buf(),
        )
    }

    #[test]
    fn importing_into_an_empty_workspace_makes_a_collection_folder() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("openapi/petstore.yaml"));
        let out = w.import().unwrap();
        assert!(
            out.written
                .contains(&PathBuf::from("petstore/fluxwire.toml"))
        );
        assert!(
            out.written
                .contains(&PathBuf::from("petstore/collection.flux.md"))
        );
        assert!(!d.path().join("fluxwire.toml").exists());
        assert!(!d.path().join("collection.flux.md").exists());
    }

    #[test]
    fn set_src_detects_the_source_and_plans() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("openapi/petstore.yaml"));
        assert_eq!(w.detected, Some(Source::OpenApi));
        assert_eq!(w.source, Some(Source::OpenApi));
        assert!(w.can_import());
        assert!(w.error.is_none());
    }

    #[test]
    fn overriding_the_source_replans_and_a_wrong_one_errors() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("postman/orders.postman_collection.json"));
        assert_eq!(w.source, Some(Source::Postman));
        w.set_source(Source::Insomnia);
        assert!(w.error.is_some());
        assert!(!w.can_import());
        w.set_source(Source::Postman);
        assert!(w.error.is_none() && w.can_import());
    }

    #[test]
    fn tree_groups_files_under_directories() {
        let d = tempfile::tempdir().unwrap();
        let mut w = root_collection_wizard(d.path());
        w.set_src(fixture("insomnia/bookstore.insomnia.json"));
        let rows = w.tree();
        let dir = rows.iter().position(|r| r.label == "books/").unwrap();
        assert_eq!(rows[dir].depth, 0);
        assert!(rows[dir].path.is_none());
        let child = &rows[dir + 1];
        assert_eq!(child.depth, 1);
        assert!(child.path.is_some());
        assert!(rows.iter().any(|r| r.label == "environments/"));
    }

    #[test]
    fn existing_files_need_confirmation_and_are_kept_by_default() {
        let d = tempfile::tempdir().unwrap();
        let mut w = root_collection_wizard(d.path());
        w.set_src(fixture("hurl/health.hurl"));
        let out = w.import().unwrap();
        assert!(out.written.contains(&PathBuf::from("get-health.flux.md")));
        std::fs::write(d.path().join("get-health.flux.md"), "mine").unwrap();
        w.replan();
        let conflicts = w.conflicts();
        assert!(conflicts.contains(&PathBuf::from("get-health.flux.md")));
        assert!(!conflicts.contains(&PathBuf::from("fluxwire.toml")));
        assert!(
            w.tree()
                .iter()
                .any(|r| r.label == "get-health.flux.md" && r.exists && !r.overwrite)
        );
        let out = w.import().unwrap();
        assert!(out.skipped.contains(&PathBuf::from("get-health.flux.md")));
        assert_eq!(
            std::fs::read_to_string(d.path().join("get-health.flux.md")).unwrap(),
            "mine"
        );
        w.toggle_overwrite(Path::new("get-health.flux.md"));
        w.import().unwrap();
        assert_ne!(
            std::fs::read_to_string(d.path().join("get-health.flux.md")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn folder_target_inside_a_collection_gets_a_folder_file_and_workspace_envs() {
        let d = tempfile::tempdir().unwrap();
        let col = d.path().join("api");
        std::fs::create_dir_all(col.join("sub")).unwrap();
        let mut w = ImportWizard::new(d.path().to_path_buf(), vec![col.clone()], col.join("sub"));
        w.set_src(fixture("http/users.http"));
        let paths: Vec<_> = w
            .plan
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert!(paths.contains(&PathBuf::from("api/sub/folder.flux.md")));
        assert!(paths.contains(&PathBuf::from("api/sub/get-user.flux.md")));
        assert!(paths.contains(&PathBuf::from("environments/dev.env")));
        assert!(paths.contains(&PathBuf::from(".gitignore")));
        assert!(!paths.iter().any(|p| p.ends_with("fluxwire.toml")));
        assert!(!w.spec_link_available());
        w.import().unwrap();
        assert!(d.path().join("environments/prod.local.env").exists());
    }

    #[test]
    fn spec_link_is_only_offered_at_a_collection_root_for_openapi() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        assert!(!w.spec_link_available());
        w.set_src(fixture("openapi/petstore.yaml"));
        assert!(w.spec_link_available());
        w.set_spec_link(true);
        let paths: Vec<_> = w
            .plan
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert!(paths.contains(&PathBuf::from("petstore/.fluxwire/sync/petstore.base.json")));
        assert!(paths.contains(&PathBuf::from("petstore/petstore/petstore.yaml")));
        w.set_spec_link(false);
        let paths: Vec<_> = w
            .plan
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert!(!paths.iter().any(|p| p.starts_with(".fluxwire")));
    }

    #[test]
    fn a_target_outside_the_workspace_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("hurl/health.hurl"));
        w.set_target(other.path().to_path_buf());
        assert!(w.error.as_deref().unwrap().contains("outside"));
        assert!(!w.can_import());
        assert!(w.import().is_err());
    }

    #[test]
    fn report_can_be_written_next_to_the_import() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("postman/shop-tests.postman_collection.json"));
        assert!(w.lossy_count() >= 3);
        w.write_report = true;
        let out = w.import().unwrap();
        assert!(out.report.contains("pm.visualizer"));
        let md = std::fs::read_to_string(d.path().join("import-report.md")).unwrap();
        assert!(md.contains("Lossy conversions"));
        assert!(out.first_request.is_some());
    }

    #[test]
    fn set_all_overwrite_selects_every_conflict() {
        let d = tempfile::tempdir().unwrap();
        let mut w = wizard(d.path());
        w.set_src(fixture("hurl/health.hurl"));
        w.import().unwrap();
        w.replan();
        w.set_all_overwrite(true);
        assert_eq!(w.overwrite.len(), w.conflicts().len());
        assert!(!w.overwrite.is_empty());
        w.set_all_overwrite(false);
        assert!(w.overwrite.is_empty());
    }
}
