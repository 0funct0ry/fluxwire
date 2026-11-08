//! Collections on disk (SPEC §6): `fluxwire.toml`, discovery, and the request tree.

use crate::parse::parse_with;
use fluxwire_model::{BlockKind, CollectionConfig, FileKind, RequestKind, TreeNode};
use fluxwire_protocol::ProtocolRegistry;
use std::path::{Path, PathBuf};

/// Highest `format` this version understands (SPEC §7.9 rule 6).
pub const SUPPORTED_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    NotFound(PathBuf),
    MissingFormat,
    FormatTooNew { found: u32, supported: u32 },
    Config(String),
    Io(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NotFound(p) => write!(f, "{} does not exist", p.display()),
            OpenError::MissingFormat => write!(
                f,
                "fluxwire.toml must declare `format = 1` under [collection]"
            ),
            OpenError::FormatTooNew { found, supported } => write!(
                f,
                "This collection uses format {found}, but this version of Fluxwire supports up to format {supported}. Update Fluxwire to open it."
            ),
            OpenError::Config(m) => write!(f, "fluxwire.toml is invalid: {m}"),
            OpenError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for OpenError {}

pub fn parse_config(text: &str) -> Result<CollectionConfig, OpenError> {
    let cfg: CollectionConfig =
        toml::from_str(text).map_err(|e| OpenError::Config(e.to_string()))?;
    match cfg.collection.format {
        0 => Err(OpenError::MissingFormat),
        n if n > SUPPORTED_FORMAT => Err(OpenError::FormatTooNew {
            found: n,
            supported: SUPPORTED_FORMAT,
        }),
        _ => Ok(cfg),
    }
}

pub fn load_config(dir: &Path) -> Result<CollectionConfig, OpenError> {
    let p = dir.join("fluxwire.toml");
    let text = std::fs::read_to_string(&p).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            OpenError::NotFound(p.clone())
        } else {
            OpenError::Io(e.to_string())
        }
    })?;
    parse_config(&text)
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "environments" | "lib" | "node_modules" | "target")
}

/// Every directory containing `fluxwire.toml` at or under `root` (depth-limited, no descent into collections).
pub fn find_collections(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if dir.join("fluxwire.toml").is_file() {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| {
                !matches!(
                    e.file_name().to_string_lossy().as_ref(),
                    "node_modules" | "target"
                )
            })
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for d in dirs {
            walk(&d, depth - 1, out);
        }
    }
    walk(root, 4, &mut out);
    out
}

#[derive(Debug, Clone)]
pub struct Collection {
    pub root: PathBuf,
    pub config: CollectionConfig,
    pub tree: Vec<TreeNode>,
    /// Relative path of `README.md`, if present.
    pub readme: Option<String>,
}

/// Forward-slash path relative to `root`, used as the stable key for tabs and tree rows.
pub fn rel_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn file_kind_for(path: &Path) -> FileKind {
    match path.file_name().and_then(|n| n.to_str()) {
        Some("folder.flux.md") => FileKind::Folder,
        Some("collection.flux.md") => FileKind::Collection,
        _ => FileKind::Request,
    }
}

pub fn is_request_file(path: &Path) -> bool {
    let n = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    n.ends_with(".flux.md") && n != "folder.flux.md" && n != "collection.flux.md"
}

pub fn scan(root: &Path) -> Result<Collection, OpenError> {
    if !root.is_dir() {
        return Err(OpenError::NotFound(root.to_path_buf()));
    }
    let config = load_config(root)?;
    let reg = ProtocolRegistry::builtin();
    let mut tree = scan_dir(root, root, &reg);
    sort_top(&mut tree, &config);
    let readme = root
        .join("README.md")
        .is_file()
        .then(|| "README.md".to_string());
    Ok(Collection {
        root: root.to_path_buf(),
        config,
        tree,
        readme,
    })
}

struct Item {
    seq: Option<i64>,
    node: TreeNode,
}

fn scan_dir(root: &Path, dir: &Path, reg: &ProtocolRegistry) -> Vec<TreeNode> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut folders: Vec<Item> = Vec::new();
    let mut reqs: Vec<Item> = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        let fname = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if skip_dir(&fname) || path.join("fluxwire.toml").is_file() {
                continue;
            }
            let (name, seq) = folder_meta(&path, &fname, reg);
            let children = scan_dir(root, &path, reg);
            folders.push(Item {
                seq,
                node: TreeNode::Folder {
                    name,
                    path: rel_key(root, &path),
                    children,
                },
            });
        } else if is_request_file(&path) {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let doc = parse_with(&text, FileKind::Request, reg);
            let stem = fname.trim_end_matches(".flux.md").to_string();
            let name = doc.name().map_or(stem, str::to_string);
            let (method, kind) = match doc.request() {
                Some(_) if doc.is_websocket() => ("WS".to_string(), RequestKind::WebSocket),
                // a GraphQL request is sent with POST or GET; the tree shows what it is
                Some(_) if doc.blocks_of(BlockKind::GraphqlQuery).next().is_some() => {
                    ("GQL".to_string(), RequestKind::Http)
                }
                Some(r) => (r.method.to_ascii_uppercase(), RequestKind::Http),
                None => ("?".to_string(), RequestKind::Other),
            };
            let kind = if doc.unsupported.is_empty() {
                kind
            } else {
                RequestKind::Other
            };
            reqs.push(Item {
                seq: doc.front.int("seq"),
                node: TreeNode::Request {
                    name,
                    path: rel_key(root, &path),
                    method,
                    kind,
                },
            });
        } else if fname.ends_with(".md")
            && fname != "collection.flux.md"
            && fname != "folder.flux.md"
            && !fname.ends_with(".flux.md")
        {
            reqs.push(Item {
                seq: None,
                node: TreeNode::Doc {
                    name: fname,
                    path: rel_key(root, &path),
                },
            });
        }
    }
    let by = |a: &Item, b: &Item| match (a.seq, b.seq) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| {
            a.node
                .name()
                .to_lowercase()
                .cmp(&b.node.name().to_lowercase())
        }),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a
            .node
            .name()
            .to_lowercase()
            .cmp(&b.node.name().to_lowercase()),
    };
    folders.sort_by(by);
    reqs.sort_by(by);
    folders.into_iter().chain(reqs).map(|i| i.node).collect()
}

fn folder_meta(dir: &Path, fname: &str, reg: &ProtocolRegistry) -> (String, Option<i64>) {
    let p = dir.join("folder.flux.md");
    match std::fs::read_to_string(p) {
        Ok(t) => {
            let d = parse_with(&t, FileKind::Folder, reg);
            (
                d.name().map_or_else(|| fname.to_string(), str::to_string),
                d.front.int("seq"),
            )
        }
        Err(_) => (fname.to_string(), None),
    }
}

/// Apply `[order] folders` to the top level: listed folders first, in the listed order (by directory name).
fn sort_top(tree: &mut [TreeNode], cfg: &CollectionConfig) {
    if cfg.order.folders.is_empty() {
        return;
    }
    let pos = |n: &TreeNode| -> usize {
        match n {
            TreeNode::Folder { path, .. } => cfg
                .order
                .folders
                .iter()
                .position(|f| f == path)
                .unwrap_or(usize::MAX),
            _ => usize::MAX,
        }
    };
    // Stable: folders named in [order] come first in that order; everything else keeps its place.
    tree.sort_by_key(|n| match n {
        TreeNode::Folder { .. } => (0, pos(n)),
        _ => (1, 0),
    });
}
