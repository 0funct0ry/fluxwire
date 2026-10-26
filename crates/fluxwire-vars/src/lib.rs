//! Interpolation, precedence chain, dynamic variables, secret tracking.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Resolved variable values for one run. M3 only carries literals and the
/// directory that relative file references (`< path`, `@file()`) resolve against;
/// the precedence chain and interpolation arrive in M4.
#[derive(Debug, Clone, Default)]
pub struct VarScope {
    pub vars: BTreeMap<String, String>,
    pub base_dir: Option<PathBuf>,
}

impl VarScope {
    pub fn with_base_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            vars: BTreeMap::new(),
            base_dir: Some(dir.into()),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}
