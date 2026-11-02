//! Write-class engine tools. Request files change only through the span editor and the
//! canonical formatter (SPEC §7.10).

use super::{Engine, Out, args, checked, existing, object, unified_diff};
use crate::ToolClass;
use crate::registry::Registry;
use crate::tools::def_in;
use fluxwire_format::{file_kind_for, format, is_canonical};
use fluxwire_model::{Edit, Span};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct SpanEdit {
    /// Byte offset where the replaced span starts.
    start: usize,
    /// Byte offset where it ends (exclusive).
    end: usize,
    /// Replacement text.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteIn {
    /// A `.flux.md` file inside an allowed root.
    path: String,
    /// Span edits applied to the existing file (byte offsets from `read_request`). Everything
    /// outside the spans is preserved byte for byte.
    edits: Option<Vec<SpanEdit>>,
    /// Whole file text, for a new file or a full rewrite; it is put through the canonical formatter.
    content: Option<String>,
    /// Report the diff without writing.
    dry_run: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct FmtIn {
    /// A `.flux.md` file, or a directory formatted recursively.
    path: String,
    dry_run: Option<bool>,
}

fn is_flux(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".flux.md"))
}

fn flux_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                flux_files(&p, out);
            }
        } else if is_flux(&p) {
            out.push(p);
        }
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("md.fluxwire-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn register(r: &mut Registry, _e: Arc<Engine>) {
    r.register(def_in::<WriteIn, Out>(
        ToolClass::Write,
        "write_request",
        "Edit a .flux.md file with span edits, or write whole content through the canonical formatter. Returns the diff and the new diagnostics; `dry_run` writes nothing.",
        |ctx, a| {
            let a: WriteIn = args(a)?;
            let abs = checked(ctx, &a.path)?;
            if !is_flux(&abs) {
                return Err("only .flux.md files can be written".into());
            }
            let old = std::fs::read_to_string(&abs).ok();
            let kind = file_kind_for(&abs);
            let new = match (&a.edits, &a.content) {
                (Some(_), Some(_)) => return Err("pass either `edits` or `content`, not both".into()),
                (None, None) => return Err("pass `edits` or `content`".into()),
                (Some(edits), None) => {
                    let old = old.as_deref().ok_or("cannot edit a file that does not exist")?;
                    let edits: Vec<Edit> = edits
                        .iter()
                        .map(|e| Edit {
                            span: Span { start: e.start, end: e.end },
                            text: e.text.clone(),
                        })
                        .collect();
                    fluxwire_model::apply_edits(old, &edits)
                        .ok_or("edits overlap or fall outside the file")?
                }
                (None, Some(c)) => format(c, kind),
            };
            let doc = fluxwire_format::parse(&new);
            let changed = old.as_deref() != Some(new.as_str());
            let dry = a.dry_run.unwrap_or(false);
            if changed && !dry {
                write_atomic(&abs, &new)?;
            }
            object(json!({
                "path": abs.display().to_string(),
                "created": old.is_none(),
                "changed": changed,
                "written": changed && !dry,
                "diff": unified_diff(&a.path, old.as_deref().unwrap_or(""), &new),
                "diagnostics": super::read::diagnostics_json(&doc),
            }))
        },
    ));

    r.register(def_in::<FmtIn, Out>(
        ToolClass::Write,
        "fmt",
        "Format .flux.md files with the canonical formatter (the same bytes the app and `fluxwire fmt` produce). Returns per-file diffs; `dry_run` writes nothing.",
        |ctx, a| {
            let a: FmtIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let dry = a.dry_run.unwrap_or(false);
            let mut files = Vec::new();
            if abs.is_dir() {
                flux_files(&abs, &mut files);
            } else if is_flux(&abs) {
                files.push(abs.clone());
            } else {
                return Err("only .flux.md files can be formatted".into());
            }
            let mut out = Vec::new();
            for f in files {
                let old = std::fs::read_to_string(&f).map_err(|e| e.to_string())?;
                let kind = file_kind_for(&f);
                let new = format(&old, kind);
                let changed = new != old;
                debug_assert!(is_canonical(&new, kind));
                if changed && !dry {
                    write_atomic(&f, &new)?;
                }
                out.push(json!({
                    "path": f.display().to_string(),
                    "changed": changed,
                    "written": changed && !dry,
                    "diff": unified_diff(&f.display().to_string(), &old, &new),
                }));
            }
            object(json!({ "files": out }))
        },
    ));
}
