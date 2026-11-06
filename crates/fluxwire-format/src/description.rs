//! The description paragraph of a request (SPEC §7.3): the first paragraph after the first
//! heading, or the first paragraph when there is no heading. Only prose outside recognised
//! fences is considered, and edits go through [`crate::edit_prose`] so nothing else moves.

use crate::prose::{ProseEditError, edit_prose, prose_regions};
use fluxwire_model::{Edit, FluxDoc, Span};

/// Where the description lives: an existing paragraph, or an insertion point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptionSite {
    Paragraph(Span),
    /// No paragraph yet; insert at this offset (end of the heading line, or region start).
    Insert(usize),
}

fn is_block_start(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#')
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("> ")
        || t.starts_with('|')
        || t.starts_with("```")
        || t.starts_with("~~~")
}

/// Locate the description paragraph. A list, table, quote or second heading directly after
/// the title means there is no description paragraph.
pub fn description_site(doc: &FluxDoc) -> Option<DescriptionSite> {
    let src = &doc.source;
    let region = prose_regions(doc)
        .into_iter()
        .find(|r| !r.slice(src).trim().is_empty() || r.end == src.len())?;
    let text = region.slice(src);
    let mut offset = region.start;
    let mut after_heading = None;
    let mut seen_heading = false;
    let mut para: Option<(usize, usize)> = None;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let start = offset;
        offset += line.len();
        if para.is_none() {
            if body.trim().is_empty() {
                continue;
            }
            if !seen_heading && body.trim_start().starts_with('#') {
                seen_heading = true;
                after_heading = Some(start + line.len());
                continue;
            }
            if is_block_start(body) {
                break;
            }
            para = Some((start, start + body.len()));
        } else if body.trim().is_empty() || is_block_start(body) {
            break;
        } else if let Some((s, _)) = para {
            para = Some((s, start + body.len()));
        }
    }
    match para {
        Some((s, e)) => Some(DescriptionSite::Paragraph(Span::new(s, e))),
        None => Some(DescriptionSite::Insert(
            after_heading.unwrap_or(region.start),
        )),
    }
}

/// Current description text, if a paragraph exists.
pub fn description(doc: &FluxDoc) -> Option<String> {
    match description_site(doc)? {
        DescriptionSite::Paragraph(s) => Some(s.slice(&doc.source).to_string()),
        DescriptionSite::Insert(_) => None,
    }
}

/// Edits that set the description to `text` (empty removes the paragraph).
pub fn set_description(doc: &FluxDoc, text: &str) -> Result<Vec<Edit>, ProseEditError> {
    let text = text.trim();
    match description_site(doc).ok_or(ProseEditError::TouchesProtected)? {
        DescriptionSite::Paragraph(s) if text.is_empty() => {
            let src = &doc.source;
            let mut end = s.end;
            while src[end..].starts_with(['\n', '\r']) {
                end += 1;
            }
            edit_prose(doc, Span::new(s.start, end), "")
        }
        DescriptionSite::Paragraph(s) => edit_prose(doc, s, text),
        DescriptionSite::Insert(_) if text.is_empty() => Ok(Vec::new()),
        DescriptionSite::Insert(at) => {
            let src = &doc.source;
            let lead = if at > 0 && !src[..at].ends_with("\n\n") {
                if src[..at].ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                }
            } else {
                ""
            };
            let trail = if src[at..].starts_with('\n') || at == src.len() {
                "\n"
            } else {
                "\n\n"
            };
            edit_prose(doc, Span::new(at, at), &format!("{lead}{text}{trail}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(src: &str, text: &str) -> String {
        let d = crate::parse(src);
        let e = set_description(&d, text).unwrap();
        fluxwire_model::apply_edits(src, &e).unwrap()
    }

    const BASE: &str = "---\nname: A\n---\n\n# Get A\n\nOld text\nline two.\n\nMore prose.\n\n```http\nGET /a\n```\n";

    #[test]
    fn finds_paragraph_after_heading() {
        let d = crate::parse(BASE);
        assert_eq!(description(&d).as_deref(), Some("Old text\nline two."));
    }

    #[test]
    fn replaces_only_the_paragraph() {
        let out = apply(BASE, "New");
        assert_eq!(out, BASE.replace("Old text\nline two.", "New"));
    }

    #[test]
    fn no_heading_uses_first_paragraph() {
        let src = "---\nname: A\n---\n\nHello.\n\n```http\nGET /a\n```\n";
        assert_eq!(description(&crate::parse(src)).as_deref(), Some("Hello."));
    }

    #[test]
    fn inserts_after_heading_when_missing() {
        let src = "---\nname: A\n---\n\n# Get A\n\n```http\nGET /a\n```\n";
        let out = apply(src, "Fresh");
        assert!(out.contains("# Get A\n\nFresh\n\n```http"), "{out}");
    }

    #[test]
    fn list_after_heading_is_not_a_description() {
        let src = "---\nname: A\n---\n\n# T\n\n- a\n- b\n\n```http\nGET /a\n```\n";
        assert_eq!(description(&crate::parse(src)), None);
    }

    #[test]
    fn empty_removes_paragraph_and_keeps_the_rest() {
        let out = apply(BASE, "");
        assert!(out.contains("# Get A\n\nMore prose.\n"), "{out}");
    }

    #[test]
    fn same_text_is_a_no_op() {
        let out = apply(BASE, "Old text\nline two.");
        assert_eq!(out, BASE);
    }
}
