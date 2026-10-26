//! Minimal multi-line code editor (ADR-002): a cursor over a borrowed `String`, key handling,
//! undo, and a highlighted, line-numbered view. The text itself lives in the owning document so
//! forms and "Open as text" edit the same buffer.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Bounds, Div, Hsla, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Stateful, Styled, canvas, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    highlight::{Spans, Tok, highlight, line_runs},
    theme::Theme,
};

pub use crate::highlight::Lang;
use crate::highlight::Lang as L;

pub const LINE_HEIGHT: f32 = 20.0;
pub const GUTTER_WIDTH: f32 = 44.0;
pub const TEXT_LEFT: f32 = GUTTER_WIDTH + 4.0;
/// Advance of one column of the bundled monospace font at 13 px.
pub const CHAR_WIDTH: f32 = 7.8;
const UNDO_LIMIT_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Ignored,
    Moved,
    Edited,
}

/// Cursor and undo history for one editable surface. Offsets are byte offsets into the text the
/// caller passes to [`EditorState::handle_key`].
#[derive(Clone, Debug, Default)]
pub struct EditorState {
    pub cursor: usize,
    goal_col: Option<usize>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
}

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(p, _)| p)
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(s.len(), |c| i + c.len_utf8())
}

pub fn line_start(s: &str, i: usize) -> usize {
    s[..i].rfind('\n').map_or(0, |p| p + 1)
}

pub fn line_end(s: &str, i: usize) -> usize {
    s[i..].find('\n').map_or(s.len(), |p| i + p)
}

/// (line, column in chars) of a byte offset.
pub fn line_col(s: &str, i: usize) -> (usize, usize) {
    let i = i.min(s.len());
    let ls = line_start(s, i);
    (s[..i].matches('\n').count(), s[ls..i].chars().count())
}

/// Byte offset of (line, column in chars), clamped to the line.
pub fn offset_of(s: &str, line: usize, col: usize) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match s[start..].find('\n') {
            Some(p) => start += p + 1,
            None => return s.len(),
        }
    }
    let end = line_end(s, start);
    s[start..end]
        .char_indices()
        .nth(col)
        .map_or(end, |(p, _)| start + p)
}

fn word_left(s: &str, mut i: usize) -> usize {
    while i > 0
        && s[..i]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric())
    {
        i = prev_boundary(s, i);
    }
    while i > 0
        && s[..i]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
    {
        i = prev_boundary(s, i);
    }
    i
}

fn word_right(s: &str, mut i: usize) -> usize {
    while i < s.len() && s[i..].chars().next().is_some_and(|c| !c.is_alphanumeric()) {
        i = next_boundary(s, i);
    }
    while i < s.len() && s[i..].chars().next().is_some_and(char::is_alphanumeric) {
        i = next_boundary(s, i);
    }
    i
}

impl EditorState {
    pub fn at(cursor: usize) -> Self {
        Self {
            cursor,
            ..Self::default()
        }
    }

    pub fn clamp(&mut self, text: &str) {
        self.cursor = self.cursor.min(text.len());
        while !text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }

