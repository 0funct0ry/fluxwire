//! Containment rules for creating and adding Workspaces and Collections (SPEC §17.9): a
//! Collection lives inside the Workspace root, Collections never nest, and the path must be one
//! `find_collections` discovers (at most 4 levels deep, not hidden, not `node_modules`/`target`).
//! Pure functions over the file system so the modals and the tests share one rule set.

use std::{
    fmt,
    path::{Component, Path, PathBuf},
};

use crate::workspace_model::Workspace;

/// Deepest level below the Workspace root at which `find_collections` still looks.
pub const MAX_DEPTH: usize = 4;

/// Resolve symlinks for the deepest existing ancestor and re-attach the rest, so paths that do
/// not exist yet still compare correctly with ones that do.
pub fn normalize(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            out.extend(rest.iter().rev());
            return out;
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent()) {
            (Some(n), Some(parent)) => {
                rest.push(n);
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// `Payments API` → `payments-api`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn bad_name(name: &str) -> bool {
    name == "."
        || name == ".."
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
}

// ---- New workspace ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewWorkspaceError {
    EmptyName,
    BadName,
    ParentMissing,
    ParentNotWritable,
    IsFile,
    /// The target exists and holds files; offer to open it instead.
    ExistsNonEmpty(PathBuf),
}

impl fmt::Display for NewWorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "Enter a name."),
            Self::BadName => write!(
                f,
                "The name can't contain path separators or reserved characters."
            ),
            Self::ParentMissing => write!(f, "The location doesn't exist."),
            Self::ParentNotWritable => write!(f, "The location isn't writable."),
            Self::IsFile => write!(f, "A file with this name already exists."),
            Self::ExistsNonEmpty(_) => {
                write!(
                    f,
                    "A folder with this name already exists. Open it instead?"
                )
            }
        }
    }
}

/// The directory a new Workspace would be created at. An existing empty directory is allowed.
pub fn plan_new_workspace(name: &str, parent: &Path) -> Result<PathBuf, NewWorkspaceError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NewWorkspaceError::EmptyName);
    }
    if bad_name(name) {
        return Err(NewWorkspaceError::BadName);
    }
    if !parent.is_dir() {
        return Err(NewWorkspaceError::ParentMissing);
    }
    let target = parent.join(name);
    if target.is_file() {
        return Err(NewWorkspaceError::IsFile);
    }
    if target.is_dir() {
        let empty = std::fs::read_dir(&target).is_ok_and(|mut d| d.next().is_none());
        return if empty {
            Ok(target)
        } else {
            Err(NewWorkspaceError::ExistsNonEmpty(target))
        };
    }
    let writable = std::fs::metadata(parent).is_ok_and(|m| !m.permissions().readonly());
    if !writable {
        return Err(NewWorkspaceError::ParentNotWritable);
    }
    Ok(target)
}

// ---- Collections -----------------------------------------------------------------------------

/// Why a directory cannot be a Collection of this Workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Outside,
    /// Inside the Collection at this path (Collections do not nest).
    Nested(PathBuf),
    /// Would swallow Collections already in the Workspace.
    Contains,
    TooDeep,
    Hidden(String),
    Excluded(String),
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outside => write!(
                f,
                "It is outside this workspace. Collections must live inside the workspace folder."
            ),
            Self::Nested(p) => write!(
                f,
                "It is inside the collection {}. Collections can't be nested.",
                p.display()
            ),
            Self::Contains => write!(
                f,
                "It contains collections already in this workspace. Collections can't be nested."
            ),
            Self::TooDeep => write!(
                f,
                "It is more than {MAX_DEPTH} levels below the workspace folder, so it would not be discovered."
            ),
            Self::Hidden(d) => write!(
                f,
                "It is inside the hidden folder {d}, so it would not be discovered."
            ),
            Self::Excluded(d) => write!(
                f,
                "It is inside {d}, which is never searched for collections, so it would not be discovered."
            ),
        }
    }
}

