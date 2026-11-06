//! Headings of a docs file, for anchors and the quick-search index (SPEC §16).

use crate::model::{Block, plain};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub anchor: String,
}

/// GitHub-style anchor: lower-case, spaces to `-`, punctuation dropped.
pub fn anchor(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Headings of `markdown` in order (top level only; nested quotes and lists are skipped).
pub fn of(markdown: &str) -> Vec<Heading> {
    crate::model::parse(markdown)
        .iter()
        .filter_map(|b| match b {
            Block::Heading(level, inl) => {
                let text = plain(inl);
                Some(Heading {
                    level: *level,
                    anchor: anchor(&text),
                    text,
                })
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_with_anchors() {
        let h = of("# Create payment\n\ntext\n\n## Edge cases, v2\n");
        assert_eq!(
            h[0],
            Heading {
                level: 1,
                text: "Create payment".into(),
                anchor: "create-payment".into()
            }
        );
        assert_eq!(h[1].anchor, "edge-cases-v2");
    }
}