    fn snapshot(&mut self, text: &str) {
        if text.len() > UNDO_LIMIT_BYTES {
            return;
        }
        self.undo.push((text.to_string(), self.cursor));
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn insert(&mut self, text: &mut String, s: &str) {
        self.snapshot(text);
        self.clamp(text);
        text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.goal_col = None;
    }

    /// Handle one key press. `clipboard` is the system clipboard text, used for paste.
    pub fn handle_key(
        &mut self,
        text: &mut String,
        ev: &KeyDownEvent,
        clipboard: Option<&str>,
    ) -> KeyOutcome {
        self.clamp(text);
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        let cmd = m.platform || m.control;
        if cmd {
            return match (key, m.shift) {
                ("z", false) => self.undo(text),
                ("z", true) | ("y", _) => self.redo(text),
                ("v", _) => match clipboard {
                    Some(c) if !c.is_empty() => {
                        self.insert(text, &c.replace("\r\n", "\n"));
                        KeyOutcome::Edited
                    }
                    _ => KeyOutcome::Ignored,
                },
                _ => KeyOutcome::Ignored,
            };
        }
        match key {
            "left" => {
                self.cursor = if m.alt {
                    word_left(text, self.cursor)
                } else {
                    prev_boundary(text, self.cursor)
                };
                self.goal_col = None;
                KeyOutcome::Moved
            }
            "right" => {
                self.cursor = if m.alt {
                    word_right(text, self.cursor)
                } else {
                    next_boundary(text, self.cursor)
                };
                self.goal_col = None;
                KeyOutcome::Moved
            }
            "up" | "down" => {
                let (line, col) = line_col(text, self.cursor);
                let goal = *self.goal_col.get_or_insert(col);
                let target = if key == "up" {
                    line.saturating_sub(1)
                } else {
                    line + 1
                };
                self.cursor = offset_of(text, target, goal);
                KeyOutcome::Moved
            }
            "home" => {
                self.cursor = line_start(text, self.cursor);
                self.goal_col = None;
                KeyOutcome::Moved
            }
            "end" => {
                self.cursor = line_end(text, self.cursor);
                self.goal_col = None;
                KeyOutcome::Moved
            }
            "backspace" => {
                if self.cursor == 0 {
                    return KeyOutcome::Ignored;
                }
                self.snapshot(text);
                let p = prev_boundary(text, self.cursor);
                text.replace_range(p..self.cursor, "");
                self.cursor = p;
                self.goal_col = None;
                KeyOutcome::Edited
            }
            "delete" => {
                if self.cursor >= text.len() {
                    return KeyOutcome::Ignored;
                }
                self.snapshot(text);
                let n = next_boundary(text, self.cursor);
                text.replace_range(self.cursor..n, "");
                KeyOutcome::Edited
            }
            "enter" => {
                let ls = line_start(text, self.cursor);
                let indent: String = text[ls..self.cursor]
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                self.insert(text, &format!("\n{indent}"));
                KeyOutcome::Edited
            }
            "tab" => {
                self.insert(text, "  ");
                KeyOutcome::Edited
            }
            _ => match ks.key_char.as_deref() {
                Some(s) if !s.is_empty() && !m.alt => {
                    self.insert(text, s);
                    KeyOutcome::Edited
                }
                Some(s) if !s.is_empty() => {
                    self.insert(text, s);
                    KeyOutcome::Edited
                }
                _ => KeyOutcome::Ignored,
            },
        }
    }

    fn undo(&mut self, text: &mut String) -> KeyOutcome {
        let Some((t, c)) = self.undo.pop() else {
            return KeyOutcome::Ignored;
        };
        self.redo.push((std::mem::replace(text, t), self.cursor));
        self.cursor = c;
        KeyOutcome::Edited
    }

    fn redo(&mut self, text: &mut String) -> KeyOutcome {
        let Some((t, c)) = self.redo.pop() else {
            return KeyOutcome::Ignored;
        };
        self.undo.push((std::mem::replace(text, t), self.cursor));
        self.cursor = c;
        KeyOutcome::Edited
    }

    /// Cursor offset for a click at `pos`, given the bounds of the scrolled content element.
    pub fn offset_for_click(text: &str, bounds: Bounds<Pixels>, pos: Point<Pixels>) -> usize {
        let y = f32::from(pos.y - bounds.origin.y).max(0.0);
        let x = f32::from(pos.x - bounds.origin.x) - TEXT_LEFT;
        let line = (y / LINE_HEIGHT) as usize;
        let col = (x / CHAR_WIDTH).round().max(0.0) as usize;
        offset_of(text, line, col)
    }
}

fn color(t: &Theme, tok: Tok) -> Hsla {
    match tok {
        Tok::Plain => t.text,
        Tok::Property => t.syntax_property,
        Tok::String => t.syntax_string,
        Tok::Number => t.syntax_number,
        Tok::Boolean => t.syntax_boolean,
        Tok::Punct => t.syntax_punctuation,
        Tok::Keyword => t.syntax_keyword,
        Tok::Function => t.syntax_function,
        Tok::Type => t.syntax_type,
        Tok::Comment => t.syntax_comment,
        Tok::Heading => t.syntax_function,
        Tok::Variable => t.accent,
    }
}

fn caret(t: &Theme) -> Div {
    div()
        .w(px(1.5))
        .h(px(16.))
        .mx(px(-0.75))
        .flex_none()
        .bg(t.accent)
}

fn line_element(
    t: &Theme,
    number: usize,
    runs: Vec<(String, Tok)>,
    caret_col: Option<usize>,
    highlight_line: bool,
) -> Div {
    let mut children: Vec<AnyElement> = Vec::new();
    let mut col = 0usize;
    let mut placed = false;
    for (s, tok) in runs {
        let n = s.chars().count();
        match caret_col {
            Some(c) if !placed && c >= col && c <= col + n => {
                let split = s.char_indices().nth(c - col).map_or(s.len(), |(p, _)| p);
                let (a, b) = s.split_at(split);
                if !a.is_empty() {
                    children.push(
                        div()
                            .text_color(color(t, tok))
                            .child(a.to_string())
                            .into_any_element(),
                    );
                }
                children.push(caret(t).into_any_element());
                if !b.is_empty() {
                    children.push(
                        div()
                            .text_color(color(t, tok))
                            .child(b.to_string())
                            .into_any_element(),
                    );
                }
                placed = true;
            }
            _ => children.push(div().text_color(color(t, tok)).child(s).into_any_element()),
        }
        col += n;
    }
    if caret_col.is_some() && !placed {
        children.push(caret(t).into_any_element());
    }
    div()
        .flex()
        .flex_row()
        .h(px(LINE_HEIGHT))
        .whitespace_nowrap()
        .when(highlight_line, |d| d.bg(t.element_hover))
        .child(
            div()
                .w(px(GUTTER_WIDTH))
                .pr(px(14.))
                .flex_none()
                .text_right()
                .text_color(t.gutter)
                .child(number.to_string()),
        )
        .children(children)
}

/// Line-numbered, highlighted view of `text`. With `cursor`, draws a caret and records the content
/// element's bounds into `bounds_cell` so clicks can be mapped back to offsets.
pub fn editor_content(
    text: &str,
    spans: &Spans,
    cursor: Option<usize>,
    bounds_cell: Option<Rc<Cell<Bounds<Pixels>>>>,
    t: &Theme,
) -> Div {
    let runs = line_runs(text, spans);
    let caret_pos = cursor.map(|c| line_col(text, c));
    let mut el = div()
        .relative()
        .flex()
        .flex_col()
        .py_2()
        .min_h(px(60.))
        .font_family(BUFFER_FONT)
        .text_size(px(13.))
        .children(runs.into_iter().enumerate().map(|(i, r)| {
            let c = caret_pos.filter(|(l, _)| *l == i).map(|(_, c)| c);
            line_element(t, i + 1, r, c, false)
        }));
    if let Some(cell) = bounds_cell {
        el = el.child(
            canvas(
                move |b, _, _| {
                    // Content origin sits below the container's vertical padding.
                    cell.set(Bounds {
                        origin: Point {
                            x: b.origin.x,
                            y: b.origin.y + px(8.),
                        },
                        size: b.size,
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
    }
    el
}

/// Read-only code surface.
pub fn code_view(text: &str, lang: L, t: &Theme) -> AnyElement {
    let spans = highlight(text, lang);
    editor_content(text, &spans, None, None, t).into_any_element()
}

pub fn scroll_container(id: &'static str) -> Stateful<Div> {
    div().id(id).flex_1().min_h_0().overflow_y_scroll()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str, ch: Option<&str>) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers: Default::default(),
                key: k.into(),
                key_char: ch.map(Into::into),
            },
            is_held: false,
        }
    }

    #[test]
    fn typing_enter_and_backspace() {
        let mut t = String::from("ab");
        let mut e = EditorState::at(2);
        assert_eq!(
            e.handle_key(&mut t, &key("c", Some("c")), None),
            KeyOutcome::Edited
        );
        e.handle_key(&mut t, &key("enter", None), None);
        e.handle_key(&mut t, &key("d", Some("d")), None);
        assert_eq!(t, "abc\nd");
        e.handle_key(&mut t, &key("backspace", None), None);
        e.handle_key(&mut t, &key("backspace", None), None);
        assert_eq!(t, "abc");
    }

    #[test]
    fn enter_keeps_indent() {
        let mut t = String::from("  x");
        let mut e = EditorState::at(3);
        e.handle_key(&mut t, &key("enter", None), None);
        assert_eq!(t, "  x\n  ");
    }

    #[test]
    fn vertical_movement_keeps_column() {
        let mut t = String::from("abcdef\nab\nabcdef");
        let mut e = EditorState::at(5);
        e.handle_key(&mut t, &key("down", None), None);
        assert_eq!(line_col(&t, e.cursor), (1, 2));
        e.handle_key(&mut t, &key("down", None), None);
        assert_eq!(line_col(&t, e.cursor), (2, 5));
    }

    #[test]
    fn undo_and_redo() {
        let mut t = String::from("a");
        let mut e = EditorState::at(1);
        e.handle_key(&mut t, &key("b", Some("b")), None);
        let mut z = key("z", None);
        z.keystroke.modifiers.platform = true;
        e.handle_key(&mut t, &z, None);
        assert_eq!(t, "a");
        z.keystroke.modifiers.shift = true;
        e.handle_key(&mut t, &z, None);
        assert_eq!(t, "ab");
    }

    #[test]
    fn multibyte_cursor_moves_by_char() {
        let mut t = String::from("é");
        let mut e = EditorState::at(2);
        e.handle_key(&mut t, &key("left", None), None);
        assert_eq!(e.cursor, 0);
        e.handle_key(&mut t, &key("delete", None), None);
        assert_eq!(t, "");
    }

    #[test]
    fn offsets_roundtrip() {
        let t = "ab\ncde\n";
        assert_eq!(offset_of(t, 1, 2), 5);
        assert_eq!(line_col(t, 5), (1, 2));
        assert_eq!(offset_of(t, 9, 0), t.len());
    }
}
