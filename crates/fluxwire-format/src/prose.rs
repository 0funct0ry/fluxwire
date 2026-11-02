//! Prose regions of a `.flux.md` file and an edit that can only change them (SPEC §7.3, §7.10).
//!
//! Prose is everything outside the front matter and outside recognised fences. Plain code
//! fences (`Prose`/`Example` blocks) are not Fluxwire's, so they count as prose.

use fluxwire_model::{BlockKind, Edit, FluxDoc, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseEditError {
    /// The range touches front matter or a recognised fence, or spans two prose regions.
    TouchesProtected,
    /// The new text would create a recognised fence or change the front matter.
    CreatesBlock,
}

fn protected(kind: &BlockKind) -> bool {
    !matches!(kind, BlockKind::Prose | BlockKind::Example)
}

/// Offset just after the end-of-line that follows `at` (or `at` when none follows).
fn after_eol(src: &str, at: usize) -> usize {
    match src[at..].find('\n') {
        Some(i) if src[at..at + i].trim().is_empty() => at + i + 1,
        _ => at,
    }
}

/// Editable spans in source order. A region starts on the line after a protected construct,
/// so an insertion at its start never lands on a fence or `---` line.
pub fn prose_regions(doc: &FluxDoc) -> Vec<Span> {
    let src = &doc.source;
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    if let Some(fm) = doc.front.span {
        cuts.push((fm.start, fm.end));
    }
    for b in doc.blocks.iter().filter(|b| protected(&b.kind)) {
        cuts.push((b.span.start, b.span.end));
    }
    cuts.sort();
    let mut out = Vec::new();
    let mut at = 0;
    for (s, e) in cuts {
        if s > at {
            out.push(Span::new(at, s));
        }
        at = at.max(after_eol(src, e));
        if e >= src.len() {
            at = src.len();
        }
    }
    out.push(Span::new(at, src.len()));
    out.retain(|r| r.start <= r.end);
    out
}

fn signature(doc: &FluxDoc) -> (Option<String>, Vec<String>) {
    (
        doc.front.span.map(|s| s.slice(&doc.source).to_string()),
        doc.blocks
            .iter()
            .filter(|b| protected(&b.kind))
            .map(|b| b.span.slice(&doc.source).to_string())
            .collect(),
    )
}

/// Replace `range` (inside one prose region) with `text`. Verified by reparsing: the front
/// matter and every recognised fence must be byte-identical afterwards.
pub fn edit_prose(doc: &FluxDoc, range: Span, text: &str) -> Result<Vec<Edit>, ProseEditError> {
    if range.start > range.end || range.end > doc.source.len() {
        return Err(ProseEditError::TouchesProtected);
    }
    let inside = prose_regions(doc)
        .iter()
        .any(|r| range.start >= r.start && range.end <= r.end);
    if !inside {
        return Err(ProseEditError::TouchesProtected);
    }
    let edit = Edit::replace(range, text.to_string());
    let new = fluxwire_model::apply_edits(&doc.source, std::slice::from_ref(&edit))
        .ok_or(ProseEditError::TouchesProtected)?;
    if signature(&crate::parse(&new)) != signature(doc) {
        return Err(ProseEditError::CreatesBlock);
    }
    Ok(vec![edit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SRC: &str = "---\nname: A\n---\n\nIntro text.\n\n```http\nGET /x\n```\n\nMiddle.\n\n```ts pre\nlet a = 1;\n```\n\nEnd.\n";

    fn doc() -> FluxDoc {
        crate::parse(SRC)
    }

    fn text(r: Span) -> &'static str {
        &SRC[r.start..r.end]
    }

    #[test]
    fn regions_exclude_front_matter_and_fences() {
        let d = doc();
        let rs = prose_regions(&d);
        assert_eq!(rs.len(), 3);
        assert_eq!(text(rs[0]), "\nIntro text.\n\n");
        assert_eq!(text(rs[1]), "\nMiddle.\n\n");
        assert_eq!(text(rs[2]), "\nEnd.\n");
    }

    #[test]
    fn edit_inside_prose_changes_only_prose() {
        let d = doc();
        let at = SRC.find("Intro").unwrap();
        let e = edit_prose(&d, Span::new(at, at + 5), "Hello").unwrap();
        let new = fluxwire_model::apply_edits(SRC, &e).unwrap();
        assert!(new.contains("Hello text.") && new.contains("```ts pre\nlet a = 1;\n```"));
    }

    #[test]
    fn edits_touching_protected_spans_are_refused() {
        let d = doc();
        let fm = SRC.find("name").unwrap();
        assert_eq!(
            edit_prose(&d, Span::new(fm, fm + 1), "x"),
            Err(ProseEditError::TouchesProtected)
        );
        let g = SRC.find("GET").unwrap();
        assert_eq!(
            edit_prose(&d, Span::new(g, g), "x"),
            Err(ProseEditError::TouchesProtected)
        );
        // spanning two regions
        let a = SRC.find("Intro").unwrap();
        let b = SRC.find("Middle").unwrap();
        assert!(edit_prose(&d, Span::new(a, b), "").is_err());
    }

    #[test]
    fn creating_a_recognised_fence_is_reported() {
        let d = doc();
        let at = SRC.find("Intro").unwrap();
        assert_eq!(
            edit_prose(&d, Span::at(at), "```http\nGET /y\n```\n"),
            Err(ProseEditError::CreatesBlock)
        );
    }

    #[test]
    fn plain_code_fences_are_prose() {
        let s = "# T\n\n```sh\nls\n```\n";
        let d = crate::parse(s);
        let at = s.find("ls").unwrap();
        assert!(edit_prose(&d, Span::new(at, at + 2), "pwd").is_ok());
    }

    #[test]
    fn empty_file_accepts_insertion() {
        let d = crate::parse("");
        assert!(edit_prose(&d, Span::at(0), "Hi\n").is_ok());
    }

    proptest! {
        #[test]
        fn prose_edits_never_change_protected_content(
            start in 0usize..SRC.len(), len in 0usize..12, ins in "[a-zA-Z ,.\n`#-]{0,16}"
        ) {
            let d = doc();
            let s = (0..=start).rev().find(|i| SRC.is_char_boundary(*i)).unwrap();
            let e = (s + len).min(SRC.len());
            let e = (e..=SRC.len()).find(|i| SRC.is_char_boundary(*i)).unwrap();
            if let Ok(edits) = edit_prose(&d, Span::new(s, e), &ins) {
                let new = fluxwire_model::apply_edits(SRC, &edits).unwrap();
                prop_assert_eq!(signature(&crate::parse(&new)), signature(&d));
            }
        }
    }
}
