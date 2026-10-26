//! Script trust (SPEC §21): one decision per collection path, kept in `trust.toml` in the
//! user config directory. Opening a collection never runs scripts; the first send from a
//! collection that has scripts asks. `fluxwire run` trusts by definition and never reads this.

use std::path::{Path, PathBuf};

const FILE: &str = "trust.toml";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TrustFile {
    #[serde(default)]
    trusted: Vec<String>,
}

fn key(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn read(config_dir: &Path) -> TrustFile {
    std::fs::read_to_string(config_dir.join(FILE))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

/// Trusted collection roots, as stored.
pub fn trusted_collections(config_dir: &Path) -> Vec<PathBuf> {
    read(config_dir)
        .trusted
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

pub fn is_trusted(config_dir: &Path, root: &Path) -> bool {
    let k = key(root);
    read(config_dir).trusted.contains(&k)
}

/// Trust or revoke `root`. Writes only `trust.toml`.
pub fn set_trusted(config_dir: &Path, root: &Path, trusted: bool) -> std::io::Result<()> {
    let k = key(root);
    let mut f = read(config_dir);
    f.trusted.retain(|t| *t != k);
    if trusted {
        f.trusted.push(k);
    }
    std::fs::create_dir_all(config_dir)?;
    let text = toml::to_string_pretty(&f).map_err(std::io::Error::other)?;
    std::fs::write(config_dir.join(FILE), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_per_collection_and_revocable() {
        let dir = std::env::temp_dir().join(format!("fw-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert!(!is_trusted(&dir, &a));
        set_trusted(&dir, &a, true).unwrap();
        set_trusted(&dir, &a, true).unwrap();
        assert!(is_trusted(&dir, &a) && !is_trusted(&dir, &b));
        assert_eq!(trusted_collections(&dir).len(), 1);
        set_trusted(&dir, &a, false).unwrap();
        assert!(!is_trusted(&dir, &a));
        std::fs::remove_dir_all(dir).ok();
    }
}
