//! QuickJS runtime, TS stripping, fw API, sandbox limits (SPEC §11).
//!
//! The crate is protocol-free: requests and responses cross the boundary as JSON "views"
//! produced by the active `Protocol` (headers as `[[name, value]]`, `body`/`text` strings, …).
//! Scripts record what they change (variables, request edits, tests, console lines) in a
//! [`ScriptState`]; the runner applies it.

mod engine;
mod native;
mod transpile;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

pub use engine::run_blocks;
pub use native::{query_json, validate_schema};

/// Script language of a block (`ts` or `js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ts,
    Js,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ts" => Some(Self::Ts),
            "js" => Some(Self::Js),
            _ => None,
        }
    }
}

/// Which phase a block belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Pre,
    Post,
    Test,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
            Self::Test => "test",
        }
    }
}

/// One fenced script block, with enough position data to map errors back to the file.
#[derive(Debug, Clone)]
pub struct ScriptBlock {
    pub role: Role,
    pub lang: Lang,
    pub source: String,
    /// Display path of the `.flux.md` file (collection-relative).
    pub file: String,
    /// 1-based line in the file of the first line of `source`.
    pub first_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Uncaught exception (S001).
    Throw,
    /// Wall-time limit hit (S002).
    Timeout,
    /// Heap limit hit (S003).
    Memory,
    /// Import outside `lib/` or unresolvable (S004).
    Import,
    /// Parse failure (S005).
    Syntax,
    /// `pm` member without a Fluxwire equivalent (S006).
    Unsupported,
}

impl ErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Throw => "S001",
            Self::Timeout => "S002",
            Self::Memory => "S003",
            Self::Import => "S004",
            Self::Syntax => "S005",
            Self::Unsupported => "S006",
        }
    }
}

/// A script failure, positioned in the original `.flux.md` file.
#[derive(Debug, Clone)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
    pub file: String,
    /// 1-based line in `file` (0 when unknown).
    pub line: u32,
    pub col: u32,
    pub stack: String,
}

impl ScriptError {
    pub(crate) fn syntax(message: &str, file: &str, line: u32) -> Self {
        Self {
            kind: ErrorKind::Syntax,
            message: message.to_string(),
            file: file.to_string(),
            line,
            col: 0,
            stack: String::new(),
        }
    }

    pub fn location(&self) -> String {
        if self.line > 0 {
            format!("{}:{}", self.file, self.line)
        } else {
            self.file.clone()
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} ({})",
            self.kind.code(),
            self.message,
            self.location()
        )
    }
}

impl std::error::Error for ScriptError {}

/// Per-execution limits (SPEC §11.1). Configurable per collection.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout_ms: u64,
    pub memory_mb: usize,
    pub stack_kb: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout_ms: 5_000,
            memory_mb: 64,
            stack_kb: 512,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarEntry {
    pub value: String,
    pub secret: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Log,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub level: LogLevel,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub ts_ms: u64,
}

#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub name: String,
    pub passed: bool,
    pub message: Option<String>,
    pub expected: Option<String>,
    pub actual: Option<String>,
}

/// Everything scripts can read and the record of what they changed.
#[derive(Debug, Clone, Default)]
pub struct ScriptState {
    /// Exchange-shaped request view from the protocol (None for non-request contexts).
    pub request: Option<Value>,
    pub response: Option<Value>,
    /// Resolved variables visible to scripts.
    pub vars: BTreeMap<String, VarEntry>,
    pub env_vars: BTreeMap<String, String>,
    pub env_name: String,
    pub globals: BTreeMap<String, String>,
    // ---- outputs ----
    /// `Some` = set, `None` = unset; applied to the runtime scope by the runner.
    pub vars_changed: BTreeMap<String, Option<VarEntry>>,
    pub env_set: BTreeMap<String, String>,
    pub persist: Vec<String>,
    /// Request edits in call order (protocol-defined JSON ops).
    pub request_ops: Vec<Value>,
    pub skip: Option<String>,
    pub console: Vec<ConsoleLine>,
    pub tests: Vec<TestOutcome>,
}

/// Lets `fw.sendRequest` reach the runner without this crate knowing about protocols.
pub trait ScriptHost: Send + Sync {
    /// Sends another request (a collection-relative `.flux.md` path, or an ad-hoc
    /// request object) and returns its response view.
    fn send_request(
        &self,
        target: &Value,
        vars: &BTreeMap<String, String>,
    ) -> Result<Value, String>;
}

/// Environment a script phase runs in.
#[derive(Clone, Default)]
pub struct RunEnv {
    pub limits: Limits,
    /// `<collection>/lib`; imports resolve only inside it. `None` disables imports.
    pub lib_dir: Option<PathBuf>,
    pub host: Option<Arc<dyn ScriptHost>>,
}

/// The `pre`/`post`/`test` blocks of `doc` with `role`, in file order. `file` is the display
/// path used in error locations. Blocks in a language other than `ts`/`js` are skipped.
pub fn blocks_from_doc(doc: &fluxwire_model::FluxDoc, file: &str, role: Role) -> Vec<ScriptBlock> {
    use fluxwire_model::BlockKind;
    let kind = match role {
        Role::Pre => BlockKind::Pre,
        Role::Post => BlockKind::Post,
        Role::Test => BlockKind::Test,
    };
    doc.blocks_of(kind)
        .filter_map(|(i, b)| {
            let lang = Lang::parse(&b.info.language)?;
            let first_line = 1 + doc.source[..b.content.start.min(doc.source.len())]
                .bytes()
                .filter(|c| *c == b'\n')
                .count() as u32;
            Some(ScriptBlock {
                role,
                lang,
                source: doc.block_text(i).to_string(),
                file: file.to_string(),
                first_line,
            })
        })
        .collect()
}

/// TypeScript declarations for the `fw` API (written to `.fluxwire/types/fw.d.ts`).
pub fn fw_dts() -> &'static str {
    include_str!("fw.d.ts")
}
