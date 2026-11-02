//! Finding `.flux.md` files and the up-front parse / `requires` checks.

use crate::exit;
use fluxwire_format::{file_kind_for, parse_with};
use fluxwire_model::Severity;
use fluxwire_protocol::ProtocolRegistry;
use std::path::{Path, PathBuf};

pub fn is_flux(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".flux.md"))
}

/// Every `*.flux.md` under `path` (or `path` itself), sorted; hidden directories are skipped.
pub fn collect(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if path.is_file() {
        if is_flux(path) {
            out.push(path.to_path_buf());
        }
        return out;
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            if p.is_dir() && !hidden {
                stack.push(p);
            } else if is_flux(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Parses each file; prints diagnostics to stderr. Returns 65 on errors, 66 for unsupported
/// features, else 0.
pub fn check(files: &[PathBuf], show_warnings: bool) -> u8 {
    let reg = ProtocolRegistry::builtin();
    let mut code = 0;
    for f in files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                code = exit::merge(code, exit::PARSE);
                continue;
            }
        };
        let doc = parse_with(&src, file_kind_for(f), &reg);
        for d in &doc.diagnostics {
            let level = match d.severity {
                Severity::Error => "error",
                Severity::Warning if show_warnings => "warning",
                Severity::Info if show_warnings => "info",
                _ => continue,
            };
            eprintln!("{}: {level} {}: {}", f.display(), d.code, d.message);
            if d.severity == Severity::Error && d.code != "F401" {
                code = exit::merge(code, exit::PARSE);
            }
        }
        if !doc.unsupported.is_empty() {
            eprintln!(
                "{}: error F401: requires unsupported features: {}",
                f.display(),
                doc.unsupported.join(", ")
            );
            code = exit::merge(code, exit::UNSUPPORTED);
        }
    }
    code
}
