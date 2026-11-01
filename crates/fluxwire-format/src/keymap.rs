//! `keymap.toml` (user config dir, SPEC §17.6 / §19): `[[bindings]]` tables with an optional
//! `context` and any number of `"keystroke" = "namespace::Action"` entries. No GPUI types here;
//! the UI resolves actions and keystrokes.

use std::path::Path;

/// One `"keystroke" = "action"` entry with the context of its table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapEntry {
    pub context: Option<String>,
    pub keystroke: String,
    /// Action name as written (`request::Send`, `Send`, `fluxwire::Send`). Empty or `none`
    /// removes the default binding of whatever the keystroke was bound to.
    pub action: String,
    /// 1-based line of the entry in the file (best effort, 0 if not found).
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    pub entries: Vec<KeymapEntry>,
    /// Problems that stop part of the file from loading: `(line, message)`.
    pub errors: Vec<(usize, String)>,
}

pub fn parse_keymap(text: &str) -> Keymap {
    let mut km = Keymap::default();
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map_or(0, |s| {
                text[..s.start.min(text.len())].matches('\n').count() + 1
            });
            km.errors
                .push((line, format!("keymap.toml: {}", e.message())));
            return km;
        }
    };
    let find_line = |needle: &str| {
        text.lines()
            .position(|l| l.contains(needle))
            .map_or(0, |i| i + 1)
    };
    for key in table.keys() {
        if key != "bindings" {
            km.errors
                .push((find_line(key), format!("unknown top-level key `{key}`")));
        }
    }
    let Some(bindings) = table.get("bindings") else {
        return km;
    };
    let Some(arr) = bindings.as_array() else {
        km.errors.push((
            find_line("bindings"),
            "`bindings` must be an array of tables".into(),
        ));
        return km;
    };
    for b in arr {
        let Some(t) = b.as_table() else {
            km.errors.push((
                find_line("bindings"),
                "`bindings` entries must be tables".into(),
            ));
            continue;
        };
        let context = t
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        for (k, v) in t {
            if k == "context" {
                continue;
            }
            match v.as_str() {
                Some(a) => km.entries.push(KeymapEntry {
                    context: context.clone(),
                    keystroke: k.clone(),
                    action: a.to_string(),
                    line: find_line(&format!("\"{k}\"")).max(find_line(k)),
                }),
                None => km.errors.push((
                    find_line(k),
                    format!("binding `{k}` must be an action name string"),
                )),
            }
        }
    }
    km
}

/// Missing file yields an empty keymap; read errors other than "not found" are reported.
pub fn load_keymap(config_dir: &Path) -> Keymap {
    match std::fs::read_to_string(config_dir.join("keymap.toml")) {
        Ok(t) => parse_keymap(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Keymap::default(),
        Err(e) => Keymap {
            entries: vec![],
            errors: vec![(0, format!("could not read keymap.toml: {e}"))],
        },
    }
}

/// Sets `keystroke` to `action` in the `[[bindings]]` table with the given `context`, creating
/// the table when there is none. An existing entry for the same context and keystroke is
/// replaced. Everything else in the file (comments, other tables, other keys) is untouched.
pub fn set_binding(text: &str, context: Option<&str>, keystroke: &str, action: &str) -> String {
    let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
        return text.to_string();
    };
    if doc.get("bindings").is_none() {
        doc["bindings"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let Some(arr) = doc["bindings"].as_array_of_tables_mut() else {
        return text.to_string();
    };
    let ix = arr
        .iter()
        .position(|t| t.get("context").and_then(|v| v.as_str()) == context);
    let i = ix.unwrap_or_else(|| {
        let mut t = toml_edit::Table::new();
        if let Some(c) = context {
            t["context"] = toml_edit::value(c);
        }
        arr.push(t);
        arr.len() - 1
    });
    arr.get_mut(i)
        .expect("index in range")
        .insert(keystroke, toml_edit::value(action));
    doc.to_string()
}

/// Removes the entry for `keystroke` in `context`, dropping its table when nothing but the
/// `context` key is left.
pub fn remove_binding(text: &str, context: Option<&str>, keystroke: &str) -> String {
    remove_entries(text, |c, k, _| c == context && k == keystroke)
}

/// Removes every entry whose action name satisfies `is_action` (any context).
pub fn remove_action_bindings(text: &str, is_action: impl Fn(&str) -> bool) -> String {
    remove_entries(text, |_, _, a| is_action(a))
}

/// Removes every entry for which `remove(context, keystroke, action)` is true.
pub fn remove_entries(text: &str, remove: impl Fn(Option<&str>, &str, &str) -> bool) -> String {
    let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
        return text.to_string();
    };
    let Some(arr) = doc
        .get_mut("bindings")
        .and_then(|i| i.as_array_of_tables_mut())
    else {
        return text.to_string();
    };
    for t in arr.iter_mut() {
        let ctx = t
            .get("context")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let doomed: Vec<String> = t
            .iter()
            .filter(|(k, _)| *k != "context")
            .filter(|(k, v)| v.as_str().is_some_and(|a| remove(ctx.as_deref(), k, a)))
            .map(|(k, _)| k.to_string())
            .collect();
        for k in doomed {
            t.remove(&k);
        }
    }
    let empty: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, t)| t.iter().all(|(k, _)| k == "context"))
        .map(|(i, _)| i)
        .collect();
    for i in empty.into_iter().rev() {
        arr.remove(i);
    }
    if arr.is_empty() {
        doc.remove("bindings");
    }
    doc.to_string()
}

