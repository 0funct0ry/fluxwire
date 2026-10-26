//! Side-by-side diffing for history entries (SPEC §13): headers and JSON-aware bodies.

use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub left: Option<String>,
    pub right: Option<String>,
    pub kind: DiffKind,
}

fn line_diff(a: &str, b: &str) -> Vec<DiffRow> {
    let d = TextDiff::from_lines(a, b);
    let mut rows = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let flush_removed = |rows: &mut Vec<DiffRow>, pending: &mut Vec<String>| {
        for l in pending.drain(..) {
            rows.push(DiffRow {
                left: Some(l),
                right: None,
                kind: DiffKind::Removed,
            });
        }
    };
    for c in d.iter_all_changes() {
        let text = c.value().trim_end_matches('\n').to_string();
        match c.tag() {
            ChangeTag::Equal => {
                flush_removed(&mut rows, &mut pending);
                rows.push(DiffRow {
                    left: Some(text.clone()),
                    right: Some(text),
                    kind: DiffKind::Same,
                });
            }
            ChangeTag::Delete => pending.push(text),
            ChangeTag::Insert => {
                if pending.is_empty() {
                    rows.push(DiffRow {
                        left: None,
                        right: Some(text),
                        kind: DiffKind::Added,
                    });
                } else {
                    // pair a deletion with its replacement on one row
                    let l = pending.remove(0);
                    rows.push(DiffRow {
                        left: Some(l),
                        right: Some(text),
                        kind: DiffKind::Changed,
                    });
                }
            }
        }
    }
    flush_removed(&mut rows, &mut pending);
    rows
}

/// Headers compared by lowercase name, in name order; repeated names compare in order.
pub fn diff_headers(a: &[(String, String)], b: &[(String, String)]) -> Vec<DiffRow> {
    let render = |h: &[(String, String)]| {
        let mut v: Vec<String> = h
            .iter()
            .map(|(k, v)| format!("{}: {v}", k.to_ascii_lowercase()))
            .collect();
        v.sort();
        v.join("\n") + if h.is_empty() { "" } else { "\n" }
    };
    line_diff(&render(a), &render(b))
}

fn canonical_json(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&v).ok().map(|s| s + "\n")
}

/// JSON bodies are pretty-printed first (so whitespace changes vanish and each key sits on its
/// own line); anything else is compared as text.
pub fn diff_json_or_text(a: &str, b: &str) -> Vec<DiffRow> {
    match (canonical_json(a), canonical_json(b)) {
        (Some(x), Some(y)) => line_diff(&x, &y),
        _ => line_diff(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(rows: &[DiffRow], k: DiffKind) -> usize {
        rows.iter().filter(|r| r.kind == k).count()
    }

    #[test]
    fn json_whitespace_is_ignored_and_changes_found() {
        let same = diff_json_or_text("{\"a\":1,\"b\":2}", "{ \"a\": 1,\n \"b\": 2 }");
        assert!(same.iter().all(|r| r.kind == DiffKind::Same));
        let d = diff_json_or_text("{\"a\":1,\"b\":2}", "{\"a\":1,\"b\":3,\"c\":4}");
        assert_eq!(count(&d, DiffKind::Changed), 1);
        assert_eq!(count(&d, DiffKind::Added), 1);
    }

    #[test]
    fn headers_add_remove_and_case() {
        let a = vec![
            ("Accept".to_string(), "x".to_string()),
            ("X-Old".into(), "1".into()),
        ];
        let b = vec![
            ("accept".to_string(), "x".to_string()),
            ("X-New".into(), "1".into()),
        ];
        let d = diff_headers(&a, &b);
        assert_eq!(count(&d, DiffKind::Same), 1);
        assert_eq!(count(&d, DiffKind::Changed), 1);
    }

    #[test]
    fn plain_text() {
        let d = diff_json_or_text("one\ntwo\n", "one\n");
        assert_eq!(count(&d, DiffKind::Removed), 1);
    }
}