/// Check `dir` against the containment rules. `dir` may be the Workspace root itself.
pub fn check_rules(ws: &Workspace, dir: &Path) -> Option<Rule> {
    let root = normalize(&ws.root);
    let dir = normalize(dir);
    let Ok(rel) = dir.strip_prefix(&root) else {
        return Some(Rule::Outside);
    };
    // Nesting: the root or any directory above `dir` (down to its parent) is a Collection.
    let mut cur = root.clone();
    let comps: Vec<_> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let mut above = vec![cur.clone()];
    for c in comps.iter().take(comps.len().saturating_sub(1)) {
        cur = cur.join(c);
        above.push(cur.clone());
    }
    if let Some(a) = above
        .iter()
        .filter(|a| **a != dir)
        .find(|a| a.join("fluxwire.toml").is_file())
    {
        return Some(Rule::Nested(a.clone()));
    }
    if comps.len() > MAX_DEPTH {
        return Some(Rule::TooDeep);
    }
    for c in &comps {
        if c.starts_with('.') {
            return Some(Rule::Hidden(c.clone()));
        }
        if c == "node_modules" || c == "target" {
            return Some(Rule::Excluded(c.clone()));
        }
    }
    if ws
        .collections
        .iter()
        .any(|c| normalize(&c.root) != dir && normalize(&c.root).starts_with(&dir))
    {
        return Some(Rule::Contains);
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewCollectionError {
    EmptyName,
    BadName,
    Rule(Rule),
    /// The target is already a Collection.
    Exists,
    IsFile,
    /// The target holds files but no `fluxwire.toml`; offer to make that folder a Collection.
    NonEmptyFolder(PathBuf),
}

impl fmt::Display for NewCollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "Enter a name."),
            Self::BadName => write!(f, "The name needs at least one letter or digit."),
            Self::Rule(r) => write!(f, "{r}"),
            Self::Exists => write!(f, "A collection already exists there."),
            Self::IsFile => write!(f, "A file with this name already exists."),
            Self::NonEmptyFolder(_) => {
                write!(
                    f,
                    "That folder already has files. Make that folder a collection instead?"
                )
            }
        }
    }
}

/// Where "New collection" with this name and location would create the Collection.
pub fn plan_new_collection(
    ws: &Workspace,
    name: &str,
    location: &Path,
) -> Result<PathBuf, NewCollectionError> {
    if name.trim().is_empty() {
        return Err(NewCollectionError::EmptyName);
    }
    let slug = slug(name);
    if slug.is_empty() || bad_name(&slug) {
        return Err(NewCollectionError::BadName);
    }
    let target = location.join(slug);
    if let Some(rule) = check_rules(ws, &target) {
        return Err(NewCollectionError::Rule(rule));
    }
    if target.join("fluxwire.toml").is_file() {
        return Err(NewCollectionError::Exists);
    }
    if target.is_file() {
        return Err(NewCollectionError::IsFile);
    }
    if target.is_dir() && std::fs::read_dir(&target).is_ok_and(|mut d| d.next().is_some()) {
        return Err(NewCollectionError::NonEmptyFolder(target));
    }
    Ok(target)
}

/// The outcome of "Add existing collection…" for a chosen directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// Already loaded; the name to show in the notice.
    AlreadyLoaded(PathBuf),
    /// Has `fluxwire.toml` and is discoverable: rescan and reveal.
    Reveal(PathBuf),
    /// No `fluxwire.toml`: confirm, then `create_collection`.
    MakeCollection(PathBuf),
    /// Cannot be part of this Workspace; nothing changes.
    Refused(Rule),
}

