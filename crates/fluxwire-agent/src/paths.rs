//! The Workspace allow-list (`--root`). Every path an agent supplies is checked here.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Roots(Vec<PathBuf>);

impl Roots {
    /// Roots that do not exist are an error, so a typo cannot silently widen or empty the list.
    pub fn new(paths: &[PathBuf]) -> std::io::Result<Self> {
        paths
            .iter()
            .map(|p| p.canonicalize())
            .collect::<Result<Vec<_>, _>>()
            .map(Roots)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn list(&self) -> &[PathBuf] {
        &self.0
    }

    /// Canonical path when `path` is inside a root. The longest existing ancestor is
    /// canonicalized so symlinks and `..` cannot escape, and not-yet-created files can be checked.
    pub fn check(&self, path: &Path) -> Result<PathBuf, String> {
        let mut existing = path.to_path_buf();
        let mut tail = Vec::new();
        while !existing.exists() {
            match (
                existing.file_name().map(|n| n.to_owned()),
                existing.parent(),
            ) {
                (Some(n), Some(p)) => {
                    tail.push(n);
                    existing = p.to_path_buf();
                }
                _ => return Err(format!("{} does not exist", path.display())),
            }
        }
        let mut full = existing.canonicalize().map_err(|e| e.to_string())?;
        for part in tail.iter().rev() {
            if part == ".." {
                return Err(format!("{} is outside the allowed roots", path.display()));
            }
            full.push(part);
        }
        if self.0.iter().any(|r| full.starts_with(r)) {
            Ok(full)
        } else {
            Err(format!("{} is outside the allowed roots", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_ok_outside_and_dotdot_refused() {
        let dir = tempdir();
        let root = dir.join("ws");
        std::fs::create_dir_all(root.join("a")).unwrap();
        let roots = Roots::new(std::slice::from_ref(&root)).unwrap();
        assert!(roots.check(&root.join("a")).is_ok());
        assert!(roots.check(&root.join("a/new.flux.md")).is_ok());
        assert!(roots.check(&root.join("a/../..")).is_err());
        assert!(roots.check(&dir).is_err());
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(Roots::new(&[PathBuf::from("/definitely/not/here")]).is_err());
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "fw-roots-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }
}
