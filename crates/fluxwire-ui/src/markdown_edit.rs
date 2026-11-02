//! Markdown editing helpers for the Docs editor and prose regions (pure, M9.06).
//!
//! Every function takes the text and caret/selection offsets and returns the edit to apply,
//! so the editor owns undo and the document owns the text.

use std::ops::Range;

/// A replacement of `range` with `text`, leaving the caret at `caret` (absolute offset after
/// the edit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdEdit {
    pub range: Range<usize>,
    pub text: String,
    pub caret: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListItem<'a> {
    indent: &'a str,
    marker: Marker<'a>,
    /// Text after the marker (and task box), without the leading space.
    content: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Marker<'a> {
    Bullet(&'a str, Option<&'a str>), // marker, task box "[ ]"/"[x]"
    Ordered(u64, char),               // number, delimiter . or )
    Quote,
}

fn parse_item(line: &str) -> Option<ListItem<'_>> {
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = line.split_at(indent_len);
    if let Some(r) = rest.strip_prefix('>') {
        return Some(ListItem {
            indent,
            marker: Marker::Quote,
            content: r.strip_prefix(' ').unwrap_or(r),
        });
    }
    for b in ["- ", "* ", "+ "] {
        if let Some(r) = rest.strip_prefix(b) {
            let (task, content) = if let Some(c) = ["[ ] ", "[x] ", "[X] "]
                .iter()
                .find_map(|t| r.strip_prefix(t).map(|c| (&r[..3], c)))
            {
                (Some(c.0), c.1)
            } else {
                (None, r)
            };
            return Some(ListItem {
                indent,
                marker: Marker::Bullet(&b[..1], task),
                content,
            });
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let d = rest[digits..].chars().next()?;
        if (d == '.' || d == ')') && rest[digits + 1..].starts_with(' ') {
            return Some(ListItem {
                indent,
                marker: Marker::Ordered(rest[..digits].parse().ok()?, d),
                content: &rest[digits + 2..],
            });
        }
    }
    None
}

fn line_bounds(text: &str, at: usize) -> Range<usize> {
    let s = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let e = text[at..].find('\n').map_or(text.len(), |i| at + i);
    s..e
}

/// Enter at `caret` (no selection) in a list item or quote: continue it, or end it when the
/// item is empty. `None` means "not in a list; insert a plain newline".
pub fn enter(text: &str, caret: usize) -> Option<MdEdit> {
    let lb = line_bounds(text, caret);
    if caret != lb.end {
        return None;
    }
    let line = &text[lb.clone()];
    let item = parse_item(line)?;
    if item.content.trim().is_empty() {
        // empty item ends the list: remove the marker, keep the indent-less empty line
        return Some(MdEdit {
            range: lb.clone(),
            text: String::new(),
            caret: lb.start,
        });
    }
    let marker = match &item.marker {
        Marker::Quote => "> ".to_string(),
        Marker::Bullet(m, task) => format!("{m} {}", if task.is_some() { "[ ] " } else { "" }),
        Marker::Ordered(n, d) => format!("{}{d} ", n + 1),
    };
    let ins = format!("\n{}{marker}", item.indent);
    let caret = caret + ins.len();
    Some(MdEdit {
        range: caret - ins.len()..caret - ins.len(),
        text: ins,
        caret,
    })
}

/// Tab / Shift+Tab on a list item nests or un-nests it, renumbering an ordered item to 1
/// when nested. `None` when the line is not a list item.
pub fn indent_item(text: &str, caret: usize, outdent: bool, unit: &str) -> Option<MdEdit> {
    let lb = line_bounds(text, caret);
    let line = &text[lb.clone()];
    let item = parse_item(line)?;
    if matches!(item.marker, Marker::Quote) {
        return None;
    }
    let new_line = if outdent {
        let strip = if item.indent.starts_with('\t') {
            1
        } else {
            unit.len().min(item.indent.len())
        };
        if item.indent.is_empty() {
            return None;
        }
        line[strip..].to_string()
    } else {
        format!("{unit}{line}")
    };
    let delta = new_line.len() as isize - line.len() as isize;
    Some(MdEdit {
        range: lb.clone(),
        caret: (caret as isize + delta).max(lb.start as isize) as usize,
        text: new_line,
    })
}

