//! Read-mostly code surface: line numbers plus tree-sitter highlighting for JSON (ADR-002).

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use tree_sitter::{Node, Parser};

use crate::{assets::BUFFER_FONT, theme::Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Property,
    String,
    Number,
    Boolean,
    Punct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Json,
    Plain,
}

/// Highlight spans (byte range, token) for JSON, in source order. Falls back to no spans when the
/// grammar cannot be loaded; partial or invalid JSON still highlights what parsed.
pub fn highlight_json(src: &str) -> Vec<(usize, usize, Tok)> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_json::LANGUAGE.into())
        .is_err()
    {
        return vec![];
    }
    let Some(tree) = parser.parse(src, None) else {
        return vec![];
    };
    let mut spans = vec![];
    collect(tree.root_node(), false, &mut spans);
    spans
}

fn collect(node: Node, is_key: bool, out: &mut Vec<(usize, usize, Tok)>) {
    let kind = node.kind();
    let tok = match kind {
        "string" if is_key => Some(Tok::Property),
        "string" => Some(Tok::String),
        "number" => Some(Tok::Number),
        "true" | "false" | "null" => Some(Tok::Boolean),
        "{" | "}" | "[" | "]" | "," | ":" => Some(Tok::Punct),
        _ => None,
    };
    if let Some(tok) = tok {
        out.push((node.start_byte(), node.end_byte(), tok));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let key = kind == "pair" && node.child_by_field_name("key") == Some(child);
        collect(child, key, out);
    }
}

/// Splits `src` into lines of `(text, token)` runs using `spans`.
pub fn line_runs(src: &str, spans: &[(usize, usize, Tok)]) -> Vec<Vec<(String, Tok)>> {
    let mut lines = vec![];
    let mut offset = 0;
    for line in src.split('\n') {
        let (ls, le) = (offset, offset + line.len());
        let mut runs = vec![];
        let mut pos = ls;
        for &(s, e, tok) in spans
            .iter()
            .filter(|(s, e, _)| *e > ls && *s < le.max(ls + 1))
        {
            let (s, e) = (s.max(ls), e.min(le));
            if s > pos {
                runs.push((src[pos..s].to_string(), Tok::Plain));
            }
            if e > s {
                runs.push((src[s..e].to_string(), tok));
            }
            pos = pos.max(e);
        }
        if pos < le {
            runs.push((src[pos..le].to_string(), Tok::Plain));
        }
        lines.push(runs);
        offset = le + 1;
    }
    lines
}

fn color(t: &Theme, tok: Tok) -> gpui::Hsla {
    match tok {
        Tok::Plain => t.text,
        Tok::Property => t.syntax_property,
        Tok::String => t.syntax_string,
        Tok::Number => t.syntax_number,
        Tok::Boolean => t.syntax_boolean,
        Tok::Punct => t.syntax_punctuation,
    }
}

/// Renders `text` with a line-number gutter.
pub fn code_view(text: &str, lang: Lang, t: &Theme) -> AnyElement {
    let spans = match lang {
        Lang::Json => highlight_json(text),
        Lang::Plain => vec![],
    };
    let lines = line_runs(text, &spans);
    div()
        .flex()
        .flex_col()
        .py_2()
        .font_family(BUFFER_FONT)
        .text_size(px(13.))
        .children(lines.into_iter().enumerate().map(|(i, runs)| {
            div()
                .flex()
                .flex_row()
                .h(px(20.))
                .child(
                    div()
                        .w(px(44.))
                        .pr(px(14.))
                        .flex_none()
                        .text_right()
                        .text_color(t.gutter)
                        .child((i + 1).to_string()),
                )
                .children(
                    runs.into_iter()
                        .map(|(s, tok)| div().text_color(color(t, tok)).child(s)),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_tokens_are_classified() {
        let src = r#"{"a": 1, "b": "x", "c": true}"#;
        let spans = highlight_json(src);
        let kinds: Vec<_> = spans.iter().map(|s| s.2).collect();
        assert!(kinds.contains(&Tok::Property));
        assert!(kinds.contains(&Tok::Number));
        assert!(kinds.contains(&Tok::String));
        assert!(kinds.contains(&Tok::Boolean));
        let key = spans.iter().find(|s| s.2 == Tok::Property).unwrap();
        assert_eq!(&src[key.0..key.1], "\"a\"");
    }

    #[test]
    fn runs_reassemble_each_line() {
        let src = "{\n  \"a\": 1\n}";
        let runs = line_runs(src, &highlight_json(src));
        let rebuilt: Vec<String> = runs
            .iter()
            .map(|l| l.iter().map(|(s, _)| s.as_str()).collect())
            .collect();
        assert_eq!(rebuilt.join("\n"), src);
    }

    #[test]
    fn invalid_json_still_yields_lines() {
        let src = "{\"a\": ";
        assert_eq!(line_runs(src, &highlight_json(src)).len(), 1);
    }
}
