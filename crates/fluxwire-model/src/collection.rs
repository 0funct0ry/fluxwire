use serde::{Deserialize, Serialize};

/// `fluxwire.toml` (SPEC §6.1). Unknown keys are ignored on read; the file is never rewritten wholesale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionConfig {
    #[serde(default)]
    pub collection: CollectionSection,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub order: Order,
    #[serde(default)]
    pub secrets: Secrets,
    #[serde(default)]
    pub openapi: Vec<OpenApiLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSection {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defaults {
    pub script_language: Option<String>,
    pub timeout_ms: Option<u64>,
    pub follow_redirects: Option<bool>,
    pub max_redirects: Option<u32>,
    pub concurrency: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    #[serde(default)]
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiLink {
    pub name: String,
    pub source: String,
    pub folder: Option<String>,
    pub mode: Option<String>,
    pub base_url_variable: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Http,
    WebSocket,
    /// Unsupported or unparseable request language; editable, not runnable.
    Other,
}

/// One node in the project tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeNode {
    Folder {
        name: String,
        path: String,
        children: Vec<TreeNode>,
    },
    Request {
        name: String,
        path: String,
        method: String,
        kind: RequestKind,
    },
    /// A `.md` file that is not a request (shown as docs).
    Doc { name: String, path: String },
}

impl TreeNode {
    pub fn path(&self) -> &str {
        match self {
            TreeNode::Folder { path, .. }
            | TreeNode::Request { path, .. }
            | TreeNode::Doc { path, .. } => path,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            TreeNode::Folder { name, .. }
            | TreeNode::Request { name, .. }
            | TreeNode::Doc { name, .. } => name,
        }
    }
}
