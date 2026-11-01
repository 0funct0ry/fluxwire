//! Assemble the variable scope and inherited context for one request (SPEC §7.8, §8.2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fluxwire_model::{BlockData, BlockKind, FileKind, FluxDoc, FmValue};
use fluxwire_protocol::ProtocolRegistry;
use fluxwire_vars::{Inherited, Layer, Source, Var, VarScope, parse_dotenv};

use crate::collection::load_config;
use crate::environments::{Environment, env_path, is_valid_var_name, local_env_path};
use crate::parse::parse_with;

pub struct ScopeInputs<'a> {
    pub collection_root: &'a Path,
    /// Absolute path of the request file (need not exist on disk).
    pub file: &'a Path,
    /// The request as currently edited; its front matter `vars` etc. win over disk.
    pub doc: &'a FluxDoc,
    pub env: Option<&'a str>,
    /// Directory holding `environments/` (the Workspace root). `None`: the collection root.
    pub env_root: Option<&'a Path>,
    /// The environment as currently edited (unsaved); its rows replace the files' values.
    pub env_overlay: Option<&'a Environment>,
    /// Runtime variables (highest precedence): name → (value, secret).
    pub runtime: &'a BTreeMap<String, (String, bool)>,
    /// `[globals]` from settings.toml.
    pub globals: &'a [(String, String)],
    pub allow_process_env: bool,
}

fn map_of(doc: &FluxDoc, key: &str) -> Vec<(String, String)> {
    match doc.front.get(key).map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn load(path: &Path, kind: FileKind, reg: &ProtocolRegistry) -> Option<FluxDoc> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_with(&text, kind, reg))
}

fn defaults_headers(doc: &FluxDoc) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, _) in doc.blocks_of(BlockKind::Defaults) {
        if let Some(BlockData::Kv(lines)) = doc.data.get(i) {
            out.extend(
                lines
                    .iter()
                    .filter(|l| l.enabled)
                    .map(|l| (l.name.clone(), l.value.clone())),
            );
        }
    }
    out
}

/// Later (inner) entries replace earlier ones with the same key.
fn overlay(base: &mut Vec<(String, String)>, over: Vec<(String, String)>, ci: bool) {
    for (k, v) in over {
        let same = |x: &str| {
            if ci {
                x.eq_ignore_ascii_case(&k)
            } else {
                x == k
            }
        };
        base.retain(|(bk, _)| !same(bk));
        base.push((k, v));
    }
}

fn apply_context(
    doc: &FluxDoc,
    label: &str,
    layer: Layer,
    secret_names: &[String],
    scope: &mut VarScope,
) {
    for (name, value) in map_of(doc, "vars") {
        let secret = secret_names.contains(&name);
        scope.set(
            layer,
            name,
            Var {
                value,
                secret,
                source: Source::new(label, None),
            },
        );
    }
    let inh = &mut scope.inherited;
    overlay(&mut inh.headers, defaults_headers(doc), true);
    overlay(&mut inh.settings, map_of(doc, "settings"), false);
    overlay(&mut inh.path, map_of(doc, "path"), false);
    let fields = map_of(doc, "auth");
    let kw = doc.front.str("auth").map(str::to_string);
    let inherits = fields.is_empty()
        && matches!(
            kw.as_deref().map(str::trim),
            None | Some("") | Some("inherit")
        );
    if !inherits {
        inh.auth = Some((kw, fields));
    }
}

/// `collection.flux.md` then each `folder.flux.md` down to `file`'s directory (outermost first),
/// with a collection-relative label for each. Missing files are skipped.
pub fn ancestor_docs(collection_root: &Path, file: &Path) -> Vec<(String, FluxDoc)> {
    let reg = ProtocolRegistry::builtin();
    let mut out = Vec::new();
    if let Some(c) = load(
        &collection_root.join("collection.flux.md"),
        FileKind::Collection,
        &reg,
    ) {
        out.push(("collection.flux.md".to_string(), c));
    }
    if let Ok(rel) = file.parent().unwrap_or(file).strip_prefix(collection_root) {
        let mut dir = collection_root.to_path_buf();
        for comp in rel.components() {
            dir.push(comp);
            if let Some(f) = load(&dir.join("folder.flux.md"), FileKind::Folder, &reg) {
                let label = dir
                    .join("folder.flux.md")
                    .strip_prefix(collection_root)
                    .unwrap_or(&dir)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((label, f));
            }
        }
    }
    out
}

