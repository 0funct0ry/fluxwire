//! Pure editing operations of the code editor (M9.06): auto-close, smart indent, Tab, comment
//! toggling, line operations, bracket matching and Home. Each returns a [`Plan`] — one text
//! replacement and the selection afterwards — so the entity applies it as a single undo step
//! and every rule is testable without GPUI.

use std::ops::Range;

use crate::language_config::{Context, LanguageConfig, enter_text, indent_of, indent_unit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub range: Range<usize>,
    pub text: String,
    /// Selection after the edit (absolute offsets in the new text).
    pub anchor: usize,
    pub head: usize,
}

impl Plan {
    fn caret(range: Range<usize>, text: String, caret: usize) -> Self {
        Plan {
            range,
            text,
            anchor: caret,
            head: caret,
        }
    }

    /// A pure caret move (nothing is replaced).
    pub fn is_move(&self) -> bool {
        self.range.is_empty() && self.text.is_empty()
    }

    pub fn apply(&self, src: &str) -> String {
        format!(
            "{}{}{}",
            &src[..self.range.start],
            self.text,
            &src[self.range.end..]
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tabs {
    pub size: usize,
    pub hard: bool,
}

fn line_start(t: &str, at: usize) -> usize {
    t[..at].rfind('\n').map_or(0, |i| i + 1)
}
fn line_end(t: &str, at: usize) -> usize {
    t[at..].find('\n').map_or(t.len(), |i| at + i)
}

/// Whole lines touched by a selection: `(start of first line, end of last line)`. A selection
/// ending at the start of a line does not include that line.
fn line_span(t: &str, sel: &Range<usize>) -> Range<usize> {
    let s = line_start(t, sel.start);
    let end_ref = if sel.end > sel.start && t[..sel.end].ends_with('\n') {
        sel.end - 1
    } else {
        sel.end
    };
    s..line_end(t, end_ref)
}

/// Typing `c` with the given selection. `None` means "insert the character normally".
pub fn type_char(
    cfg: &LanguageConfig,
    t: &str,
    sel: Range<usize>,
    c: char,
    ctx: Context,
    auto_close: bool,
) -> Option<Plan> {
    if !auto_close {
        return None;
    }
    // Selection surround.
    if !sel.is_empty() {
        let close = cfg.surrounds(c)?;
        let inner = &t[sel.clone()];
        let text = format!("{c}{inner}{close}");
        return Some(Plan {
            anchor: sel.start + c.len_utf8(),
            head: sel.start + c.len_utf8() + inner.len(),
            range: sel,
            text,
        });
    }
    let at = sel.start;
    let next = t[at..].chars().next();
    let prev = t[..at].chars().last();
    // Step over a closer already there.
    if next == Some(c) && cfg.is_closer(c) {
        return Some(Plan::caret(at..at, String::new(), at + c.len_utf8()));
    }
    // Dedent a closer typed on a whitespace-only line.
    if cfg.dedents_on(c) {
        let ls = line_start(t, at);
        let before = &t[ls..at];
        if !before.is_empty() && before.trim().is_empty() {
            let new_indent = before
                .strip_suffix('\t')
                .or_else(|| before.strip_suffix("  "))
                .unwrap_or(before.trim_end_matches(' '))
                .to_string();
            let text = format!("{new_indent}{c}");
            let caret = ls + text.len();
            return Some(Plan::caret(ls..at, text, caret));
        }
    }
    if cfg.should_auto_close(c, prev, next, ctx) {
        let close = cfg.pair_for_open(c)?.close;
        return Some(Plan::caret(
            at..at,
            format!("{c}{close}"),
            at + c.len_utf8(),
        ));
    }
    None
}

/// Backspace with a collapsed caret between an empty pair deletes both characters.
pub fn backspace_pair(cfg: &LanguageConfig, t: &str, caret: usize) -> Option<Plan> {
    let prev = t[..caret].chars().last()?;
    let next = t[caret..].chars().next()?;
    let p = cfg.pair_for_open(prev)?;
    (p.close == next).then(|| {
        Plan::caret(
            caret - prev.len_utf8()..caret + next.len_utf8(),
            String::new(),
            caret - prev.len_utf8(),
        )
    })
}

/// Enter: replaces the selection with a newline plus indentation.
pub fn enter(cfg: &LanguageConfig, t: &str, sel: Range<usize>, tabs: Tabs) -> Plan {
    let t2 = format!("{}{}", &t[..sel.start], &t[sel.end..]);
    let (ins, caret) = enter_text(cfg, &t2, sel.start, tabs.size, tabs.hard);
    let caret = sel.start + caret;
    Plan::caret(sel, ins, caret)
}

/// Tab / Shift+Tab.
pub fn tab(t: &str, sel: Range<usize>, outdent: bool, tabs: Tabs) -> Plan {
    let unit = indent_unit(tabs.size, tabs.hard);
    let multi = t[sel.clone()].contains('\n');
    if !multi && !outdent && sel.is_empty() {
        // insert spaces to the next tab stop (or a tab)
        let col = t[line_start(t, sel.start)..sel.start]
            .chars()
            .fold(0, |c, ch| {
                if ch == '\t' {
                    c + tabs.size - c % tabs.size
                } else {
                    c + 1
                }
            });
        let ins = if tabs.hard {
            "\t".to_string()
        } else {
            " ".repeat(tabs.size - col % tabs.size)
        };
        let caret = sel.start + ins.len();
        return Plan::caret(sel, ins, caret);
    }
    let span = line_span(t, &sel);
    let lines: Vec<String> = t[span.clone()]
        .split('\n')
        .map(|l| {
            if outdent {
                if let Some(r) = l.strip_prefix('\t') {
                    r.to_string()
                } else {
                    let n = l.len() - l.trim_start_matches(' ').len();
                    l[n.min(tabs.size)..].to_string()
                }
            } else if l.is_empty() {
                String::new()
            } else {
                format!("{unit}{l}")
            }
        })
        .collect();
    let text = lines.join("\n");
    let delta_first = text.split('\n').next().map_or(0, str::len) as isize
        - t[span.clone()].split('\n').next().map_or(0, str::len) as isize;
    let end = span.start + text.len();
    let (a, h) = if sel.is_empty() {
        let c = (sel.start as isize + delta_first).max(span.start as isize) as usize;
        (c, c)
    } else {
        (span.start, end)
    };
    Plan {
        range: span,
        text,
        anchor: a,
        head: h,
    }
}

/// Comment toggle: line comments per line, or a block comment around the selection/line.
pub fn toggle_comment(cfg: &LanguageConfig, t: &str, sel: Range<usize>) -> Option<Plan> {
    let span = line_span(t, &sel);
    if let Some(tok) = cfg.line_comment {
        let lines: Vec<&str> = t[span.clone()].split('\n').collect();
        let content: Vec<&&str> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
        let all = !content.is_empty() && content.iter().all(|l| l.trim_start().starts_with(tok));
        let min_indent = content
            .iter()
            .map(|l| indent_of(l).len())
            .min()
            .unwrap_or(0);
        let out: Vec<String> = lines
            .iter()
            .map(|l| {
                if l.trim().is_empty() {
                    (*l).to_string()
                } else if all {
                    let i = indent_of(l).len();
                    let rest = &l[i + tok.len()..];
                    format!("{}{}", &l[..i], rest.strip_prefix(' ').unwrap_or(rest))
                } else {
                    format!("{}{tok} {}", &l[..min_indent], &l[min_indent..])
                }
            })
            .collect();
        let text = out.join("\n");
        let end = span.start + text.len();
        let (a, h) = if sel.is_empty() {
            let d = text.len() as isize - t[span.clone()].len() as isize;
            let c = (sel.start as isize + d).clamp(span.start as isize, end as isize) as usize;
            (c, c)
        } else {
            (span.start, end)
        };
        return Some(Plan {
            range: span,
            text,
            anchor: a,
            head: h,
        });
    }
    let (open, close) = cfg.block_comment?;
    let target = if sel.is_empty() {
        span.clone()
    } else {
        sel.clone()
    };
    let s = &t[target.clone()];
    let trimmed = s.trim();
    let text = if trimmed.starts_with(open) && trimmed.ends_with(close) {
        let lead = s.len() - s.trim_start().len();
        let inner = &trimmed[open.len()..trimmed.len() - close.len()];
        let inner = inner.strip_prefix(' ').unwrap_or(inner);
        let inner = inner.strip_suffix(' ').unwrap_or(inner);
        format!(
            "{}{inner}{}",
            &s[..lead],
            &s[s.len() - (s.len() - s.trim_end().len())..]
        )
    } else {
        let lead = s.len() - s.trim_start().len();
        format!(
            "{}{open} {} {close}{}",
            &s[..lead],
            s.trim(),
            &s[s.trim_end().len()..]
        )
    };
    let caret = target.start + text.len();
    Some(Plan {
        range: target.clone(),
        text,
        anchor: if sel.is_empty() { caret } else { target.start },
        head: caret,
    })
}

/// Alt+Up / Alt+Down: moves the selected lines. `None` at the document edge.
pub fn move_lines(t: &str, sel: Range<usize>, up: bool) -> Option<Plan> {
    let span = line_span(t, &sel);
    if up {
        if span.start == 0 {
            return None;
        }
        let prev_start = line_start(t, span.start - 1);
        let prev = &t[prev_start..span.start - 1];
        let block = &t[span.clone()];
        let text = format!("{block}\n{prev}");
        let d = prev.len() + 1;
        Some(Plan {
            range: prev_start..span.end,
            text,
            anchor: sel.start - d,
            head: sel.end - d,
        })
    } else {
        if span.end >= t.len() {
            return None;
        }
        let next_end = line_end(t, span.end + 1);
        let next = &t[span.end + 1..next_end];
        let block = &t[span.clone()];
        let text = format!("{next}\n{block}");
        let d = next.len() + 1;
        Some(Plan {
            range: span.start..next_end,
            text,
            anchor: sel.start + d,
            head: sel.end + d,
        })
    }
}

/// Duplicates the selected lines below (or the selection itself when it is within a line).
pub fn duplicate(t: &str, sel: Range<usize>) -> Plan {
    if !sel.is_empty() && !t[sel.clone()].contains('\n') {
        let s = &t[sel.clone()];
        return Plan {
            range: sel.end..sel.end,
            text: s.to_string(),
            anchor: sel.end,
            head: sel.end + s.len(),
        };
    }
    let span = line_span(t, &sel);
    let block = &t[span.clone()];
    let d = block.len() + 1;
    Plan {
        range: span.end..span.end,
        text: format!("\n{block}"),
        anchor: sel.start + d,
        head: sel.end + d,
    }
}

/// Deletes the selected lines.
pub fn delete_lines(t: &str, sel: Range<usize>) -> Plan {
    let span = line_span(t, &sel);
    let (s, e) = if span.end < t.len() {
        (span.start, span.end + 1)
    } else if span.start > 0 {
        (span.start - 1, span.end)
    } else {
        (0, span.end)
    };
    let caret = s.min(line_end(
        &format!("{}{}", &t[..s], &t[e..]),
        s.min(t.len() - (e - s)),
    ));
    Plan::caret(s..e, String::new(), caret.min(s))
}

/// Home: first non-blank character, or column 0 when already there.
pub fn home(t: &str, caret: usize) -> usize {
    let ls = line_start(t, caret);
    let first = ls + indent_of(&t[ls..line_end(t, ls)]).len();
    if caret == first { ls } else { first }
}

/// Matching bracket for the one at or just before `caret` (checks the character after the
/// caret first, then the one before). `skip(offset)` is true for offsets inside strings or
/// comments (from the syntax tree). Returns `(bracket, partner)`.
pub fn matching_bracket(
    cfg: &LanguageConfig,
    t: &str,
    caret: usize,
    skip: &dyn Fn(usize) -> bool,
) -> Option<(usize, usize)> {
    let cands = [
        t[caret..].chars().next().map(|c| (caret, c)),
        t[..caret].chars().last().map(|c| (caret - c.len_utf8(), c)),
    ];
    for (pos, c) in cands.into_iter().flatten() {
        let Some((other, forward)) = cfg.matching_bracket(c) else {
            continue;
        };
        if skip(pos) {
            continue;
        }
        let mut depth = 0i32;
        if forward {
            for (o, ch) in t[pos..].char_indices() {
                let o = pos + o;
                if skip(o) {
                    continue;
                }
                if ch == c {
                    depth += 1;
                } else if ch == other {
                    depth -= 1;
                    if depth == 0 {
                        return Some((pos, o));
                    }
                }
            }
        } else {
            for (o, ch) in t[..=pos].char_indices().rev() {
                if skip(o) {
                    continue;
                }
                if ch == c {
                    depth += 1;
                } else if ch == other {
                    depth -= 1;
                    if depth == 0 {
                        return Some((pos, o));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Lang;

    const T2: Tabs = Tabs {
        size: 2,
        hard: false,
    };

    fn json() -> LanguageConfig {
        LanguageConfig::for_lang(Lang::Json)
    }
    fn ts() -> LanguageConfig {
        LanguageConfig::for_lang(Lang::Ts)
    }
    /// Applies a plan and returns the text with `|` at the caret.
    fn run(src: &str, p: &Plan) -> String {
        let n = p.apply(src);
        format!("{}|{}", &n[..p.head], &n[p.head..])
    }

    #[test]
    fn opener_inserts_closer_with_caret_between() {
        let p = type_char(&json(), "", 0..0, '{', Context::Code, true).unwrap();
        assert_eq!(run("", &p), "{|}");
        assert!(type_char(&json(), "", 0..0, '{', Context::Code, false).is_none());
    }

    #[test]
    fn closer_steps_over() {
        let p = type_char(&json(), "{}", 1..1, '}', Context::Code, true).unwrap();
        assert!(p.is_move());
        assert_eq!(p.head, 2);
    }

    #[test]
    fn quote_inside_string_is_plain() {
        assert!(
            type_char(&json(), "\"ab\"", 3..3, '"', Context::String, true)
                .is_some_and(|p| p.is_move())
        );
        assert!(type_char(&json(), "\"a", 2..2, '"', Context::String, true).is_none());
    }

    #[test]
    fn selection_surrounds() {
        let p = type_char(&ts(), "say hi", 4..6, '"', Context::Code, true).unwrap();
        assert_eq!(p.apply("say hi"), "say \"hi\"");
        assert_eq!((p.anchor, p.head), (5, 7));
        assert!(type_char(&ts(), "say hi", 4..6, 'x', Context::Code, true).is_none());
    }

    #[test]
    fn backspace_deletes_empty_pair() {
        let p = backspace_pair(&json(), "[]", 1).unwrap();
        assert_eq!(p.apply("[]"), "");
        assert!(backspace_pair(&json(), "[x]", 1).is_none());
    }

    #[test]
    fn closer_on_blank_line_dedents() {
        let p = type_char(&json(), "{\n  ", 4..4, '}', Context::Code, true).unwrap();
        assert_eq!(p.apply("{\n  "), "{\n}");
    }

    #[test]
    fn enter_between_braces() {
        let p = enter(&json(), "{}", 1..1, T2);
        assert_eq!(run("{}", &p), "{\n  |\n}");
    }

    #[test]
    fn tab_inserts_to_next_stop_or_tab() {
        let p = tab(
            "ab",
            2..2,
            false,
            Tabs {
                size: 4,
                hard: false,
            },
        );
        assert_eq!(p.apply("ab"), "ab  ");
        let p = tab(
            "ab",
            2..2,
            false,
            Tabs {
                size: 4,
                hard: true,
            },
        );
        assert_eq!(p.apply("ab"), "ab\t");
    }

    #[test]
    fn tab_indents_and_outdents_selected_lines() {
        let src = "a\nb\n\nc";
        let p = tab(src, 0..src.len(), false, T2);
        assert_eq!(p.apply(src), "  a\n  b\n\n  c");
        let out = p.apply(src);
        let q = tab(&out, 0..out.len(), true, T2);
        assert_eq!(q.apply(&out), src);
        let single = tab("  x", 3..3, true, T2);
        assert_eq!(single.apply("  x"), "x");
        assert_eq!(single.head, 1);
    }

    #[test]
    fn comment_toggle_round_trip() {
        let src = "  a\n  b";
        let p = toggle_comment(&ts(), src, 0..src.len()).unwrap();
        assert_eq!(p.apply(src), "  // a\n  // b");
        let on = p.apply(src);
        let q = toggle_comment(&ts(), &on, 0..on.len()).unwrap();
        assert_eq!(q.apply(&on), src);
    }

    #[test]
    fn comment_toggle_is_nothing_in_json_and_blocks_in_xml() {
        assert!(toggle_comment(&json(), "{}", 0..0).is_none());
        let x = LanguageConfig::for_lang(Lang::Xml);
        let p = toggle_comment(&x, "<a/>", 0..0).unwrap();
        assert_eq!(p.apply("<a/>"), "<!-- <a/> -->");
        let on = p.apply("<a/>");
        assert_eq!(toggle_comment(&x, &on, 0..0).unwrap().apply(&on), "<a/>");
    }

    #[test]
    fn move_lines_swaps_and_keeps_selection() {
        let src = "a\nb\nc";
        let p = move_lines(src, 2..3, true).unwrap();
        assert_eq!(p.apply(src), "b\na\nc");
        assert_eq!((p.anchor, p.head), (0, 1));
        let p = move_lines(src, 2..3, false).unwrap();
        assert_eq!(p.apply(src), "a\nc\nb");
        assert_eq!((p.anchor, p.head), (4, 5));
        assert!(move_lines(src, 0..0, true).is_none());
        assert!(move_lines(src, 4..4, false).is_none());
    }

    #[test]
    fn duplicate_lines_and_selection() {
        let p = duplicate("a\nb", 0..0);
        assert_eq!(p.apply("a\nb"), "a\na\nb");
        assert_eq!(p.head, 2);
        let q = duplicate("hello", 1..3);
        assert_eq!(q.apply("hello"), "helello");
    }

    #[test]
    fn delete_lines_cases() {
        assert_eq!(delete_lines("a\nb\nc", 2..2).apply("a\nb\nc"), "a\nc");
        assert_eq!(delete_lines("a\nb", 2..2).apply("a\nb"), "a");
        assert_eq!(delete_lines("a", 0..0).apply("a"), "");
        assert_eq!(delete_lines("a\nb\nc", 0..3).apply("a\nb\nc"), "c");
    }

    #[test]
    fn home_toggles() {
        let t = "  ab";
        assert_eq!(home(t, 4), 2);
        assert_eq!(home(t, 2), 0);
        assert_eq!(home(t, 0), 2);
    }

    #[test]
    fn bracket_matching_respects_skip() {
        let t = r#"{"a": "}", "b": [1]}"#;
        let in_str = |o: usize| (6..9).contains(&o);
        assert_eq!(matching_bracket(&json(), t, 0, &|_| false), Some((0, 7)));
        assert_eq!(
            matching_bracket(&json(), t, 0, &in_str),
            Some((0, t.len() - 1))
        );
        assert_eq!(
            matching_bracket(&json(), t, t.len(), &in_str),
            Some((t.len() - 1, 0))
        );
        assert_eq!(matching_bracket(&json(), "abc", 1, &|_| false), None);
    }
}
