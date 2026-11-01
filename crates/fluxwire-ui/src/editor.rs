//! Multi-line code editor (ADR-002, ADR-011): selection, clipboard, coalescing undo and a
//! highlighted, line-numbered view, all on top of [`TextBuffer`]. The text itself lives in the
//! owning document so forms and "Open as text" edit the same buffer.
//!
//! Known gap: these surfaces take typed characters from key events, so IME composition and the
//! emoji picker are not available in them yet (single-line fields use `TextInput`).

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Bounds, Div, Hsla, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Stateful, Styled, canvas, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    highlight::{Spans, Tok, highlight, line_runs},
    input::TextBuffer,
    theme::Theme,
};

pub use crate::highlight::Lang;
use crate::highlight::Lang as L;

pub const LINE_HEIGHT: f32 = 20.0;
pub const GUTTER_WIDTH: f32 = 44.0;
pub const TEXT_LEFT: f32 = GUTTER_WIDTH + 4.0;
/// Advance of one column of the bundled monospace font at 13 px.
pub const CHAR_WIDTH: f32 = 7.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Ignored,
    Moved,
    Edited,
}

/// Cursor, selection and undo history for one editable surface. Offsets are byte offsets into the
/// text the caller passes to [`EditorState::handle_key`]. `cursor` is the selection head.
#[derive(Clone, Debug, Default)]
pub struct EditorState {
    pub cursor: usize,
    pub anchor: usize,
    goal_col: Option<usize>,
    buf: Option<TextBuffer>,
    clip_out: Option<String>,
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

impl EditorState {
    pub fn at(cursor: usize) -> Self {
        Self {
            cursor,
            anchor: cursor,
            ..Self::default()
        }
    }

    /// Collapse the selection onto `off`.
    pub fn set_cursor(&mut self, off: usize) {
        self.cursor = off;
        self.anchor = off;
        self.goal_col = None;
    }

    pub fn clamp(&mut self, text: &str) {
        for p in [&mut self.cursor, &mut self.anchor] {
            *p = (*p).min(text.len());
            while !text.is_char_boundary(*p) {
                *p -= 1;
            }
        }
    }

    /// Ordered (start, end) of the selection; empty when there is none.
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    /// Text a copy or cut put on the clipboard; the caller writes it to the system clipboard.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clip_out.take()
    }

    /// Make the internal buffer mirror `text` and the current selection.
    fn sync(&mut self, text: &str) {
        self.clamp(text);
        let (anchor, head) = (self.anchor, self.cursor);
        let buf = self.buf.get_or_insert_with(|| TextBuffer::new("", true));
        if buf.text() != text {
            buf.set_text(text);
        }
        if (buf.anchor(), buf.head()) != (anchor, head) {
            buf.select(anchor, head);
        }
    }

    fn finish(&mut self, text: &mut String) {
        let Some(buf) = self.buf.as_ref() else { return };
        self.cursor = buf.head();
        self.anchor = buf.anchor();
        if buf.text() != text.as_str() {
            *text = buf.text().to_string();
        }
    }

    pub fn insert(&mut self, text: &mut String, s: &str) {
        self.sync(text);
        if let Some(b) = self.buf.as_mut() {
            b.insert(s);
        }
        self.goal_col = None;
        self.finish(text);
    }

    /// Mouse press at byte offset `off`: caret, extend (Shift), word (double) or line (triple).
    pub fn mouse_down(&mut self, text: &str, off: usize, clicks: usize, shift: bool) {
        self.sync(text);
        let Some(buf) = self.buf.as_mut() else { return };
        match clicks {
            0 | 1 => buf.move_to(off, shift),
            2 => buf.select_word_at(off),
            _ => buf.select_line_at(off),
        }
        self.cursor = buf.head();
        self.anchor = buf.anchor();
        self.goal_col = None;
    }

    /// Mouse drag: extend the selection to `off`.
    pub fn drag_to(&mut self, text: &str, off: usize) {
        self.sync(text);
        let Some(buf) = self.buf.as_mut() else { return };
        buf.move_to(off, true);
        self.cursor = buf.head();
        self.goal_col = None;
    }

