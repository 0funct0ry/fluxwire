//! Secret detection for imported environments and `fluxwire.toml` merging.

use toml_edit::{DocumentMut, value};

pub fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "secret",
        "password",
        "passwd",
        "token",
        "apikey",
        "api_key",
        "api-key",
        "private",
        "credential",
        "bearer",
    ]
    .iter()
    .any(|k| n.contains(k))
}

pub fn dotenv_value(v: &str) -> String {
    if v.is_empty()
        || v.chars()
            .any(|c| c.is_whitespace() || matches!(c, '#' | '"' | '\'' | '\\'))
    {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

pub fn dotenv(rows: &[(String, String)]) -> String {
    let mut s = String::new();
    for (k, v) in rows {
        s.push_str(&format!("{k}={}\n", dotenv_value(v)));
    }
    s
}

/// Variable names must satisfy `is_valid_var_name`; map anything else onto underscores.
pub fn var_name(n: &str) -> String {
    let mut s: String = n
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// Merge `[secrets].names` of `new` into the existing `fluxwire.toml` text. `None` when
/// nothing changes.
pub fn merge_config(old: &str, new: &str) -> Option<String> {
    let mut doc: DocumentMut = old.parse().ok()?;
    let newdoc: DocumentMut = new.parse().ok()?;
    let secrets = merge_secrets(&mut doc, &newdoc);
    let links = merge_openapi_links(&mut doc, &newdoc);
    (secrets || links).then(|| doc.to_string())
}

/// Add `[[openapi]]` entries of `new` whose `name` the existing file does not have, so a Spec
/// link imported into an existing Collection is actually recorded (SPEC §6.1).
fn merge_openapi_links(doc: &mut DocumentMut, newdoc: &DocumentMut) -> bool {
    let Some(wanted) = newdoc.get("openapi").and_then(|o| o.as_array_of_tables()) else {
        return false;
    };
    let have: Vec<String> = doc
        .get("openapi")
        .and_then(|o| o.as_array_of_tables())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut changed = false;
    for t in wanted.iter() {
        let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if have.iter().any(|h| h == name) {
            continue;
        }
        if doc.get("openapi").is_none() {
            doc["openapi"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
        if let Some(arr) = doc["openapi"].as_array_of_tables_mut() {
            arr.push(t.clone());
            changed = true;
        }
    }
    changed
}

fn merge_secrets(doc: &mut DocumentMut, newdoc: &DocumentMut) -> bool {
    let wanted: Vec<String> = newdoc
        .get("secrets")
        .and_then(|s| s.get("names"))
        .and_then(|n| n.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return false;
    }
    let mut names: Vec<String> = doc
        .get("secrets")
        .and_then(|s| s.get("names"))
        .and_then(|n| n.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let before = names.len();
    for w in wanted {
        if !names.contains(&w) {
            names.push(w);
        }
    }
    if names.len() == before {
        return false;
    }
    let mut arr = toml_edit::Array::new();
    for n in names {
        arr.push(n);
    }
    if doc.get("secrets").is_none() {
        doc["secrets"] = toml_edit::table();
    }
    doc["secrets"]["names"] = value(arr);
    true
}

#[cfg(test)]
mod link_merge_tests {
    use super::merge_config;

    #[test]
    fn spec_link_is_added_to_an_existing_config_once() {
        let old = "[collection]\nname = \"x\"\nformat = 1\n";
        let new = "[collection]\nname = \"n\"\nformat = 1\n\n[[openapi]]\nname = \"pets\"\nsource = \"openapi/pets.yaml\"\nfolder = \"pets\"\n";
        let merged = merge_config(old, new).unwrap();
        assert!(merged.starts_with(old) && merged.contains("[[openapi]]\nname = \"pets\""));
        assert_eq!(
            merge_config(&merged, new),
            None,
            "a second import adds nothing"
        );
    }
}
