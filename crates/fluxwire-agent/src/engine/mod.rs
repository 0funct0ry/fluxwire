//! Engine tools (M9.2): read, write and execute tools over the Workspace, built on the same
//! `fluxwire-format` / `fluxwire-runner` calls as the app (ADR-009). No GPUI.
//!
//! Handlers are synchronous; the server runs them on the blocking pool, so they may wait on
//! runner events.

mod execute;
mod read;
mod record;
mod write;

use crate::registry::{Registry, ToolCtx};
use rmcp::model::JsonObject;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Output of every engine tool: a JSON object (the schema is the generic object schema).
pub type Out = Map<String, Value>;

/// Process-wide inputs of the engine tools.
pub struct Engine {
    /// User config dir: `settings.toml` and `trust.toml` (read only).
    pub config_dir: Option<PathBuf>,
    /// User data dir holding `history.sqlite`; `None` disables history.
    pub data_dir: Option<PathBuf>,
    live: Mutex<HashMap<String, Box<dyn Fn() + Send + Sync>>>,
    store: Mutex<Option<fluxwire_store::Store>>,
}

impl Engine {
    pub fn new(config_dir: Option<PathBuf>, data_dir: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Engine {
            config_dir,
            data_dir,
            live: Mutex::default(),
            store: Mutex::default(),
        })
    }

    /// The user's real config and data directories.
    pub fn from_environment() -> Arc<Self> {
        let config = directories::ProjectDirs::from("dev", "fluxwire", "fluxwire")
            .map(|d| d.config_dir().to_path_buf());
        Self::new(config, fluxwire_store::data_dir())
    }

    pub(crate) fn store(&self) -> Option<fluxwire_store::Store> {
        let mut g = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            let dir = self.data_dir.as_ref()?;
            *g = fluxwire_store::Store::open(&dir.join("history.sqlite")).ok();
        }
        g.clone()
    }

    pub(crate) fn track(&self, key: String, cancel: impl Fn() + Send + Sync + 'static) {
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, Box::new(cancel));
    }

    pub(crate) fn untrack(&self, key: &str) {
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }

    /// Cancels a live send or run; false when the key is unknown.
    pub(crate) fn cancel(&self, key: &str) -> bool {
        match self.live.lock().unwrap_or_else(|p| p.into_inner()).get(key) {
            Some(f) => {
                f();
                true
            }
            None => false,
        }
    }
}

pub fn register(r: &mut Registry, e: Arc<Engine>) {
    read::register(r, e.clone());
    write::register(r, e.clone());
    execute::register(r, e);
}

pub(crate) fn args<T: DeserializeOwned>(a: Option<JsonObject>) -> Result<T, String> {
    serde_json::from_value(Value::Object(a.unwrap_or_default()))
        .map_err(|e| format!("invalid arguments: {e}"))
}

pub(crate) fn object(v: Value) -> Result<Value, String> {
    debug_assert!(v.is_object());
    Ok(v)
}

/// A path argument checked against the allow-list.
pub(crate) fn checked(ctx: &ToolCtx, p: &str) -> Result<PathBuf, String> {
    ctx.roots.check(Path::new(p))
}

/// Like [`checked`], and the path must exist.
pub(crate) fn existing(ctx: &ToolCtx, p: &str) -> Result<PathBuf, String> {
    let abs = checked(ctx, p)?;
    if abs.exists() {
        Ok(abs)
    } else {
        Err(format!("{p} does not exist"))
    }
}

/// The allowed root that contains `abs` (the Workspace root for environments and history).
pub(crate) fn workspace_of(ctx: &ToolCtx, abs: &Path) -> Option<PathBuf> {
    ctx.roots
        .list()
        .iter()
        .filter(|r| abs.starts_with(r))
        .max_by_key(|r| r.components().count())
        .cloned()
}

/// Replaces secret values in every string and masks sensitive header values.
pub(crate) fn scrub(secrets: &[(String, String)], v: &mut Value) {
    match v {
        Value::String(s) => *s = fluxwire_runner::redact_with(secrets, s),
        Value::Array(a) => a.iter_mut().for_each(|x| scrub(secrets, x)),
        Value::Object(m) => m.values_mut().for_each(|x| scrub(secrets, x)),
        _ => {}
    }
}

pub(crate) const SENSITIVE_HEADERS: [&str; 6] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
];

pub(crate) fn mask_header(name: &str, value: &str) -> String {
    if SENSITIVE_HEADERS
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
    {
        "‹redacted›".to_string()
    } else {
        value.to_string()
    }
}

/// Unified diff of two texts (empty when equal).
pub(crate) fn unified_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}
