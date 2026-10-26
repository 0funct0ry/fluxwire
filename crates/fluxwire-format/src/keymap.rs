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
}
