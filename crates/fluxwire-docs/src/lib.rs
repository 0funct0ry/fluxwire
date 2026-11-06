//! Markdown rendering and static HTML export for API documentation (SPEC §16, M13).
//!
//! GPUI-free: the block model (`model`), the chips shown for executable blocks (`chips`), the
//! generated sections (`generated`), link and heading helpers, and the static HTML writer
//! (`html`, `export`).

pub mod chips;
pub mod example;
pub mod export;
pub mod generated;
pub mod headings;
pub mod html;
pub mod links;
pub mod model;

use fluxwire_model::FluxDoc;

/// Offset where the Docs text starts: just after the front matter and its line break.
pub fn base_of(doc: &FluxDoc) -> usize {
    match doc.front.span {
        Some(s) => {
            let rest = &doc.source[s.end..];
            if rest.starts_with("\r\n") {
                s.end + 2
            } else if rest.starts_with('\n') {
                s.end + 1
            } else {
                s.end
            }
        }
        None => 0,
    }
}

/// The Markdown of a file as Docs shows it: everything after the front matter.
pub fn docs_text(doc: &FluxDoc) -> &str {
    &doc.source[base_of(doc)..]
}
