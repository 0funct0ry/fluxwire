//! File operations behind the workspace tree: create, rename, duplicate, move, delete (to trash).

use crate::collection::{file_kind_for, is_request_file};
use crate::edit::{apply, set_front_matter};
use crate::parse::parse_with;
use crate::template::*;
use fluxwire_model::{FileKind, FmValue};
use fluxwire_protocol::ProtocolRegistry;
use std::io;
use std::path::{Path, PathBuf};

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

fn unique(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    p
}

pub fn create_collection(dir: &Path, name: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir.join("environments"))?;
    write_new(&dir.join("fluxwire.toml"), &new_collection_toml(name))?;
    write_new(&dir.join("collection.flux.md"), &new_collection_file(name))?;
    write_new(&dir.join(".gitignore"), GITIGNORE)?;
    Ok(())
}

fn write_new(path: &Path, text: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, text)
}

/// Default URL of a new request: `{{baseUrl}}/` (`wss://{{host}}/` for WebSocket).
pub fn default_request_url(websocket: bool) -> &'static str {
    if websocket {
        "wss://{{host}}/"
    } else {
        "{{baseUrl}}/"
    }
}

pub fn create_request(
    dir: &Path,
    name: &str,
    description: &str,
    websocket: bool,
) -> io::Result<PathBuf> {
    create_request_with_url(
        dir,
        name,
        description,
        websocket,
        default_request_url(websocket),
    )
}

/// Like `create_request`, with the URL written on the request line (may be empty).
pub fn create_request_with_url(
    dir: &Path,
    name: &str,
    description: &str,
    websocket: bool,
    url: &str,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let p = unique(dir, &slug(name), ".flux.md");
    let text = if websocket {
        new_websocket(name, description, url)
    } else {
        new_request(name, description, "GET", url)
    };
    std::fs::write(&p, text)?;
    Ok(p)
}

/// Rename a collection: its `name` in `fluxwire.toml` and the front matter `name` of
/// `collection.flux.md`. The folder keeps its name, so keys and tabs stay valid. Only the
/// `name` line is replaced; the rest of the file is untouched.
pub fn rename_collection(dir: &Path, new_name: &str) -> io::Result<()> {
    let toml_path = dir.join("fluxwire.toml");
    let text = std::fs::read_to_string(&toml_path)?;
    let line = format!("name = {}", crate::template::toml_string(new_name));
    let mut out = String::with_capacity(text.len() + line.len());
    let (mut in_collection, mut done) = (false, false);
    for l in text.split_inclusive('\n') {
        let t = l.trim();
        if t.starts_with('[') {
            if in_collection && !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            in_collection = t == "[collection]";
        }
        let is_name = in_collection
            && !done
            && t.strip_prefix("name")
                .is_some_and(|r| r.trim_start().starts_with('='));
        if is_name {
            out.push_str(&line);
            out.push_str(if l.ends_with('\n') { "\n" } else { "" });
            done = true;
        } else {
            out.push_str(l);
        }
    }
    if !done {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !in_collection {
            out.push_str("[collection]\n");
        }
        out.push_str(&line);
        out.push('\n');
    }
    std::fs::write(&toml_path, out)?;
    let meta = dir.join("collection.flux.md");
    if meta.exists() {
        set_name(&meta, new_name)?;
    }
    Ok(())
}

pub fn create_folder(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let p = unique(parent, &slug(name), "");
    std::fs::create_dir_all(&p)?;
    std::fs::write(p.join("folder.flux.md"), new_folder(name))?;
    Ok(p)
}

/// Rewrite the front matter `name` of a file through the span editor. Missing files are ignored.
fn set_name(path: &Path, name: &str) -> io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let doc = parse_with(&text, file_kind_for(path), &ProtocolRegistry::builtin());
    let edits = set_front_matter(&doc, "name", &FmValue::Scalar(name.to_string()));
    if let Some(new) = apply(&doc, &edits) {
        std::fs::write(path, new)?;
    }
    Ok(())
}

