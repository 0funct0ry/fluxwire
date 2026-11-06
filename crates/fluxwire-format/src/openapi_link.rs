//! Editing the `[[openapi]]` Spec links of `fluxwire.toml` (SPEC §6.1) without touching
//! anything else in the file.

use std::path::Path;

/// Remove the Spec link called `name`. Returns whether an entry was removed. The spec file,
/// the Requests and the base snapshot are left alone; callers decide about the snapshot.
pub fn remove_link(collection_root: &Path, name: &str) -> std::io::Result<bool> {
    let path = collection_root.join("fluxwire.toml");
    let text = std::fs::read_to_string(&path)?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| std::io::Error::other(e.to_string()))?;
    let Some(arr) = doc
        .get_mut("openapi")
        .and_then(|o| o.as_array_of_tables_mut())
    else {
        return Ok(false);
    };
    let Some(i) = arr
        .iter()
        .position(|t| t.get("name").and_then(|n| n.as_str()) == Some(name))
    else {
        return Ok(false);
    };
    arr.remove(i);
    if arr.is_empty() {
        doc.remove("openapi");
    }
    std::fs::write(&path, doc.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_the_named_link_and_keeps_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let toml = "# keep\n[collection]\nname = \"x\"\nformat = 1\n\n[[openapi]]\nname = \"a\"\nsource = \"a.yaml\"\n\n[[openapi]]\nname = \"b\"\nsource = \"b.yaml\"\n";
        std::fs::write(d.path().join("fluxwire.toml"), toml).unwrap();
        assert!(remove_link(d.path(), "a").unwrap());
        let out = std::fs::read_to_string(d.path().join("fluxwire.toml")).unwrap();
        assert!(
            out.starts_with("# keep\n[collection]")
                && out.contains("name = \"b\"")
                && !out.contains("a.yaml")
        );
        assert!(remove_link(d.path(), "b").unwrap());
        let out = std::fs::read_to_string(d.path().join("fluxwire.toml")).unwrap();
        assert!(!out.contains("openapi"));
        assert!(!remove_link(d.path(), "b").unwrap());
    }
}
