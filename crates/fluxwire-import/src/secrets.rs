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
        return None;
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
        return None;
    }
    let mut arr = toml_edit::Array::new();
    for n in names {
        arr.push(n);
    }
    if doc.get("secrets").is_none() {
        doc["secrets"] = toml_edit::table();
    }
    doc["secrets"]["names"] = value(arr);
    Some(doc.to_string())
}