/// Rename a request file or folder; the file/dir name follows the slug of the new display name.
pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    if path.is_dir() {
        let target = unique(parent, &slug(new_name), "");
        if target != path {
            std::fs::rename(path, &target)?;
        }
        let meta = target.join("folder.flux.md");
        if meta.exists() {
            set_name(&meta, new_name)?;
        } else {
            std::fs::write(&meta, new_folder(new_name))?;
        }
        Ok(target)
    } else {
        let target = if slug(new_name)
            == path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .trim_end_matches(".flux.md")
        {
            path.to_path_buf()
        } else {
            unique(parent, &slug(new_name), ".flux.md")
        };
        if is_request_file(path) {
            set_name(path, new_name)?;
        }
        if target != path {
            std::fs::rename(path, &target)?;
        }
        Ok(target)
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let t = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &t)?;
        } else {
            std::fs::copy(e.path(), t)?;
        }
    }
    Ok(())
}

pub fn duplicate(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("copy");
    if path.is_dir() {
        let target = unique(parent, &format!("{fname}-copy"), "");
        copy_dir(path, &target)?;
        let meta = target.join("folder.flux.md");
        if meta.exists() {
            let text = std::fs::read_to_string(&meta)?;
            let d = parse_with(&text, FileKind::Folder, &ProtocolRegistry::builtin());
            let n = format!("{} copy", d.name().unwrap_or(fname));
            set_name(&meta, &n)?;
        }
        Ok(target)
    } else {
        let stem = fname.trim_end_matches(".flux.md").trim_end_matches(".md");
        let ext = if fname.ends_with(".flux.md") {
            ".flux.md"
        } else {
            ".md"
        };
        let target = unique(parent, &format!("{stem}-copy"), ext);
        std::fs::copy(path, &target)?;
        if is_request_file(&target) {
            let text = std::fs::read_to_string(&target)?;
            let d = parse_with(&text, FileKind::Request, &ProtocolRegistry::builtin());
            set_name(&target, &format!("{} copy", d.name().unwrap_or(stem)))?;
        }
        Ok(target)
    }
}

pub fn move_to(path: &Path, dest_dir: &Path) -> io::Result<PathBuf> {
    if path.is_dir() && dest_dir.starts_with(path) {
        return Err(io::Error::other("cannot move a folder into itself"));
    }
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("no file name"))?;
    let target = dest_dir.join(name);
    if target == path {
        return Ok(target);
    }
    if target.exists() {
        return Err(io::Error::other(format!(
            "{} already exists in the destination",
            name.to_string_lossy()
        )));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Move to the OS trash (never a hard delete).
pub fn delete(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(|e| io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_with_an_empty_url_parses_with_an_empty_target() {
        let dir = std::env::temp_dir().join(format!("fluxwire-emptyurl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for ws in [false, true] {
            let p = create_request_with_url(&dir, "Thing", "", ws, "").unwrap();
            let text = std::fs::read_to_string(&p).unwrap();
            let doc = crate::parse(&text);
            let req = doc.request().expect("request block");
            assert_eq!(req.target, "", "{text:?}");
            assert_eq!(
                crate::format(&text, fluxwire_model::FileKind::Request),
                text,
                "stays canonical"
            );
        }
    }

    #[test]
    fn rename_collection_replaces_only_the_name() {
        let dir = std::env::temp_dir().join(format!("fluxwire-rencoll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        create_collection(&dir, "Old").unwrap();
        let toml = dir.join("fluxwire.toml");
        let mut t = std::fs::read_to_string(&toml).unwrap();
        t.push_str("\n# keep me\n[defaults]\ntimeout = 5\n");
        std::fs::write(&toml, &t).unwrap();
        rename_collection(&dir, "New \"Name\"").unwrap();
        let after = std::fs::read_to_string(&toml).unwrap();
        assert_eq!(
            after,
            t.replace("name = \"Old\"", "name = \"New \\\"Name\\\"\"")
        );
        let meta = std::fs::read_to_string(dir.join("collection.flux.md")).unwrap();
        assert!(meta.contains("name: "), "{meta}");
        assert!(!meta.contains("name: Old"), "{meta}");
    }
}
