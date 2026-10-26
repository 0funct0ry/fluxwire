use crate::{Diagnostic, Span};
use serde::{Deserialize, Serialize};

/// Value of a front matter entry (SPEC §7.4 subset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FmValue {
    Null,
    Scalar(String),
    List(Vec<String>),
    Map(Vec<(String, String)>),
}

impl FmValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FmValue::Scalar(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FmEntry {
    pub key: String,
    /// The whole entry: key line through the last child line, without the final newline.
    pub span: Span,
    pub key_span: Span,
    pub value: FmValue,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontMatter {
    /// Whole front matter including both `---` lines (without the last newline). `None` if absent.
    pub span: Option<Span>,
    /// Span between the delimiter lines (start of first entry line to start of closing `---`).
    pub body: Span,
    pub entries: Vec<FmEntry>,
}

impl FrontMatter {
    pub fn get(&self, key: &str) -> Option<&FmEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|e| e.value.as_str())
    }

    pub fn list(&self, key: &str) -> Vec<String> {
        match self.get(key).map(|e| &e.value) {
            Some(FmValue::List(v)) => v.clone(),
            Some(FmValue::Scalar(s)) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.str(key).and_then(|s| s.trim().parse().ok())
    }

    pub fn requires(&self) -> Vec<String> {
        self.list("requires")
    }
}

/// Parsed fence info string (SPEC §7.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FenceInfo {
    pub language: String,
    pub role: Option<String>,
    pub label: Option<String>,
    pub attrs: Vec<(String, String)>,
    /// Role token span, for fixes such as "use 'example'".
    pub role_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    /// The request itself (`http`, `ws`, or a registered request language).
    Request,
    Expect,
    Form,
    Multipart,
    Defaults,
    Pre,
    Post,
    Test,
    Message,
    Config,
    Example,
    /// `x-` role: preserved, ignored by core.
    Extension,
    /// Reserved role or request language not supported by this version.
    Reserved,
    /// Role-capable language with a role nobody owns (F201).
    UnknownRole,
    /// Request language known to be reserved/unknown (not runnable).
    UnknownRequest,
    /// Plain code in prose, not owned by Fluxwire.
    Prose,
}

