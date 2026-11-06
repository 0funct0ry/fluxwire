//! Span edits used by OpenAPI pull (SPEC §15.3) that the request editor does not need:
//! the `http expect` status line and `http form` / `http multipart` lines.

use crate::edit::{eol, insert_block, set_block_content};
use fluxwire_model::{BlockData, BlockKind, Edit, FluxDoc};

/// Set the status of the `http expect` block to `HTTP <status>`, keeping every other line.
/// Creates the block when missing; `None` leaves the document alone.
pub fn set_expect_status(doc: &FluxDoc, status: u16) -> Vec<Edit> {
    let line = format!("HTTP {status}");
    let Some(i) = doc.blocks.iter().position(|b| b.kind == BlockKind::Expect) else {
        return insert_block(doc, BlockKind::Expect, "http expect", &line);
    };
    let content = doc.block_text(i);
    let nl = eol(doc);
    let mut lines: Vec<&str> = content.lines().collect();
    match lines
        .iter()
        .position(|l| l.trim_start().starts_with("HTTP"))
    {
        Some(p) if lines[p].trim() == line => return Vec::new(),
        Some(p) => lines[p] = &line,
        None => lines.insert(0, &line),
    }
    set_block_content(doc, i, &lines.join(nl))
}

/// One `name: value` line of a form or multipart block; `# ` prefix when disabled.
pub fn kv_line(enabled: bool, name: &str, value: &str) -> String {
    let p = if enabled { "" } else { "# " };
    format!("{p}{name}: {value}")
}

/// Edit a single line of the form/multipart block `kind`: replace the value of `name`, add the
/// line when absent. Returns no edits when the value is already `value`.
pub fn set_kv_value(doc: &FluxDoc, kind: BlockKind, name: &str, value: &str) -> Vec<Edit> {
    let info = if kind == BlockKind::Multipart {
        "http multipart"
    } else {
        "http form"
    };
    let Some(i) = doc.blocks.iter().position(|b| b.kind == kind) else {
        return insert_block(doc, kind, info, &kv_line(true, name, value));
    };
    let lines = match &doc.data[i] {
        BlockData::Kv(l) => l.as_slice(),
        _ => &[],
    };
    match lines.iter().find(|l| l.name == name) {
        Some(l) if l.value == value => Vec::new(),
        Some(l) => vec![Edit::replace(l.span, kv_line(l.enabled, name, value))],
        None => {
            let mut text = doc.block_text(i).trim_end_matches(['\n', '\r']).to_string();
            if !text.is_empty() {
                text.push_str(eol(doc));
            }
            text.push_str(&kv_line(true, name, value));
            set_block_content(doc, i, &text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, f: impl Fn(&FluxDoc) -> Vec<Edit>) -> String {
        let d = crate::parse(src);
        fluxwire_model::apply_edits(src, &f(&d)).unwrap()
    }

    #[test]
    fn expect_status_replaced_in_place() {
        let src = "```http\nGET /a\n```\n\n```http expect\nHTTP 200\nContent-Type: application/json\n```\n";
        let out = run(src, |d| set_expect_status(d, 201));
        assert_eq!(out, src.replace("HTTP 200", "HTTP 201"));
    }

    #[test]
    fn expect_status_same_is_noop_and_missing_creates() {
        let src = "```http\nGET /a\n```\n\n```http expect\nHTTP 200\n```\n";
        assert_eq!(run(src, |d| set_expect_status(d, 200)), src);
        let bare = "```http\nGET /a\n```\n";
        let out = run(bare, |d| set_expect_status(d, 204));
        assert!(out.contains("```http expect\nHTTP 204\n```"), "{out}");
    }

    #[test]
    fn kv_value_edit_and_add() {
        let src = "```http\nPOST /a\n```\n\n```http form\nname: x\n# off: 1\n```\n";
        let out = run(src, |d| set_kv_value(d, BlockKind::Form, "name", "y"));
        assert_eq!(out, src.replace("name: x", "name: y"));
        let out = run(src, |d| set_kv_value(d, BlockKind::Form, "new", "z"));
        assert!(out.contains("# off: 1\nnew: z\n```"), "{out}");
        assert_eq!(
            run(src, |d| set_kv_value(d, BlockKind::Form, "name", "x")),
            src
        );
    }
}
