//! Compact chips for executable blocks ("POST /v1/payments", "3 tests"), SPEC §16.

use fluxwire_model::{BlockKind, FluxDoc};

/// The request sub-tab a chip jumps to (indexes as in the request editor), or
/// [`OPEN_AS_TEXT`].
pub const OPEN_AS_TEXT: usize = usize::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    pub tab: usize,
}

/// Chip for block `i`, or `None` for prose code and `example` blocks (rendered as samples).
pub fn chip(doc: &FluxDoc, i: usize) -> Option<Chip> {
    let b = &doc.blocks[i];
    let lang = &b.info.language;
    let (label, tab) = match b.kind {
        BlockKind::Request => {
            let label = match doc.request() {
                Some(r) => format!("{} {}", r.method, r.target),
                None => lang.clone(),
            };
            (label, 2)
        }
        BlockKind::Expect => ("expect".to_string(), 5),
        BlockKind::Test => {
            let text = doc.block_text(i);
            let n = text.matches("test(").count().max(1);
            (format!("{n} test{}", if n == 1 { "" } else { "s" }), 5)
        }
        BlockKind::Pre => (format!("{lang} pre script"), 4),
        BlockKind::Post => (format!("{lang} post script"), 4),
        BlockKind::Message => (
            format!(
                "{lang} message {}",
                b.info.label.clone().unwrap_or_default()
            )
            .trim_end()
            .to_string(),
            2,
        ),
        BlockKind::Example | BlockKind::Prose => return None,
        _ => (
            b.info
                .role
                .clone()
                .map_or_else(|| lang.clone(), |r| format!("{lang} {r}")),
            OPEN_AS_TEXT,
        ),
    };
    Some(Chip { label, tab })
}

/// Chip for the block whose fence starts at `source_offset`.
pub fn chip_at(doc: &FluxDoc, source_offset: usize) -> Option<Chip> {
    let i = doc
        .blocks
        .iter()
        .position(|b| b.span.start == source_offset)?;
    chip(doc, i)
}

/// Whether the block whose fence starts at `source_offset` is an `example` block.
pub fn is_example_at(doc: &FluxDoc, source_offset: usize) -> bool {
    doc.blocks
        .iter()
        .any(|b| b.span.start == source_offset && b.kind == BlockKind::Example)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "---\nname: A\n---\n\n# A\n\n```http\nPOST {{baseUrl}}/v1/p\n```\n\n```http example\nHTTP 201\n\n{}\n```\n\n```ts test\nfw.test(\"a\", () => {});\nfw.test(\"b\", () => {});\n```\n";

    #[test]
    fn request_example_and_test_chips() {
        let doc = fluxwire_format::parse(SRC);
        assert_eq!(
            chip(&doc, 0).unwrap(),
            Chip {
                label: "POST {{baseUrl}}/v1/p".into(),
                tab: 2
            }
        );
        assert!(chip(&doc, 1).is_none());
        assert_eq!(chip(&doc, 2).unwrap().label, "2 tests");
        let at = doc.blocks[2].span.start;
        assert_eq!(chip_at(&doc, at).unwrap().tab, 5);
        assert!(is_example_at(&doc, doc.blocks[1].span.start));
    }
}