    /// Handle one key press. `clipboard` is the system clipboard text, used for paste.
    pub fn handle_key(
        &mut self,
        text: &mut String,
        ev: &KeyDownEvent,
        clipboard: Option<&str>,
    ) -> KeyOutcome {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let key = ks.key.as_str();
        let (shift, word) = (m.shift, m.alt || m.control);
        let cmd = m.platform || m.control;
        self.sync(text);
        let Some(buf) = self.buf.as_mut() else {
            return KeyOutcome::Ignored;
        };
        let mut moved = false;
        let mut goal_reset = true;
        let edited = if cmd && !m.alt {
            match (key, shift) {
                ("z", false) => buf.undo(),
                ("z", true) | ("y", _) => buf.redo(),
                ("a", _) => {
                    buf.select_all();
                    moved = true;
                    false
                }
                ("c", _) => {
                    self.clip_out = buf.copy();
                    self.finish(text);
                    return KeyOutcome::Moved;
                }
                ("x", _) => {
                    self.clip_out = buf.cut();
                    self.finish(text);
                    return KeyOutcome::Edited;
                }
                ("v", _) => match clipboard {
                    Some(c) if !c.is_empty() => buf.paste(c),
                    _ => false,
                },
                ("left", _) if m.platform => {
                    buf.to_line_start(shift);
                    moved = true;
                    false
                }
                ("right", _) if m.platform => {
                    buf.to_line_end(shift);
                    moved = true;
                    false
                }
                ("up", _) if m.platform => {
                    buf.to_doc_start(shift);
                    moved = true;
                    false
                }
                ("down", _) if m.platform => {
                    buf.to_doc_end(shift);
                    moved = true;
                    false
                }
                ("backspace", _) if m.platform => buf.delete_to_line_start(),
                _ => false,
            }
        } else {
            match key {
                "left" | "right" => {
                    let left = key == "left";
                    match (left, word) {
                        (true, true) => buf.word_left_by(shift),
                        (true, false) => buf.left(shift),
                        (false, true) => buf.word_right_by(shift),
                        (false, false) => buf.right(shift),
                    }
                    moved = true;
                    false
                }
                "up" | "down" => {
                    let head = buf.head();
                    let (line, col) = line_col(buf.text(), head);
                    let goal = *self.goal_col.get_or_insert(col);
                    let target = if key == "up" {
                        line.saturating_sub(1)
                    } else {
                        line + 1
                    };
                    let off = offset_of(buf.text(), target, goal);
                    buf.move_to(off, shift);
                    goal_reset = false;
                    moved = true;
                    false
                }
                "home" => {
                    buf.to_line_start(shift);
                    moved = true;
                    false
                }
                "end" => {
                    buf.to_line_end(shift);
                    moved = true;
                    false
                }
                "backspace" if word => buf.delete_word_back(),
                "backspace" => buf.backspace(),
                "delete" if word => buf.delete_word_forward(),
                "delete" => buf.delete(),
                "enter" => {
                    let start = buf.selection().0;
                    let ls = line_start(buf.text(), start);
                    let indent: String = buf.text()[ls..start]
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .collect();
                    buf.insert(&format!("\n{indent}"))
                }
                "tab" => buf.insert("  "),
                _ => match ks.key_char.as_deref() {
                    Some(s) if !s.is_empty() && !cmd => buf.insert(s),
                    _ => false,
                },
            }
        };
        if goal_reset {
            self.goal_col = None;
        }
        self.finish(text);
        if edited {
            KeyOutcome::Edited
        } else if moved {
            KeyOutcome::Moved
        } else {
            KeyOutcome::Ignored
        }
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

pub(crate) fn tok_color(t: &Theme, tok: Tok) -> Hsla {
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
    sel: Option<(usize, usize)>,
    highlight_line: bool,
) -> Div {
    let mut children: Vec<AnyElement> = Vec::new();
    let mut col = 0usize;
    let mut placed = false;
    for (s, tok) in runs {
        let n = s.chars().count();
        let mut cuts: Vec<usize> = caret_col
            .into_iter()
            .chain(sel.into_iter().flat_map(|(a, b)| [a, b]))
            .filter(|c| *c > col && *c < col + n)
            .map(|c| c - col)
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut bounds = vec![0];
        bounds.extend(cuts);
        bounds.push(n);
        for w in bounds.windows(2) {
            let (a, b) = (w[0], w[1]);
            if !placed && caret_col == Some(col + a) {
                children.push(caret(t).into_any_element());
                placed = true;
            }
            let piece: String = s.chars().skip(a).take(b - a).collect();
            if piece.is_empty() {
                continue;
            }
            let selected = sel.is_some_and(|(x, y)| col + a >= x && col + a < y);
            children.push(
                div()
                    .text_color(tok_color(t, tok))
                    .when(selected, |d| d.bg(t.accent_bg()))
                    .child(piece)
                    .into_any_element(),
            );
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

/// Line-numbered, highlighted view of `text`. With `cursor`, draws a caret (and the `selection`
/// range, when non-empty) and records the content element's bounds into `bounds_cell` so clicks
/// can be mapped back to offsets.
pub fn editor_content(
    text: &str,
    spans: &Spans,
    cursor: Option<usize>,
    selection: (usize, usize),
    bounds_cell: Option<Rc<Cell<Bounds<Pixels>>>>,
    t: &Theme,
) -> Div {
    let runs = line_runs(text, spans);
    let caret_pos = cursor.map(|c| line_col(text, c));
    let sel = cursor
        .filter(|_| selection.0 < selection.1)
        .map(|_| (line_col(text, selection.0), line_col(text, selection.1)));
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
            let line_len = r.iter().map(|(s, _)| s.chars().count()).sum::<usize>();
            let s = sel.and_then(|((l0, c0), (l1, c1))| {
                (i >= l0 && i <= l1).then_some({
                    (
                        if i == l0 { c0 } else { 0 },
                        if i == l1 { c1 } else { line_len },
                    )
                })
            });
            line_element(t, i + 1, r, c, s, false)
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
    editor_content(text, &spans, None, (0, 0), None, t).into_any_element()
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

    fn with(k: &str, f: impl Fn(&mut gpui::Modifiers)) -> KeyDownEvent {
        let mut e = key(k, None);
        f(&mut e.keystroke.modifiers);
        e
    }

    #[test]
    fn shift_arrows_select_and_typing_replaces() {
        let mut t = String::from("hello world");
        let mut e = EditorState::at(0);
        for _ in 0..5 {
            e.handle_key(&mut t, &with("right", |m| m.shift = true), None);
        }
        assert_eq!(e.selection(), (0, 5));
        e.handle_key(&mut t, &key("x", Some("x")), None);
        assert_eq!(t, "x world");
        assert_eq!(e.selection(), (1, 1));
    }

    #[test]
    fn copy_cut_paste_through_the_clipboard() {
        let mut t = String::from("abc def");
        let mut e = EditorState::at(0);
        e.handle_key(&mut t, &with("a", |m| m.platform = true), None);
        e.handle_key(&mut t, &with("c", |m| m.platform = true), None);
        assert_eq!(e.take_clipboard().as_deref(), Some("abc def"));
        e.handle_key(&mut t, &with("x", |m| m.platform = true), None);
        assert_eq!(t, "");
        assert_eq!(e.take_clipboard().as_deref(), Some("abc def"));
        e.handle_key(&mut t, &with("v", |m| m.platform = true), Some("a\r\nb"));
        assert_eq!(t, "a\nb");
    }

    #[test]
    fn typing_run_undoes_in_one_step() {
        let mut t = String::new();
        let mut e = EditorState::default();
        for c in ["a", "b", "c"] {
            e.handle_key(&mut t, &key(c, Some(c)), None);
        }
        e.handle_key(&mut t, &with("z", |m| m.platform = true), None);
        assert_eq!(t, "");
    }

    #[test]
    fn mouse_click_variants_and_drag() {
        let t = String::from("foo bar\nbaz");
        let mut e = EditorState::default();
        e.mouse_down(&t, 5, 2, false);
        assert_eq!(e.selection(), (4, 7));
        e.mouse_down(&t, 5, 3, false);
        assert_eq!(e.selection(), (0, 7));
        e.mouse_down(&t, 1, 1, false);
        e.drag_to(&t, 10);
        assert_eq!(e.selection(), (1, 10));
        e.mouse_down(&t, 3, 1, true);
        assert_eq!(e.selection(), (1, 3));
    }

    #[test]
    fn tab_inserts_two_spaces_and_word_delete() {
        let mut t = String::from("foo bar");
        let mut e = EditorState::at(7);
        e.handle_key(&mut t, &with("backspace", |m| m.alt = true), None);
        assert_eq!(t, "foo ");
        e.handle_key(&mut t, &key("tab", None), None);
        assert_eq!(t, "foo   ");
    }
}