/// Renumber the ordered items of the run containing `line_start`'s list at one indent level.
pub fn renumber(text: &str) -> String {
    let mut counters: Vec<(usize, u64)> = Vec::new();
    let mut out = Vec::new();
    for line in text.split('\n') {
        match parse_item(line) {
            Some(ListItem {
                indent,
                marker: Marker::Ordered(_, d),
                content,
            }) => {
                let w = indent.len();
                counters.retain(|(i, _)| *i <= w);
                let n = match counters.iter_mut().find(|(i, _)| *i == w) {
                    Some(c) => {
                        c.1 += 1;
                        c.1
                    }
                    None => {
                        counters.push((w, 1));
                        1
                    }
                };
                out.push(format!("{indent}{n}{d} {content}"));
            }
            Some(_) => out.push(line.to_string()),
            None => {
                if line.trim().is_empty() {
                    // blank lines keep the list
                } else {
                    counters.clear();
                }
                out.push(line.to_string());
            }
        }
    }
    out.join("\n")
}

/// Typing `c` over a non-empty selection: wrap it for `* _ ` ~`, else `None`.
pub fn surround(text: &str, sel: Range<usize>, c: char) -> Option<MdEdit> {
    if sel.is_empty() || !matches!(c, '*' | '_' | '`' | '~') {
        return None;
    }
    let s = &text[sel.clone()];
    let new = format!("{c}{s}{c}");
    Some(MdEdit {
        range: sel.clone(),
        caret: sel.start + new.len() - 1,
        text: new,
    })
}

/// Pasting `pasted` over a selection: a URL makes `[selection](url)`.
pub fn paste_link(text: &str, sel: Range<usize>, pasted: &str) -> Option<MdEdit> {
    let p = pasted.trim();
    let is_url =
        (p.starts_with("http://") || p.starts_with("https://")) && !p.contains(char::is_whitespace);
    if sel.is_empty() || !is_url || text[sel.clone()].contains('\n') {
        return None;
    }
    let new = format!("[{}]({p})", &text[sel.clone()]);
    Some(MdEdit {
        range: sel.clone(),
        caret: sel.start + new.len(),
        text: new,
    })
}

/// What Markdown completion applies at `caret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdTrigger {
    /// After an opening ``` — the partial info string.
    FenceInfo { partial: Range<usize> },
    /// After `](` — a relative path.
    LinkPath { partial: Range<usize> },
    /// After `](#` — a heading anchor.
    Anchor { partial: Range<usize> },
}

pub fn trigger_at(text: &str, caret: usize) -> Option<MdTrigger> {
    let lb = line_bounds(text, caret);
    let before = &text[lb.start..caret];
    if let Some(i) = before.rfind("](") {
        let rest = &before[i + 2..];
        if !rest.contains(')') {
            let start = lb.start + i + 2;
            return Some(match rest.strip_prefix('#') {
                Some(_) => MdTrigger::Anchor {
                    partial: start + 1..caret,
                },
                None => MdTrigger::LinkPath {
                    partial: start..caret,
                },
            });
        }
    }
    let t = before.trim_start();
    if let Some(r) = t.strip_prefix("```")
        && !r.contains('`')
        && before.len() - t.len() <= 3
    {
        return Some(MdTrigger::FenceInfo {
            partial: caret - r.len()..caret,
        });
    }
    None
}