impl BlockKind {
    /// Order used when inserting a block that does not exist yet (SPEC §7.10).
    pub fn default_order(&self) -> u8 {
        match self {
            BlockKind::Request => 0,
            BlockKind::Expect => 1,
            BlockKind::Form | BlockKind::Multipart | BlockKind::Defaults => 1,
            BlockKind::Pre => 2,
            BlockKind::Post => 3,
            BlockKind::Test => 4,
            BlockKind::Message => 5,
            BlockKind::Config => 6,
            _ => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub info: FenceInfo,
    /// Whole fenced block from the opening fence to the end of the closing fence (no final newline).
    pub span: Span,
    /// Info string text after the fence characters.
    pub info_span: Span,
    /// Content lines between the fences (excluding the fence lines, including trailing newline of last line).
    pub content: Span,
    /// Whether the fence starts at column 0 (only those are normalised by the formatter).
    pub top_level: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryParam {
    pub enabled: bool,
    pub name: String,
    pub value: Option<String>,
    /// Whole line, without newline.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub enabled: bool,
    pub name: String,
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    None,
    Raw(Span),
    /// `< path`
    File {
        path: String,
        span: Span,
    },
}

/// Parsed `http` / `ws` request block (SPEC §7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub method_span: Span,
    pub target: String,
    pub target_span: Span,
    pub version: Option<String>,
    pub request_line: Span,
    pub query: Vec<QueryParam>,
    pub headers: Vec<Header>,
    /// Comment lines in the head that are not disabled entries.
    pub comments: Vec<Span>,
    /// First head line through last head line (no trailing newline).
    pub head: Span,
    pub body: Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusMatch {
    Exact(u16),
    Class(u8),
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectBlock {
    pub status: StatusMatch,
    pub version: Option<String>,
    pub headers: Vec<Header>,
    pub body: Body,
}

/// A saved message of a session protocol such as WebSocket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBlock {
    /// Index into `FluxDoc::blocks`.
    pub index: usize,
    pub label: String,
    /// `json`, `text`, `xml` or `binary`.
    pub language: String,
    /// Block content without its final line break.
    pub text: String,
    /// Path from `@file(path)` or `< path`, for a body read from disk (binary messages).
    pub file: Option<String>,
}

impl MessageBlock {
    fn file_ref(text: &str) -> Option<String> {
        let t = text.trim();
        if t.lines().count() != 1 {
            return None;
        }
        let path = t
            .strip_prefix("@file(")
            .and_then(|r| r.strip_suffix(')'))
            .map(|p| p.trim().trim_matches('"'))
            .or_else(|| t.strip_prefix("< ").map(str::trim))?;
        (!path.is_empty()).then(|| path.to_string())
    }

    /// The frame kind this message is sent as.
    pub fn frame_kind(&self) -> &'static str {
        if self.language == "binary" {
            "binary"
        } else {
            "text"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvLine {
    pub enabled: bool,
    pub name: String,
    pub value: String,
    pub span: Span,
}

/// Typed payload of a block, when the core understands it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockData {
    None,
    Request(HttpRequest),
    Expect(ExpectBlock),
    Kv(Vec<KvLine>),
}

/// Kind of file (SPEC §7.4 `kind`, or implied by file name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Request,
    Folder,
    Collection,
}

/// A parsed `.flux.md` file. `source` is kept byte for byte; everything else is spans into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FluxDoc {
    pub source: String,
    pub front: FrontMatter,
    pub blocks: Vec<Block>,
    pub data: Vec<BlockData>,
    pub diagnostics: Vec<Diagnostic>,
    /// Features listed in `requires` (or implied by reserved request languages) not supported here.
    pub unsupported: Vec<String>,
    pub kind: FileKind,
}

impl FluxDoc {
    pub fn runnable(&self) -> bool {
        self.unsupported.is_empty() && self.request_index().is_some()
    }

    pub fn request_index(&self) -> Option<usize> {
        self.blocks
            .iter()
            .position(|b| b.kind == BlockKind::Request)
    }

    pub fn request(&self) -> Option<&HttpRequest> {
        match self.data.get(self.request_index()?)? {
            BlockData::Request(r) => Some(r),
            _ => None,
        }
    }

    pub fn is_websocket(&self) -> bool {
        self.request_index()
            .is_some_and(|i| self.blocks[i].info.language == "ws")
    }

    pub fn blocks_of(&self, kind: BlockKind) -> impl Iterator<Item = (usize, &Block)> {
        self.blocks
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.kind == kind)
    }

    /// Saved `<lang> message <label>` blocks in file order (SPEC §7.5).
    pub fn messages(&self) -> Vec<MessageBlock> {
        self.blocks_of(BlockKind::Message)
            .map(|(i, b)| {
                let text = self.block_text(i);
                let text = text
                    .strip_suffix('\n')
                    .map(|t| t.strip_suffix('\r').unwrap_or(t))
                    .unwrap_or(text);
                MessageBlock {
                    index: i,
                    label: b.info.label.clone().unwrap_or_else(|| "message".into()),
                    language: b.info.language.clone(),
                    file: MessageBlock::file_ref(text),
                    text: text.to_string(),
                }
            })
            .collect()
    }

    pub fn block_text(&self, i: usize) -> &str {
        self.blocks[i].content.slice(&self.source)
    }

    pub fn name(&self) -> Option<&str> {
        self.front.str("name")
    }

    /// Blocks and front matter keys shown in the "Other content" strip.
    pub fn other_content(&self) -> Vec<String> {
        const KNOWN: &[&str] = &[
            "requires",
            "name",
            "kind",
            "seq",
            "tags",
            "operation",
            "deprecated",
            "auth",
            "path",
            "vars",
            "capture",
            "settings",
        ];
        let mut out = Vec::new();
        for e in &self.front.entries {
            if !KNOWN.contains(&e.key.as_str()) {
                out.push(format!("front matter: {}", e.key));
            }
        }
        for b in &self.blocks {
            match b.kind {
                BlockKind::Extension
                | BlockKind::Reserved
                | BlockKind::UnknownRole
                | BlockKind::UnknownRequest => {
                    let mut s = b.info.language.clone();
                    if let Some(r) = &b.info.role {
                        s.push(' ');
                        s.push_str(r);
                    }
                    out.push(format!("block: {s}"));
                }
                _ => {}
            }
        }
        out
    }
}
