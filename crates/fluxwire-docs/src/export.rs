//! `fluxwire docs export`: a single-page static HTML reference of a collection, folder or file.

use crate::html::{self, FileCtx, Rewrite};
use crate::links::{self, Link};
use crate::{base_of, generated, headings, model};
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ExportError {
    /// `path` does not exist or holds no docs.
    NotFound(String),
    Io(io::Error),
}

impl From<io::Error> for ExportError {
    fn from(e: io::Error) -> Self {
        ExportError::Io(e)
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::NotFound(p) => write!(f, "{p}: no documentation files found"),
            ExportError::Io(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Exported {
    pub index: PathBuf,
    pub files: usize,
    pub images: usize,
}

fn is_doc_file(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    n.ends_with(".flux.md") || n.eq_ignore_ascii_case("readme.md")
}

/// Docs files below `dir`: `collection.flux.md`, then `README.md` and `folder.flux.md`
/// before the requests of each folder, folders in name order.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    let rank = |p: &Path| match p.file_name().and_then(|n| n.to_str()).unwrap_or("") {
        "collection.flux.md" => 0,
        n if n.eq_ignore_ascii_case("readme.md") => 1,
        "folder.flux.md" => 2,
        _ => 3,
    };
    let mut files: Vec<_> = entries
        .iter()
        .filter(|p| p.is_file() && is_doc_file(p))
        .cloned()
        .collect();
    files.sort_by_key(|p| (rank(p), p.clone()));
    out.extend(files);
    for d in entries.iter().filter(|p| p.is_dir()) {
        let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || matches!(name, "environments" | "node_modules" | "target") {
            continue;
        }
        collect(d, out)?;
    }
    Ok(())
}

fn slug(rel: &str) -> String {
    rel.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Renders the docs of `path` (a collection or folder directory, or one file) to
/// `out/index.html`, copying local images to `out/assets/`.
pub fn export(path: &Path, out: &Path) -> Result<Exported, ExportError> {
    let (root, files) = if path.is_dir() {
        let mut f = Vec::new();
        collect(path, &mut f)?;
        (path.to_path_buf(), f)
    } else if path.is_file() {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            vec![path.to_path_buf()],
        )
    } else {
        return Err(ExportError::NotFound(path.display().to_string()));
    };
    if files.is_empty() {
        return Err(ExportError::NotFound(path.display().to_string()));
    }
    fs::create_dir_all(out)?;

    let images: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let mut nav = Vec::new();
    let mut body = String::new();
    let mut title = root
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "API reference".into());

    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        let src = fs::read_to_string(f)?;
        let id = slug(&rel);
        let flux = rel.ends_with(".flux.md");
        let doc = flux.then(|| fluxwire_format::parse(&src));
        let (base, md) = match &doc {
            Some(d) => (base_of(d), &src[base_of(d)..]),
            None => (0, src.as_str()),
        };
        let heads = headings::of(md);
        let label = doc
            .as_ref()
            .and_then(|d| d.name().map(str::to_string))
            .or_else(|| heads.first().map(|h| h.text.clone()))
            .unwrap_or_else(|| rel.clone());
        if rel == "collection.flux.md" {
            title = label.clone();
        }
        let link = |h: &str| match links::resolve(&rel, h) {
            Link::Doc { path, fragment } => match fragment {
                Some(fr) => format!("#{}-{}", slug(&path), fr),
                None => format!("#{}", slug(&path)),
            },
            Link::Anchor(a) => format!("#{id}-{a}"),
            Link::External(u) => u,
            Link::Other(o) => o,
        };
        let image = |h: &str| match links::resolve(&rel, h) {
            Link::External(u) => u,
            _ => match links::normalise(&rel, h) {
                Some(p) if root.join(&p).is_file() => {
                    images.borrow_mut().push(p.clone());
                    format!("assets/{p}")
                }
                _ => h.to_string(),
            },
        };
        let rw = Rewrite {
            link: &link,
            image: &image,
        };
        let prefix = format!("{id}-");
        let blocks = model::parse(md);
        body.push_str(&format!(
            "<section class=\"file\" id=\"{}\">\n<p class=\"chip\">{}</p>\n",
            html::esc(&id),
            html::esc(&rel)
        ));
        body.push_str(&html::render_blocks(
            &blocks,
            &FileCtx {
                doc: doc.as_ref(),
                base,
            },
            &rw,
            &prefix,
        ));
        if let Some(d) = &doc {
            body.push_str(&html::render_sections(&generated::generate(d)));
        }
        body.push_str("</section>\n");
        nav.push((id, label));
    }

    let mut imgs = images.into_inner();
    imgs.sort();
    imgs.dedup();
    for p in &imgs {
        let dest = out.join("assets").join(p);
        if let Some(d) = dest.parent() {
            fs::create_dir_all(d)?;
        }
        fs::copy(root.join(p), dest)?;
    }
    let index = out.join("index.html");
    fs::write(&index, html::page(&title, &nav, &body))?;
    Ok(Exported {
        index,
        files: files.len(),
        images: imgs.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn exports_a_collection_to_one_page() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(
            r,
            "collection.flux.md",
            "---\nkind: collection\nname: Shop\n---\n\n# Shop\n\nSee [create](pay/create.flux.md) ![logo](img/l.png).\n",
        );
        write(
            r,
            "pay/create.flux.md",
            "---\nname: Create\n---\n\n# Create\n\n```http\nPOST {{u}}/p\nX-A: 1\n```\n",
        );
        write(r, "img/l.png", "png");
        write(r, "environments/x.env", "A=1");
        let out = d.path().join("site");
        let e = export(r, &out).unwrap();
        assert_eq!((e.files, e.images), (2, 1));
        let h = fs::read_to_string(&e.index).unwrap();
        assert!(h.contains("<title>Shop</title>"));
        assert!(h.contains("href=\"#pay-create-flux-md\""));
        assert!(h.contains("src=\"assets/img/l.png\""));
        assert!(h.contains("POST {{u}}/p"));
        assert!(h.contains("<h4>Headers</h4>"));
        assert!(!h.contains("<script"));
        assert!(out.join("assets/img/l.png").is_file());
    }

    #[test]
    fn missing_path_and_empty_dir_are_not_found() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(
            export(&d.path().join("no"), d.path()),
            Err(ExportError::NotFound(_))
        ));
        assert!(matches!(
            export(d.path(), &d.path().join("o")),
            Err(ExportError::NotFound(_))
        ));
    }
}