/// GitHub-style heading anchors of a document, in order.
pub fn heading_anchors(text: &str) -> Vec<String> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for l in text.lines() {
        if l.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = l.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && l[hashes..].starts_with(' ') {
            let slug: String = l[hashes..]
                .trim()
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
                .map(|c| if c == ' ' { '-' } else { c })
                .collect();
            out.push(slug);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, e: &MdEdit) -> String {
        format!(
            "{}{}{}",
            &text[..e.range.start],
            e.text,
            &text[e.range.end..]
        )
    }

    #[test]
    fn enter_continues_bullets_numbers_tasks_and_quotes() {
        for (src, want) in [
            ("- a", "- a\n- "),
            ("* a", "* a\n* "),
            ("  + a", "  + a\n  + "),
            ("1. a", "1. a\n2. "),
            ("9) a", "9) a\n10) "),
            ("- [x] a", "- [x] a\n- [ ] "),
            ("> q", "> q\n> "),
        ] {
            let e = enter(src, src.len()).unwrap();
            assert_eq!(apply(src, &e), want, "{src}");
            assert_eq!(e.caret, want.len());
        }
    }

    #[test]
    fn enter_on_empty_item_ends_the_list() {
        let src = "- a\n- ";
        let e = enter(src, src.len()).unwrap();
        assert_eq!(apply(src, &e), "- a\n");
        assert_eq!(e.caret, 4);
    }

    #[test]
    fn enter_elsewhere_is_plain() {
        assert!(enter("plain", 5).is_none());
        assert!(enter("- a b", 3).is_none()); // caret mid-line
    }

    #[test]
    fn tab_nests_and_shift_tab_unnests() {
        let e = indent_item("- a\n- b", 7, false, "  ").unwrap();
        assert_eq!(apply("- a\n- b", &e), "- a\n  - b");
        assert_eq!(e.caret, 9);
        let e = indent_item("- a\n  - b", 9, true, "  ").unwrap();
        assert_eq!(apply("- a\n  - b", &e), "- a\n- b");
        assert!(indent_item("- a", 3, true, "  ").is_none());
        assert!(indent_item("text", 4, false, "  ").is_none());
    }

    #[test]
    fn renumber_ordered_lists_per_level() {
        assert_eq!(
            renumber("1. a\n1. b\n  1. c\n  1. d\n1. e"),
            "1. a\n2. b\n  1. c\n  2. d\n3. e"
        );
        assert_eq!(renumber("1. a\ntext\n5. b"), "1. a\ntext\n1. b");
    }

    #[test]
    fn surround_wraps_selection() {
        let e = surround("a word b", 2..6, '*').unwrap();
        assert_eq!(apply("a word b", &e), "a *word* b");
        assert_eq!(e.caret, 7);
        assert!(surround("a", 0..0, '*').is_none());
        assert!(surround("a", 0..1, 'x').is_none());
    }

    #[test]
    fn pasting_a_url_over_a_selection_links_it() {
        let e = paste_link("see docs", 4..8, "https://example.com/x").unwrap();
        assert_eq!(apply("see docs", &e), "see [docs](https://example.com/x)");
        assert!(paste_link("see docs", 4..8, "not a url").is_none());
        assert!(paste_link("see docs", 4..4, "https://e.com").is_none());
    }

    #[test]
    fn completion_triggers() {
        assert_eq!(
            trigger_at("```ht", 5),
            Some(MdTrigger::FenceInfo { partial: 3..5 })
        );
        assert_eq!(
            trigger_at("x\n```", 5),
            Some(MdTrigger::FenceInfo { partial: 5..5 })
        );
        assert_eq!(
            trigger_at("[a](do", 6),
            Some(MdTrigger::LinkPath { partial: 4..6 })
        );
        assert_eq!(
            trigger_at("[a](#he", 7),
            Some(MdTrigger::Anchor { partial: 5..7 })
        );
        assert_eq!(trigger_at("[a](x) text", 11), None);
        assert_eq!(trigger_at("plain", 5), None);
    }

    #[test]
    fn heading_anchors_skip_fences() {
        let t = "# Hello World\n\n```\n# not a heading\n```\n## API: v2\n";
        assert_eq!(heading_anchors(t), ["hello-world", "api-v2"]);
    }
}