/// Writes `keymap.toml` atomically (temp file in the same directory, then rename).
pub fn save_keymap(config_dir: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let tmp = config_dir.join(".keymap.toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, config_dir.join("keymap.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contexts_and_entries() {
        let km = parse_keymap(
            "[[bindings]]\ncontext = \"RequestEditor\"\n\"cmd-r\" = \"request::Send\"\n\n[[bindings]]\n\"cmd-1\" = \"tab::Close\"\n",
        );
        assert!(km.errors.is_empty(), "{:?}", km.errors);
        assert_eq!(km.entries.len(), 2);
        let send = km.entries.iter().find(|e| e.keystroke == "cmd-r").unwrap();
        assert_eq!(send.context.as_deref(), Some("RequestEditor"));
        assert_eq!(send.action, "request::Send");
        assert_eq!(send.line, 3);
        assert_eq!(
            km.entries
                .iter()
                .find(|e| e.keystroke == "cmd-1")
                .unwrap()
                .context,
            None
        );
    }

    #[test]
    fn reports_bad_input_without_dropping_the_rest() {
        let km = parse_keymap("[[bindings]]\n\"cmd-r\" = 3\n\"cmd-s\" = \"Save\"\n");
        assert_eq!(km.entries.len(), 1);
        assert_eq!(km.errors.len(), 1);
        let km = parse_keymap("[[bindings\n");
        assert!(km.entries.is_empty() && !km.errors.is_empty());
        assert!(parse_keymap("").entries.is_empty());
    }

    #[test]
    fn set_binding_preserves_comments_and_other_tables() {
        let src =
            "# mine\n[[bindings]]\ncontext = \"RequestEditor\"\n\"cmd-r\" = \"Send\" # keep\n";
        let out = set_binding(src, Some("RequestEditor"), "cmd-t", "Save");
        assert!(out.contains("# mine") && out.contains("# keep"), "{out}");
        assert_eq!(parse_keymap(&out).entries.len(), 2);
        assert_eq!(
            set_binding(&out, Some("RequestEditor"), "cmd-t", "Save"),
            out
        );
    }

    #[test]
    fn set_binding_creates_tables_and_replaces_same_key() {
        let out = set_binding("", None, "cmd-1", "CloseTab");
        let out = set_binding(&out, None, "cmd-1", "Quit");
        let km = parse_keymap(&out);
        assert_eq!(km.entries.len(), 1);
        assert_eq!(km.entries[0].action, "Quit");
        let out = set_binding(&out, Some("Modal"), "cmd-2", "Dismiss");
        assert_eq!(parse_keymap(&out).entries.len(), 2);
    }

    #[test]
    fn remove_drops_empty_tables_and_only_owned_entries() {
        let src = "[[bindings]]\n\"a\" = \"X\"\n\n[[bindings]]\ncontext = \"C\"\n\"b\" = \"Y\"\n\"c\" = \"Z\"\n";
        let out = remove_binding(src, Some("C"), "b");
        assert_eq!(parse_keymap(&out).entries.len(), 2);
        let out = remove_action_bindings(&out, |a| a == "X" || a == "Z");
        assert!(parse_keymap(&out).entries.is_empty());
        assert!(!out.contains("bindings"), "{out}");
    }

    #[test]
    fn save_keymap_is_atomic_and_readable() {
        let d = std::env::temp_dir().join(format!("fw-km-{}", std::process::id()));
        save_keymap(&d, "[[bindings]]\n\"a\" = \"X\"\n").unwrap();
        assert_eq!(load_keymap(&d).entries.len(), 1);
        assert!(!d.join(".keymap.toml.tmp").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
