//! GPUI-free assembly of everything a send or folder run needs besides the request itself:
//! scope, net and exec settings, script limits and trust (ADR-009). The UI, MCP and the CLI
//! all call this so `build_scope` stays the single source of precedence.

use crate::{
    DEFAULT_CONCURRENCY, Limits, RunMode, RunPlan, ScriptOptions, SendOptions, net_config_from_dir,
    settings_from_defaults,
};
use fluxwire_format::{UserSettings, ancestor_docs, build_scope, is_trusted, load_config};
use fluxwire_model::{BlockKind, CollectionConfig, FluxDoc};
use fluxwire_protocol::EnvState;
use fluxwire_vars::VarScope;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Script limits from a Collection's `[scripts]` table over the built-in defaults.
pub fn limits_from(cfg: &CollectionConfig) -> Limits {
    let mut l = Limits::default();
    if let Some(ms) = cfg.scripts.timeout_ms {
        l.timeout_ms = ms;
    }
    if let Some(mb) = cfg.scripts.memory_mb {
        l.memory_mb = mb as usize;
    }
    l
}

fn is_script(doc: &FluxDoc) -> bool {
    doc.blocks
        .iter()
        .any(|b| matches!(b.kind, BlockKind::Pre | BlockKind::Post | BlockKind::Test))
}

/// Whether `doc` or any `collection.flux.md`/`folder.flux.md` above it carries a script or test.
pub fn has_scripts(doc: &FluxDoc, root: Option<&Path>, abs: &Path) -> bool {
    is_script(doc) || root.is_some_and(|r| ancestor_docs(r, abs).iter().any(|(_, d)| is_script(d)))
}

/// The nearest ancestor of `path` (itself included) that holds a `fluxwire.toml`.
pub fn collection_root_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|d| d.join("fluxwire.toml").is_file())
        .map(Path::to_path_buf)
}

/// Options for a folder run beyond the Collection defaults.
#[derive(Debug, Clone, Default)]
pub struct FolderOpts {
    pub sequential: bool,
    pub concurrency: Option<u32>,
    pub iterations: Option<u32>,
    pub data: Vec<BTreeMap<String, String>>,
    pub filter: Option<String>,
    pub only: Option<Vec<String>>,
    pub bail: bool,
    pub delay_ms: u64,
    pub timeout_ms: Option<u64>,
}

/// Process-wide inputs: user settings, the config dir (for `trust.toml`), the chosen
/// environment and runtime variables. Stateless: no cookie persistence, no unsaved overlay.
#[derive(Debug, Clone, Default)]
pub struct Prep {
    pub config_dir: Option<PathBuf>,
    pub settings: UserSettings,
    pub env: Option<String>,
    /// Environment directory (the Workspace root). `None`: the Collection root.
    pub env_root: Option<PathBuf>,
    pub runtime: BTreeMap<String, (String, bool)>,
}

impl Prep {
    pub fn new(
        config_dir: Option<PathBuf>,
        env: Option<String>,
        env_root: Option<PathBuf>,
    ) -> Self {
        let settings = config_dir
            .as_deref()
            .map(fluxwire_format::load_settings)
            .unwrap_or_default();
        Prep {
            config_dir,
            settings,
            env,
            env_root,
            runtime: BTreeMap::new(),
        }
    }

    /// Reads `trust.toml` only; never writes it.
    pub fn trusted(&self, root: &Path) -> bool {
        self.config_dir
            .as_deref()
            .is_some_and(|d| is_trusted(d, root))
    }

    pub fn globals(&self) -> Vec<(String, String)> {
        self.settings
            .globals
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn scope(&self, root: &Path, abs: &Path, doc: &FluxDoc) -> VarScope {
        build_scope(&fluxwire_format::ScopeInputs {
            collection_root: root,
            file: abs,
            doc,
            env: self.env.as_deref(),
            env_root: self.env_root.as_deref(),
            env_overlay: None,
            runtime: &self.runtime,
            globals: &self.globals(),
            allow_process_env: self.settings.allow_process_env,
        })
    }

    pub fn script_options(
        &self,
        root: &Path,
        file: Option<PathBuf>,
        cfg: &CollectionConfig,
    ) -> ScriptOptions {
        ScriptOptions {
            trusted: self.trusted(root),
            limits: limits_from(cfg),
            collection_root: Some(root.to_path_buf()),
            file,
            globals: self.globals(),
            allow_process_env: self.settings.allow_process_env,
            env_overlay: None,
            env_root: self.env_root.clone(),
        }
    }

    pub fn send_options(&self, doc: &FluxDoc, abs: &Path, root: &Path) -> SendOptions {
        let cfg = load_config(root).unwrap_or_default();
        SendOptions {
            scope: self.scope(root, abs, doc),
            settings: settings_from_defaults(&cfg.defaults),
            net: net_config_from_dir(root),
            env: self.env.as_deref().map(EnvState::new),
            scripts: self.script_options(root, Some(abs.to_path_buf()), &cfg),
        }
    }

    pub fn run_plan(&self, root: &Path, folder: &str, o: FolderOpts) -> RunPlan {
        let cfg = load_config(root).unwrap_or_default();
        RunPlan {
            collection_root: root.to_path_buf(),
            folder: folder.to_string(),
            mode: if o.sequential {
                RunMode::Sequential
            } else {
                RunMode::Concurrent
            },
            concurrency: Some(
                o.concurrency
                    .filter(|c| *c > 0)
                    .or(cfg.defaults.concurrency)
                    .unwrap_or(DEFAULT_CONCURRENCY),
            ),
            iterations: o.iterations.unwrap_or(1).max(1),
            data: o.data,
            filter: o.filter.filter(|s| !s.trim().is_empty()),
            only: o.only,
            bail: o.bail,
            delay_ms: o.delay_ms,
            timeout_ms: o.timeout_ms,
            env: self.env.clone(),
            env_state: self.env.as_deref().map(EnvState::new),
            settings: settings_from_defaults(&cfg.defaults),
            net: net_config_from_dir(root),
            scripts: self.script_options(root, None, &cfg),
            runtime: self.runtime.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_override_defaults() {
        let mut c = CollectionConfig::default();
        assert_eq!(limits_from(&c).timeout_ms, Limits::default().timeout_ms);
        c.scripts.timeout_ms = Some(7);
        c.scripts.memory_mb = Some(3);
        let l = limits_from(&c);
        assert_eq!((l.timeout_ms, l.memory_mb), (7, 3));
    }

    #[test]
    fn finds_collection_root_and_detects_scripts() {
        let d = std::env::temp_dir().join(format!("fw-prep-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        std::fs::write(d.join("fluxwire.toml"), "[collection]\nformat = 1\n").unwrap();
        assert_eq!(collection_root_of(&d.join("a/b")).unwrap(), d);
        assert!(collection_root_of(Path::new("/")).is_none());
        let plain = fluxwire_format::parse("# x\n\n```http\nGET /\n```\n");
        assert!(!has_scripts(&plain, Some(&d), &d.join("a/x.flux.md")));
        std::fs::write(
            d.join("a/folder.flux.md"),
            "# f\n\n```ts pre\nfw.log(1)\n```\n",
        )
        .unwrap();
        assert!(has_scripts(&plain, Some(&d), &d.join("a/x.flux.md")));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn untrusted_without_config_dir() {
        let p = Prep::default();
        assert!(!p.trusted(Path::new("/x")));
        let plan = p.run_plan(Path::new("/nonexistent"), "", FolderOpts::default());
        assert!(!plan.scripts.trusted);
        assert_eq!(plan.iterations, 1);
    }
}