pub fn build_scope(i: &ScopeInputs) -> VarScope {
    let reg = ProtocolRegistry::builtin();
    let secret_names = load_config(i.collection_root)
        .map(|c| c.secrets.names)
        .unwrap_or_default();
    let env_dir = i.env_root.unwrap_or(i.collection_root);
    let mut scope = VarScope::default();
    scope.base_dir = i.file.parent().map(Path::to_path_buf);
    scope.inherited = Inherited::default();
    scope.allow_process_env = i.allow_process_env;
    scope.env_name = i.env.map(str::to_string);

    for (k, v) in i.globals {
        scope.set(
            Layer::Global,
            k.clone(),
            Var {
                value: v.clone(),
                secret: secret_names.contains(k),
                source: Source::new("settings.toml [globals]", None),
            },
        );
    }

    if let Some(c) = load(
        &i.collection_root.join("collection.flux.md"),
        FileKind::Collection,
        &reg,
    ) {
        apply_context(
            &c,
            "collection.flux.md",
            Layer::Collection,
            &secret_names,
            &mut scope,
        );
    }

    if let (Some(env), Some(ov)) = (
        i.env,
        i.env_overlay.filter(|o| Some(o.name.as_str()) == i.env),
    ) {
        for local in [false, true] {
            for r in ov.rows.iter().filter(|r| r.local == local) {
                let name = r.name.trim();
                if !is_valid_var_name(name) {
                    continue;
                }
                let file = if local {
                    format!("{env}.local.env")
                } else {
                    format!("{env}.env")
                };
                scope.set(
                    if local { Layer::LocalEnv } else { Layer::Env },
                    name.to_string(),
                    Var {
                        secret: local || r.secret || secret_names.iter().any(|n| n == name),
                        value: r.value.clone(),
                        source: Source::new(file, (r.line > 0).then_some(r.line)),
                    },
                );
            }
        }
    } else if let Some(env) = i.env {
        for (path, layer, label_local) in [
            (env_path(env_dir, env), Layer::Env, false),
            (local_env_path(env_dir, env), Layer::LocalEnv, true),
        ] {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for e in parse_dotenv(&text) {
                scope.set(
                    layer,
                    e.name.clone(),
                    Var {
                        secret: label_local || secret_names.contains(&e.name),
                        value: e.value,
                        source: Source::new(label.clone(), Some(e.line)),
                    },
                );
            }
        }
    }

    // Folders, outermost first, so inner values win.
    if let Ok(rel) = i
        .file
        .parent()
        .unwrap_or(i.file)
        .strip_prefix(i.collection_root)
    {
        let mut dir: PathBuf = i.collection_root.to_path_buf();
        for comp in rel.components() {
            dir.push(comp);
            if let Some(f) = load(&dir.join("folder.flux.md"), FileKind::Folder, &reg) {
                let label = format!(
                    "{}/folder.flux.md",
                    dir.strip_prefix(i.collection_root)
                        .unwrap_or(&dir)
                        .to_string_lossy()
                );
                apply_context(&f, &label, Layer::Folder, &secret_names, &mut scope);
            }
        }
    }

    // The request itself: only `vars` participates in the variable chain; headers, auth,
    // settings and path are applied by the protocol on top of the inherited context.
    let label = i
        .file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (name, value) in map_of(i.doc, "vars") {
        let secret = secret_names.contains(&name);
        scope.set(
            Layer::Request,
            name,
            Var {
                value,
                secret,
                source: Source::new(label.clone(), None),
            },
        );
    }

    for (name, (value, secret)) in i.runtime {
        scope.set(
            Layer::Runtime,
            name.clone(),
            Var {
                value: value.clone(),
                secret: *secret || secret_names.contains(name),
                source: Source::new("runtime", None),
            },
        );
    }
    scope
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw-scope-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("environments")).unwrap();
        std::fs::create_dir_all(d.join("pay")).unwrap();
        std::fs::write(
            d.join("fluxwire.toml"),
            "[collection]\nname = \"c\"\nformat = 1\n[secrets]\nnames = [\"apiKey\"]\n",
        )
        .unwrap();
        d
    }

    fn scope(d: &Path, env: Option<&str>, src: &str) -> VarScope {
        let doc = parse(src);
        let file = d.join("pay/r.flux.md");
        build_scope(&ScopeInputs {
            collection_root: d,
            file: &file,
            doc: &doc,
            env,
            env_root: None,
            env_overlay: None,
            runtime: &BTreeMap::new(),
            globals: &[("g".into(), "glob".into())],
            allow_process_env: false,
        })
    }

    #[test]
    fn precedence_provenance_and_secrets() {
        let d = tree("prec");
        std::fs::write(
            d.join("environments/staging.env"),
            "a=env\nb=env\napiKey=k\n",
        )
        .unwrap();
        std::fs::write(
            d.join("environments/staging.local.env"),
            "\nb=local\ntok=t\n",
        )
        .unwrap();
        std::fs::write(
            d.join("collection.flux.md"),
            "---\nvars:\n  c: coll\n  a: coll\n---\n# C\n",
        )
        .unwrap();
        std::fs::write(
            d.join("pay/folder.flux.md"),
            "---\nvars:\n  f: fold\n  b: fold\n---\n# F\n",
        )
        .unwrap();
        let s = scope(&d, Some("staging"), "---\nvars:\n  r: req\n---\n# R\n");
        assert_eq!(s.get("a"), Some("env")); // env > collection
        assert_eq!(s.get("b"), Some("fold")); // folder > local env
        assert_eq!(s.get("c"), Some("coll"));
        assert_eq!(s.get("g"), Some("glob"));
        assert_eq!(s.get("r"), Some("req"));
        assert!(s.var("tok").unwrap().secret);
        assert!(s.var("apiKey").unwrap().secret);
        assert!(!s.var("a").unwrap().secret);
        assert_eq!(s.var("a").unwrap().source.describe(), "staging.env, line 1");
        assert_eq!(s.env_name.as_deref(), Some("staging"));
    }

    #[test]
    fn switching_environment_changes_values() {
        let d = tree("switch");
        std::fs::write(d.join("environments/local.env"), "baseUrl=http://l\n").unwrap();
        std::fs::write(d.join("environments/staging.env"), "baseUrl=https://s\n").unwrap();
        assert_eq!(
            scope(&d, Some("local"), "# R\n").get("baseUrl"),
            Some("http://l")
        );
        assert_eq!(
            scope(&d, Some("staging"), "# R\n").get("baseUrl"),
            Some("https://s")
        );
        assert_eq!(scope(&d, None, "# R\n").get("baseUrl"), None);
    }

    #[test]
    fn inherits_headers_auth_settings_with_inner_winning() {
        let d = tree("inh");
        std::fs::write(
            d.join("collection.flux.md"),
            "---\nauth:\n  type: bearer\n  token: c\nsettings:\n  timeout_ms: 1\n---\n# C\n\n```http defaults\nAccept: a/c\nX-A: 1\n```\n",
        )
        .unwrap();
        std::fs::write(
            d.join("pay/folder.flux.md"),
            "---\nauth:\n  type: bearer\n  token: f\n---\n# F\n\n```http defaults\naccept: a/f\n```\n",
        )
        .unwrap();
        let s = scope(&d, None, "# R\n");
        let h: BTreeMap<_, _> = s.inherited.headers.iter().cloned().collect();
        assert_eq!(h.get("accept").map(String::as_str), Some("a/f"));
        assert_eq!(h.get("X-A").map(String::as_str), Some("1"));
        assert!(!h.contains_key("Accept"));
        assert_eq!(
            s.inherited.auth.as_ref().unwrap().1,
            vec![
                ("type".into(), "bearer".into()),
                ("token".into(), "f".into())
            ]
        );
        assert_eq!(
            s.inherited.settings,
            vec![("timeout_ms".into(), "1".into())]
        );
    }

    #[test]
    fn environment_files_come_from_env_root_when_given() {
        let d = tree("envroot");
        let ws = d.join("ws");
        std::fs::create_dir_all(ws.join("environments")).unwrap();
        std::fs::write(ws.join("environments/shared.env"), "a=from-workspace\n").unwrap();
        let file = d.join("req.flux.md");
        let doc = parse_with("", FileKind::Request, &ProtocolRegistry::builtin());
        let mk = |env_root: Option<&Path>| {
            build_scope(&ScopeInputs {
                collection_root: &d,
                file: &file,
                doc: &doc,
                env: Some("shared"),
                env_root,
                env_overlay: None,
                runtime: &BTreeMap::new(),
                globals: &[],
                allow_process_env: false,
            })
        };
        assert_eq!(
            mk(Some(&ws)).var("a").map(|v| v.value.clone()).as_deref(),
            Some("from-workspace")
        );
        assert!(mk(None).var("a").is_none());
    }
}