pub fn classify_add(ws: &Workspace, dir: &Path) -> AddOutcome {
    if let Some(rule) = check_rules(ws, dir) {
        return AddOutcome::Refused(rule);
    }
    let n = normalize(dir);
    if ws.collections.iter().any(|c| normalize(&c.root) == n) {
        return AddOutcome::AlreadyLoaded(dir.to_path_buf());
    }
    if dir.join("fluxwire.toml").is_file() {
        AddOutcome::Reveal(dir.to_path_buf())
    } else {
        AddOutcome::MakeCollection(dir.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fluxwire-contain-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn collection(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fluxwire_format::create_collection(dir, "c").unwrap();
    }

    #[test]
    fn new_workspace_validation() {
        let parent = tmp("nw");
        assert_eq!(
            plan_new_workspace(" ", &parent),
            Err(NewWorkspaceError::EmptyName)
        );
        assert_eq!(
            plan_new_workspace("a/b", &parent),
            Err(NewWorkspaceError::BadName)
        );
        assert_eq!(
            plan_new_workspace("a?b", &parent),
            Err(NewWorkspaceError::BadName)
        );
        assert_eq!(
            plan_new_workspace("x", &parent.join("missing")),
            Err(NewWorkspaceError::ParentMissing)
        );
        fs::write(parent.join("f"), "x").unwrap();
        assert_eq!(
            plan_new_workspace("f", &parent),
            Err(NewWorkspaceError::IsFile)
        );
        fs::create_dir_all(parent.join("full")).unwrap();
        fs::write(parent.join("full/a"), "x").unwrap();
        assert!(matches!(
            plan_new_workspace("full", &parent),
            Err(NewWorkspaceError::ExistsNonEmpty(_))
        ));
        fs::create_dir_all(parent.join("empty")).unwrap();
        assert_eq!(
            plan_new_workspace("empty", &parent),
            Ok(parent.join("empty"))
        );
        assert_eq!(plan_new_workspace("demo", &parent), Ok(parent.join("demo")));
    }

    #[test]
    fn new_collection_rules() {
        let root = tmp("nc");
        collection(&root.join("existing"));
        let ws = Workspace::open(&root).unwrap();
        let plan = |name: &str, loc: &Path| plan_new_collection(&ws, name, loc);
        assert_eq!(plan("", &root), Err(NewCollectionError::EmptyName));
        assert_eq!(plan("Payments API", &root), Ok(root.join("payments-api")));
        assert_eq!(
            plan("x", root.parent().unwrap()),
            Err(NewCollectionError::Rule(Rule::Outside))
        );
        assert!(matches!(
            plan("inner", &root.join("existing")),
            Err(NewCollectionError::Rule(Rule::Nested(_)))
        ));
        assert_eq!(plan("existing", &root), Err(NewCollectionError::Exists));
        let deep = root.join("a/b/c/d");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(plan("ok", &root.join("a/b/c")), Ok(root.join("a/b/c/ok")));
        assert_eq!(
            plan("too", &deep),
            Err(NewCollectionError::Rule(Rule::TooDeep))
        );
        for (dir, want) in [
            (".hidden", Rule::Hidden(".hidden".into())),
            ("node_modules", Rule::Excluded("node_modules".into())),
            ("target", Rule::Excluded("target".into())),
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
            assert_eq!(
                plan("x", &root.join(dir)),
                Err(NewCollectionError::Rule(want))
            );
        }
        fs::create_dir_all(root.join("stuff")).unwrap();
        fs::write(root.join("stuff/readme"), "x").unwrap();
        assert_eq!(
            plan("stuff", &root),
            Err(NewCollectionError::NonEmptyFolder(root.join("stuff")))
        );
    }

    #[test]
    fn add_existing_outcomes() {
        let root = tmp("add");
        collection(&root.join("loaded"));
        collection(&root.join("deep/a/b/c/d/e"));
        fs::create_dir_all(root.join("plain")).unwrap();
        let outside = tmp("add-outside");
        let ws = Workspace::open(&root).unwrap();
        assert!(matches!(
            classify_add(&ws, &root.join("loaded")),
            AddOutcome::AlreadyLoaded(_)
        ));
        assert!(matches!(
            classify_add(&ws, &root.join("plain")),
            AddOutcome::MakeCollection(_)
        ));
        assert_eq!(
            classify_add(&ws, &root.join("deep/a/b/c/d/e")),
            AddOutcome::Refused(Rule::TooDeep)
        );
        assert_eq!(
            classify_add(&ws, &outside),
            AddOutcome::Refused(Rule::Outside)
        );
        collection(&root.join("late"));
        // loaded before the rescan: has fluxwire.toml and is discoverable
        assert!(matches!(
            classify_add(&ws, &root.join("late")),
            AddOutcome::Reveal(_)
        ));
        // the root would swallow `loaded`
        assert_eq!(
            classify_add(&ws, &root),
            AddOutcome::Refused(Rule::Contains)
        );
    }

    #[test]
    fn root_collection_blocks_everything_inside() {
        let root = tmp("single");
        collection(&root);
        let ws = Workspace::open(&root).unwrap();
        assert!(matches!(
            plan_new_collection(&ws, "x", &root),
            Err(NewCollectionError::Rule(Rule::Nested(_)))
        ));
    }
}
