//! Interpolation, precedence chain, dynamic variables, secret tracking (SPEC §8).

mod dotenv;
mod dynamic;
mod interp;

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use dotenv::{DotEnvEntry, parse_dotenv, write_dotenv_value};
pub use interp::{Interp, VarRef, VarRefKind, interpolate, refs};

/// Marker substituted for secret values in anything that is logged or copied.
pub const SECRET_MASK: &str = "‹secret›";

/// Where a resolved value came from, e.g. `staging.local.env`, line 4.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Source {
    pub label: String,
    pub line: Option<usize>,
}

impl Source {
    pub fn new(label: impl Into<String>, line: Option<usize>) -> Self {
        Self {
            label: label.into(),
            line,
        }
    }

    pub fn describe(&self) -> String {
        match self.line {
            Some(l) => format!("{}, line {l}", self.label),
            None => self.label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {
    pub value: String,
    pub secret: bool,
    pub source: Source,
}

/// Precedence layers, lowest first (SPEC §8.2). A later `set` wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Global,
    Collection,
    Env,
    LocalEnv,
    Folder,
    Request,
    Cli,
    Runtime,
}

/// An `auth` front matter value: the keyword form (`none`) and/or the field map.
pub type AuthSpec = (Option<String>, Vec<(String, String)>);

/// Settings, headers, auth and path values merged from the collection and
/// folder chain (SPEC §7.8). Innermost already wins; the request is applied on top by the protocol.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inherited {
    pub headers: Vec<(String, String)>,
    /// `auth` front matter of the nearest ancestor that sets one: keyword + fields.
    pub auth: Option<AuthSpec>,
    pub settings: Vec<(String, String)>,
    pub path: Vec<(String, String)>,
}

/// Resolved variable values for one run plus inherited collection/folder context.
#[derive(Debug, Clone, Default)]
pub struct VarScope {
    pub vars: BTreeMap<String, Var>,
    layers: BTreeMap<String, Layer>,
    pub base_dir: Option<PathBuf>,
    pub inherited: Inherited,
    pub allow_process_env: bool,
    /// Name of the active environment, used to key per-environment state.
    pub env_name: Option<String>,
}

impl VarScope {
    pub fn with_base_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: Some(dir.into()),
            ..Self::default()
        }
    }

    /// Set a variable at `layer`; ignored when a higher layer already set it.
    pub fn set(&mut self, layer: Layer, name: impl Into<String>, var: Var) {
        let name = name.into();
        if self.layers.get(&name).is_some_and(|l| *l > layer) {
            return;
        }
        self.layers.insert(name.clone(), layer);
        self.vars.insert(name, var);
    }

    /// Convenience for tests and runtime values: a non-secret literal at `Layer::Runtime`.
    pub fn set_literal(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.set(
            Layer::Runtime,
            name,
            Var {
                value: value.into(),
                secret: false,
                source: Source::new("runtime", None),
            },
        );
    }

    /// Remove a variable from every layer (`fw.vars.unset`).
    pub fn remove(&mut self, name: &str) {
        self.layers.remove(name);
        self.vars.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(|v| v.value.as_str())
    }

    pub fn var(&self, name: &str) -> Option<&Var> {
        self.vars.get(name)
    }

    /// Replace every secret value in `text` with `‹secret:name›`.
    pub fn redact(&self, text: &str) -> String {
        let mut secrets: Vec<(&String, &Var)> = self
            .vars
            .iter()
            .filter(|(_, v)| v.secret && !v.value.is_empty())
            .collect();
        // Longest first so a secret that contains another is replaced whole.
        secrets.sort_by_key(|(_, v)| std::cmp::Reverse(v.value.len()));
        let mut out = text.to_string();
        for (name, v) in secrets {
            out = out.replace(&v.value, &format!("‹secret:{name}›"));
        }
        out
    }

    pub fn secret_values(&self) -> Vec<&str> {
        self.vars
            .values()
            .filter(|v| v.secret && !v.value.is_empty())
            .map(|v| v.value.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(v: &str, secret: bool) -> Var {
        Var {
            value: v.into(),
            secret,
            source: Source::new("t", None),
        }
    }

    #[test]
    fn higher_layers_win_regardless_of_set_order() {
        let cases: &[(&[(Layer, &str)], &str)] = &[
            (&[(Layer::Env, "env"), (Layer::LocalEnv, "local")], "local"),
            (&[(Layer::LocalEnv, "local"), (Layer::Env, "env")], "local"),
            (&[(Layer::Global, "g"), (Layer::Collection, "c")], "c"),
            (&[(Layer::Runtime, "rt"), (Layer::Request, "req")], "rt"),
            (&[(Layer::Folder, "f"), (Layer::Env, "e")], "f"),
            (&[(Layer::Collection, "c"), (Layer::Env, "e")], "e"),
            (&[(Layer::Request, "r"), (Layer::Cli, "cli")], "cli"),
        ];
        for (sets, want) in cases {
            let mut s = VarScope::default();
            for (l, v) in *sets {
                s.set(*l, "x", var(v, false));
            }
            assert_eq!(s.get("x"), Some(*want), "{sets:?}");
        }
    }

    #[test]
    fn redact_replaces_longest_secret_first() {
        let mut s = VarScope::default();
        s.set(Layer::Env, "a", var("abc", true));
        s.set(Layer::Env, "b", var("abcdef", true));
        s.set(Layer::Env, "pub", var("open", false));
        assert_eq!(
            s.redact("x abcdef y abc open"),
            "x ‹secret:b› y ‹secret:a› open"
        );
    }
}
