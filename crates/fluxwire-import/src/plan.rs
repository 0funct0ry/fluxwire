//! Plan, report, source detection and the writer.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Postman,
    Insomnia,
    OpenApi,
    Hurl,
    Http,
}

impl Source {
    pub const ALL: [Source; 5] = [
        Source::Postman,
        Source::Insomnia,
        Source::OpenApi,
        Source::Hurl,
        Source::Http,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Source::Postman => "postman",
            Source::Insomnia => "insomnia",
            Source::OpenApi => "openapi",
            Source::Hurl => "hurl",
            Source::Http => "http",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Postman => "Postman",
            Source::Insomnia => "Insomnia",
            Source::OpenApi => "OpenAPI",
            Source::Hurl => "Hurl",
            Source::Http => ".http file",
        }
    }

    pub fn from_id(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|x| x.id() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedKind {
    Request,
    Folder,
    Collection,
    Env,
    LocalEnv,
    /// `fluxwire.toml`: created when missing; when present only `[secrets]` names are merged.
    Config,
    /// Lines appended when missing.
    Gitignore,
    /// Copied spec or sync snapshot.
    Support,
}

#[derive(Debug, Clone)]
pub struct PlannedFile {
    /// Relative to the target folder.
    pub path: PathBuf,
    pub contents: String,
    pub kind: PlannedKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lossy {
    /// File the item belongs to (relative), or empty for source-wide items.
    pub file: String,
    pub feature: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub source: String,
    pub lossy: Vec<Lossy>,
    /// Files skipped by `apply` because they exist and were not confirmed.
    pub skipped: Vec<PathBuf>,
}

impl ImportReport {
    pub fn lossy(&mut self, file: &str, feature: impl Into<String>, reason: impl Into<String>) {
        let item = Lossy {
            file: file.to_string(),
            feature: feature.into(),
            reason: reason.into(),
        };
        if !self.lossy.contains(&item) {
            self.lossy.push(item);
        }
    }

    /// The `import-report.md` text.
    pub fn to_markdown(&self, files: &[PlannedFile]) -> String {
        let mut s = format!("# Import report\n\nSource: {}\n\n", self.source);
        s.push_str(&format!("## Files ({})\n\n", files.len()));
        for f in files {
            let note = if self.skipped.contains(&f.path) {
                " (skipped, already exists)"
            } else {
                ""
            };
            s.push_str(&format!("- `{}`{note}\n", f.path.display()));
        }
        s.push_str(&format!(
            "\n## Lossy conversions ({})\n\n",
            self.lossy.len()
        ));
        if self.lossy.is_empty() {
            s.push_str("None.\n");
        }
        for l in &self.lossy {
            let at = if l.file.is_empty() {
                String::new()
            } else {
                format!("`{}`: ", l.file)
            };
            s.push_str(&format!("- {at}{}. {}\n", l.feature, l.reason));
        }
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportPlan {
    pub files: Vec<PlannedFile>,
    pub report: ImportReport,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// OpenAPI: copy the spec into the collection and write the `[[openapi]]` entry and base
    /// snapshot (full sync arrives in M12).
    pub spec_link: bool,
    /// Name for the collection when the source has none.
    pub name: Option<String>,
}

#[derive(Debug)]
pub enum ImportError {
    /// The source could not be parsed (CLI exit 65).
    Parse(String),
    Io(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Parse(m) | ImportError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ImportError {}

fn first_json_object(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Guess the source of `path` from its name and content.
pub fn detect(path: &Path, text: &str) -> Option<Source> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".hurl") {
        return Some(Source::Hurl);
    }
    if name.ends_with(".http") || name.ends_with(".rest") || name == "http-client.env.json" {
        return Some(Source::Http);
    }
    if let Some(v) = first_json_object(text) {
        if v.get("openapi").is_some() || v.get("swagger").is_some() {
            return Some(Source::OpenApi);
        }
        if v.get("_type").and_then(|t| t.as_str()) == Some("export")
            || v.get("__export_format").is_some()
        {
            return Some(Source::Insomnia);
        }
        if v.pointer("/info/schema")
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.contains("postman"))
            || v.get("_postman_variable_scope").is_some()
            || (v.get("item").is_some() && v.get("info").is_some())
            || (v.get("values").is_some() && v.get("name").is_some())
        {
            return Some(Source::Postman);
        }
        return None;
    }
    if let Ok(v) = serde_yaml_ng::from_str::<serde_json::Value>(text) {
        if v.get("openapi").is_some() || v.get("swagger").is_some() {
            return Some(Source::OpenApi);
        }
        if v.get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.contains("insomnia"))
        {
            return Some(Source::Insomnia);
        }
    }
    let head = text.trim_start();
    if head.starts_with("GET ")
        || head.starts_with("POST ")
        || head.starts_with("PUT ")
        || head.starts_with("DELETE ")
        || head.starts_with("PATCH ")
    {
        return Some(if text.contains("\nHTTP ") || text.contains("\nHTTP/") {
            Source::Hurl
        } else {
            Source::Http
        });
    }
    None
}

/// Build the plan for `src` (a file, or a directory for Hurl). Writes nothing.
pub fn plan(source: Source, src: &Path, opts: &Options) -> Result<ImportPlan, ImportError> {
    let io = |e: std::io::Error| ImportError::Io(format!("{}: {e}", src.display()));
    let mut p = match source {
        Source::Hurl => crate::hurl::plan(src, opts)?,
        Source::Http => {
            let text = std::fs::read_to_string(src).map_err(io)?;
            crate::httpfile::plan(src, &text, opts)?
        }
        Source::Postman => {
            let text = std::fs::read_to_string(src).map_err(io)?;
            crate::postman::plan(&text, opts)?
        }
        Source::Insomnia => {
            let text = std::fs::read_to_string(src).map_err(io)?;
            crate::insomnia::plan(&text, opts)?
        }
        Source::OpenApi => {
            let text = std::fs::read_to_string(src).map_err(io)?;
            crate::openapi::plan(src, &text, opts)?
        }
    };
    p.report.source = source.label().to_string();
    for f in &mut p.files {
        match f.kind {
            PlannedKind::Request => {
                f.contents = fluxwire_format::format(&f.contents, fluxwire_model::FileKind::Request)
            }
            PlannedKind::Folder => {
                f.contents = fluxwire_format::format(&f.contents, fluxwire_model::FileKind::Folder)
            }
            PlannedKind::Collection => {
                f.contents =
                    fluxwire_format::format(&f.contents, fluxwire_model::FileKind::Collection)
            }
            _ => {}
        }
    }
    p.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(p)
}

/// Write `plan` into `target`. `overwrite(path)` decides for each existing request, folder,
/// collection or environment file; a `false` answer skips the file and records it in the
/// report. Returns the paths written.
pub fn apply(
    plan: &mut ImportPlan,
    target: &Path,
    overwrite: &dyn Fn(&Path) -> bool,
) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let mut skipped = BTreeSet::new();
    for f in &plan.files {
        let dest = target.join(&f.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match f.kind {
            PlannedKind::Gitignore => {
                let old = std::fs::read_to_string(&dest).unwrap_or_default();
                let mut out = old.clone();
                for line in f.contents.lines() {
                    if !old.lines().any(|l| l.trim() == line.trim()) {
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str(line);
                        out.push('\n');
                    }
                }
                if out != old {
                    std::fs::write(&dest, out)?;
                    written.push(f.path.clone());
                }
            }
            PlannedKind::Config if dest.exists() => {
                let old = std::fs::read_to_string(&dest)?;
                if let Some(merged) = crate::secrets::merge_config(&old, &f.contents) {
                    std::fs::write(&dest, merged)?;
                    written.push(f.path.clone());
                }
            }
            _ => {
                if dest.exists() && !overwrite(&f.path) {
                    skipped.insert(f.path.clone());
                    continue;
                }
                std::fs::write(&dest, &f.contents)?;
                written.push(f.path.clone());
            }
        }
    }
    plan.report.skipped = skipped.into_iter().collect();
    Ok(written)
}
