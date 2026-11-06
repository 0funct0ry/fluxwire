//! Docs editor model (M9.06): the file after its front matter, prose editable, everything
//! else read-only. Pure helpers; the entity wiring is in `ops.rs`.

use fluxwire_model::{FluxDoc, Span};
use std::ops::Range;

/// Offset where the Docs text starts: just after the front matter and its line break.
pub fn base_of(doc: &FluxDoc) -> usize {
    match doc.front.span {
        Some(s) => {
            let src = &doc.source;
            if src[s.end..].starts_with("\r\n") {
                s.end + 2
            } else if src[s.end..].starts_with('\n') {
                s.end + 1
            } else {
                s.end
            }
        }
        None => 0,
    }
}

/// Read-only ranges of the Docs text (relative to `base_of`): everything outside the prose
/// regions.
pub fn protected_ranges(doc: &FluxDoc) -> Vec<Range<usize>> {
    let base = base_of(doc);
    let len = doc.source.len();
    let mut out = Vec::new();
    let mut at = base;
    for r in fluxwire_format::prose_regions(doc) {
        let (s, e) = (r.start.max(base), r.end.max(base));
        if s > at {
            out.push(at - base..s - base);
        }
        at = at.max(e);
    }
    if at < len {
        out.push(at - base..len - base);
    }
    out
}

/// The single replacement that turns `old` into `new` (common prefix and suffix removed),
/// as `(range in old, replacement)`.
pub fn diff(old: &str, new: &str) -> (Range<usize>, String) {
    let mut p = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(p) || !new.is_char_boundary(p) {
        p -= 1;
    }
    let max_s = old.len().min(new.len()) - p;
    let mut s = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_s)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s) {
        s -= 1;
    }
    (p..old.len() - s, new[p..new.len() - s].to_string())
}

/// Chip text for a protected block, shown beside it: what it is and where to edit it.
pub fn chip_for(info: &str) -> String {
    let lang = info.split_whitespace().next().unwrap_or("");
    let role = info.split_whitespace().nth(1).unwrap_or("");
    match (lang, role) {
        (_, "pre") | (_, "post") => format!("{lang} {role} · edit in Scripts"),
        (_, "test") | (_, "expect") => format!("{lang} {role} · edit in Tests"),
        ("http", "") | ("ws", "") => format!("{lang} request · edit in Params/Body"),
        _ => format!("{info} · edit in Open as text"),
    }
}

/// Chip tag meaning "open the raw file" instead of a request sub-tab.
pub const OPEN_AS_TEXT: usize = usize::MAX;

/// The request sub-tab that owns a protected block: Body for the request, Scripts for
/// `pre`/`post`, Tests for `test`/`expect`; anything else is edited as text.
pub fn chip_tab(kind: &fluxwire_model::BlockKind) -> usize {
    use fluxwire_model::BlockKind::*;
    match kind {
        Request => 2,
        Pre | Post => 4,
        Test | Expect => 5,
        _ => OPEN_AS_TEXT,
    }
}

/// Applies a Docs-text change to the document source through the prose editor.
pub fn apply(
    doc: &FluxDoc,
    old: &str,
    new: &str,
) -> Result<String, fluxwire_format::ProseEditError> {
    let base = base_of(doc);
    let (r, text) = diff(old, new);
    let edits = fluxwire_format::edit_prose(doc, Span::new(base + r.start, base + r.end), &text)?;
    fluxwire_model::apply_edits(&doc.source, &edits)
        .ok_or(fluxwire_format::ProseEditError::TouchesProtected)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "---\nname: A\n---\n\nIntro.\n\n```http\nGET /x\n```\n\nEnd.\n";

    #[test]
    fn base_is_after_front_matter() {
        let d = fluxwire_format::parse(SRC);
        assert_eq!(&SRC[base_of(&d)..base_of(&d) + 1], "\n");
        assert_eq!(base_of(&fluxwire_format::parse("hi")), 0);
    }

    #[test]
    fn protected_ranges_cover_the_fence_only() {
        let d = fluxwire_format::parse(SRC);
        let base = base_of(&d);
        let r = protected_ranges(&d);
        assert_eq!(r.len(), 1);
        assert_eq!(
            &SRC[base + r[0].start..base + r[0].end],
            "```http\nGET /x\n```\n"
        );
    }

    #[test]
    fn diff_is_a_minimal_replacement() {
        assert_eq!(diff("abcd", "abXcd"), (2..2, "X".into()));
        assert_eq!(diff("abcd", "ad"), (1..3, String::new()));
        assert_eq!(diff("same", "same"), (4..4, String::new()));
        assert_eq!(diff("é", "è"), (0..2, "è".into()));
    }

    #[test]
    fn apply_changes_prose_and_refuses_blocks() {
        let d = fluxwire_format::parse(SRC);
        let base = base_of(&d);
        let old = &SRC[base..];
        let new = old.replace("Intro.", "Hello.");
        let out = apply(&d, old, &new).unwrap();
        assert!(out.contains("Hello.") && out.contains("```http\nGET /x\n```"));
        let bad = old.replace("GET /x", "GET /y");
        assert!(apply(&d, old, &bad).is_err());
    }

    #[test]
    fn chips_open_the_owning_sub_tab() {
        use fluxwire_model::BlockKind;
        assert_eq!(chip_tab(&BlockKind::Request), 2);
        assert_eq!(chip_tab(&BlockKind::Post), 4);
        assert_eq!(chip_tab(&BlockKind::Expect), 5);
        assert_eq!(chip_tab(&BlockKind::Config), OPEN_AS_TEXT);
    }

    #[test]
    fn expect_block_chip_points_to_tests() {
        assert_eq!(chip_for("http expect"), "http expect · edit in Tests");
        assert_eq!(chip_for("http"), "http request · edit in Params/Body");
    }

    #[test]
    fn chips_name_the_owning_tab() {
        assert_eq!(chip_for("http"), "http request · edit in Params/Body");
        assert_eq!(chip_for("ts pre"), "ts pre · edit in Scripts");
        assert_eq!(chip_for("ts test"), "ts test · edit in Tests");
    }
}
