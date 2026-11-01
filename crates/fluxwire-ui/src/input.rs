//! The one text-editing core (M9.02, SPEC §17.11).
//!
//! `TextBuffer` is pure logic: text, a selection, grapheme- and word-aware movement,
//! edits and a coalescing undo stack. Offsets are byte offsets that always sit on grapheme
//! boundaries. Views (the single-line input and the code editor) render and route events;
//! they never re-implement caret or selection logic.

use std::time::{Duration, Instant};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// Same cap as the code editor's undo history.
const UNDO_LIMIT_BYTES: usize = 1024 * 1024;
const UNDO_LIMIT_STEPS: usize = 200;
/// A pause this long ends the current undo group.
pub const GROUP_PAUSE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Backspace,
    Delete,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    anchor: usize,
    head: usize,
}

#[derive(Clone, Debug)]
pub struct TextBuffer {
    text: String,
    anchor: usize,
    head: usize,
    multiline: bool,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_kind: Option<EditKind>,
    last_edit: Option<Instant>,
    revision: u64,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new("", false)
    }
}

/// Word characters; everything else (including `{{`, `}}`, `.`, `-`, `/`) separates words.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Normalise text going into a buffer. Single-line: each CR/LF run becomes one space and other
/// control characters are dropped. Multi-line: CRLF/CR become LF, tabs and LF stay.
pub fn sanitize(s: &str, multiline: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_break = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' if multiline => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\r' | '\n' => {
                if !in_break {
                    out.push(' ');
                }
                in_break = true;
                continue;
            }
            '\t' if multiline => out.push('\t'),
            c if c.is_control() => {}
            c => out.push(c),
        }
        in_break = false;
    }
    out
}

impl TextBuffer {
    pub fn new(text: &str, multiline: bool) -> Self {
        let text = sanitize(text, multiline);
        let end = text.len();
        Self {
            text,
            anchor: end,
            head: end,
            multiline,
            undo: Vec::new(),
            redo: Vec::new(),
            last_kind: None,
            last_edit: None,
            revision: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }
    pub fn head(&self) -> usize {
        self.head
    }
    pub fn anchor(&self) -> usize {
        self.anchor
    }
    /// Bumped by every change to the text (not by caret moves); lets owners skip no-op events.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn has_selection(&self) -> bool {
        self.anchor != self.head
    }
    /// Selection as (start, end), start <= end.
    pub fn selection(&self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
    pub fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replace the whole text (not an undoable edit); the caret goes to the end.
    pub fn set_text(&mut self, text: &str) {
        self.text = sanitize(text, self.multiline);
        self.anchor = self.text.len();
        self.head = self.text.len();
        self.undo.clear();
        self.redo.clear();
        self.last_kind = None;
        self.revision += 1;
    }

    /// Ends the current undo group (caret move, paste, cut, blur).
    pub fn break_group(&mut self) {
        self.last_kind = None;
    }

    // ---- boundaries ----

    /// Nearest grapheme boundary at or before `idx`.
    fn clamp(&self, idx: usize) -> usize {
        let mut i = idx.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        let mut c = GraphemeCursor::new(i, self.text.len(), true);
        if c.is_boundary(&self.text, 0).unwrap_or(true) {
            i
        } else {
            self.prev_boundary(i)
        }
    }

    pub fn snap(&self, idx: usize) -> usize {
        self.clamp(idx)
    }

    pub fn prev_boundary(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }
        let mut c = GraphemeCursor::new(idx, self.text.len(), true);
        c.prev_boundary(&self.text, 0).ok().flatten().unwrap_or(0)
    }

    pub fn next_boundary(&self, idx: usize) -> usize {
        if idx >= self.text.len() {
            return self.text.len();
        }
        let mut c = GraphemeCursor::new(idx, self.text.len(), true);
        c.next_boundary(&self.text, 0)
            .ok()
            .flatten()
            .unwrap_or(self.text.len())
    }

    pub fn word_left(&self, idx: usize) -> usize {
        let mut i = idx;
        let before = |i: usize| self.text[..i].chars().next_back();
        while let Some(c) = before(i) {
            if is_word(c) {
                break;
            }
            i -= c.len_utf8();
        }
        while let Some(c) = before(i) {
            if !is_word(c) {
                break;
            }
            i -= c.len_utf8();
        }
        i
    }

    pub fn word_right(&self, idx: usize) -> usize {
        let mut i = idx;
        let after = |i: usize| self.text[i..].chars().next();
        while let Some(c) = after(i) {
            if is_word(c) {
                break;
            }
            i += c.len_utf8();
        }
        while let Some(c) = after(i) {
            if !is_word(c) {
                break;
            }
            i += c.len_utf8();
        }
        i
    }

    pub fn line_start(&self, idx: usize) -> usize {
        self.text[..idx].rfind('\n').map_or(0, |p| p + 1)
    }

    pub fn line_end(&self, idx: usize) -> usize {
        self.text[idx..]
            .find('\n')
            .map_or(self.text.len(), |p| idx + p)
    }

    /// Word (or run of separators, or the grapheme) around `idx`.
    pub fn word_range(&self, idx: usize) -> (usize, usize) {
        let idx = self.clamp(idx);
        let on_word = self.text[idx..].chars().next().is_some_and(is_word);
        let before_word = self.text[..idx].chars().next_back().is_some_and(is_word);
        if !on_word && !before_word {
            // Separator: select the grapheme under the pointer (or before it at the end).
            let end = self.next_boundary(idx);
            return if end > idx {
                (idx, end)
            } else {
                (self.prev_boundary(idx), idx)
            };
        }
        let mut s = idx;
        while let Some(c) = self.text[..s].chars().next_back() {
            if !is_word(c) {
                break;
            }
            s -= c.len_utf8();
        }
        let mut e = idx;
        while let Some(c) = self.text[e..].chars().next() {
            if !is_word(c) {
                break;
            }
            e += c.len_utf8();
        }
        (s, e)
    }

    // ---- selection and movement ----

    /// Move the head to `idx`; `extend` keeps the anchor.
    pub fn move_to(&mut self, idx: usize, extend: bool) {
        self.head = self.clamp(idx);
        if !extend {
            self.anchor = self.head;
        }
        self.break_group();
    }

    pub fn select(&mut self, anchor: usize, head: usize) {
        self.anchor = self.clamp(anchor);
        self.head = self.clamp(head);
        self.break_group();
    }

    pub fn select_all(&mut self) {
        self.select(0, self.text.len());
    }

    pub fn select_word_at(&mut self, idx: usize) {
        let (s, e) = self.word_range(idx);
        self.select(s, e);
    }

    /// Triple click: the line (multi-line) or everything (single-line).
    pub fn select_line_at(&mut self, idx: usize) {
        if !self.multiline {
            return self.select_all();
        }
        let idx = self.clamp(idx);
        let s = self.line_start(idx);
        let e = self.line_end(idx);
        self.select(s, e);
    }

    /// Plain Left collapses to the selection start; otherwise moves one grapheme.
    pub fn left(&mut self, extend: bool) {
        if self.has_selection() && !extend {
            let (s, _) = self.selection();
            return self.move_to(s, false);
        }
        self.move_to(self.prev_boundary(self.head), extend);
    }

    pub fn right(&mut self, extend: bool) {
        if self.has_selection() && !extend {
            let (_, e) = self.selection();
            return self.move_to(e, false);
        }
        self.move_to(self.next_boundary(self.head), extend);
    }

    pub fn word_left_by(&mut self, extend: bool) {
        self.move_to(self.word_left(self.head), extend);
    }
    pub fn word_right_by(&mut self, extend: bool) {
        self.move_to(self.word_right(self.head), extend);
    }
    pub fn to_line_start(&mut self, extend: bool) {
        self.move_to(self.line_start(self.head), extend);
    }
    pub fn to_line_end(&mut self, extend: bool) {
        self.move_to(self.line_end(self.head), extend);
    }
    pub fn to_doc_start(&mut self, extend: bool) {
        self.move_to(0, extend);
    }
    pub fn to_doc_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    // ---- edits ----

    fn push_undo(&mut self, kind: EditKind) {
        let fresh = self.last_kind != Some(kind)
            || kind == EditKind::Other
            || self.last_edit.is_none_or(|t| t.elapsed() >= GROUP_PAUSE);
        if fresh {
            self.undo.push(Snapshot {
                text: self.text.clone(),
                anchor: self.anchor,
                head: self.head,
            });
            let mut total: usize = self.undo.iter().map(|s| s.text.len()).sum();
            while self.undo.len() > UNDO_LIMIT_STEPS
                || (total > UNDO_LIMIT_BYTES && self.undo.len() > 1)
            {
                total -= self.undo.remove(0).text.len();
            }
        }
        self.redo.clear();
        self.last_kind = Some(kind);
        self.last_edit = Some(Instant::now());
    }

    fn replace_range(&mut self, start: usize, end: usize, with: &str, kind: EditKind) {
        self.push_undo(kind);
        self.text.replace_range(start..end, with);
        self.revision += 1;
        self.head = start + with.len();
        self.anchor = self.head;
    }

    /// Insert (typing, IME commit, paste); replaces the selection.
    pub fn insert(&mut self, s: &str) -> bool {
        self.insert_kind(s, EditKind::Typing)
    }

    fn insert_kind(&mut self, s: &str, kind: EditKind) -> bool {
        let s = sanitize(s, self.multiline);
        if s.is_empty() && !self.has_selection() {
            return false;
        }
        if self.text.len() - (self.selection().1 - self.selection().0) + s.len() > UNDO_LIMIT_BYTES
        {
            return false;
        }
        let (a, b) = self.selection();
        self.replace_range(a, b, &s, kind);
        true
    }

    pub fn paste(&mut self, s: &str) -> bool {
        self.break_group();
        let changed = self.insert_kind(s, EditKind::Other);
        self.break_group();
        changed
    }

    pub fn backspace(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection(EditKind::Backspace);
        }
        if self.head == 0 {
            return false;
        }
        let s = self.prev_boundary(self.head);
        self.replace_range(s, self.head, "", EditKind::Backspace);
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection(EditKind::Delete);
        }
        if self.head >= self.text.len() {
            return false;
        }
        let e = self.next_boundary(self.head);
        self.replace_range(self.head, e, "", EditKind::Delete);
        true
    }

    pub fn delete_word_back(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection(EditKind::Other);
        }
        let s = self.word_left(self.head);
        if s == self.head {
            return false;
        }
        self.replace_range(s, self.head, "", EditKind::Other);
        true
    }

    pub fn delete_word_forward(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection(EditKind::Other);
        }
        let e = self.word_right(self.head);
        if e == self.head {
            return false;
        }
        self.replace_range(self.head, e, "", EditKind::Other);
        true
    }

    pub fn delete_to_line_start(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection(EditKind::Other);
        }
        let s = self.line_start(self.head);
        if s == self.head {
            return false;
        }
        self.replace_range(s, self.head, "", EditKind::Other);
        true
    }

    fn delete_selection(&mut self, kind: EditKind) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        self.replace_range(a, b, "", kind);
        true
    }

    /// Delete the selection (the context-menu "Delete").
    pub fn delete_selected(&mut self) -> bool {
        self.break_group();
        let r = self.delete_selection(EditKind::Other);
        self.break_group();
        r
    }

    /// Copy: `None` for an empty selection (never the whole field).
    pub fn copy(&self) -> Option<String> {
        self.has_selection()
            .then(|| self.selected_text().to_string())
    }

    pub fn cut(&mut self) -> Option<String> {
        let s = self.copy()?;
        self.break_group();
        self.delete_selection(EditKind::Other);
        self.break_group();
        Some(s)
    }

    /// Replace `start..end` (byte range) and leave the caret after it. Used by IME.
    pub fn replace_range_with(&mut self, start: usize, end: usize, with: &str) {
        let (a, b) = (self.clamp(start), self.clamp(end));
        let (a, b) = (a.min(b), a.max(b));
        let s = sanitize(with, self.multiline);
        if a == b && s.is_empty() {
            return;
        }
        self.replace_range(a, b, &s, EditKind::Typing);
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            text: std::mem::replace(&mut self.text, prev.text),
            anchor: self.anchor,
            head: self.head,
        });
        self.anchor = prev.anchor;
        self.head = prev.head;
        self.last_kind = None;
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            text: std::mem::replace(&mut self.text, next.text),
            anchor: self.anchor,
            head: self.head,
        });
        self.anchor = next.anchor;
        self.head = next.head;
        self.last_kind = None;
        self.revision += 1;
        true
    }

    /// Number of grapheme clusters (for tests and masked drawing).
    pub fn grapheme_count(&self) -> usize {
        self.text.graphemes(true).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        TextBuffer::new(s, false)
    }

    #[test]
    fn typing_replaces_selection_and_moves_caret() {
        let mut b = buf("hello");
        b.select_all();
        b.insert("yo");
        assert_eq!(b.text(), "yo");
        assert_eq!(b.head(), 2);
    }

    #[test]
    fn left_right_collapse_selection() {
        let mut b = buf("abcd");
        b.select(1, 3);
        b.left(false);
        assert_eq!((b.head(), b.anchor()), (1, 1));
        b.select(1, 3);
        b.right(false);
        assert_eq!((b.head(), b.anchor()), (3, 3));
    }

    #[test]
    fn shift_extends() {
        let mut b = buf("abcd");
        b.move_to(1, false);
        b.right(true);
        b.right(true);
        assert_eq!(b.selection(), (1, 3));
        assert_eq!(b.selected_text(), "bc");
    }

    #[test]
    fn graphemes_are_never_split() {
        for s in ["é", "e\u{301}", "👩‍👩‍👧", "日"] {
            let mut b = buf(&format!("a{s}b"));
            b.move_to(1, false);
            b.right(false);
            assert_eq!(b.head(), 1 + s.len(), "right over {s}");
            b.left(false);
            assert_eq!(b.head(), 1, "left over {s}");
            b.move_to(1 + s.len(), false);
            b.backspace();
            assert_eq!(b.text(), "ab", "backspace {s}");
        }
        let mut b = buf("a👩‍👩‍👧b");
        b.move_to(1, false);
        b.delete();
        assert_eq!(b.text(), "ab");
    }

    #[test]
    fn word_boundaries_around_variables() {
        let b = buf("{{baseUrl}}/v1");
        let end = b.text().len();
        let v = b.word_left(end);
        assert_eq!(&b.text()[v..], "v1");
        let base = b.word_left(v);
        assert_eq!(&b.text()[base..], "baseUrl}}/v1");
        assert_eq!(b.word_right(0), "{{baseUrl".len());
        assert_eq!(b.word_right("{{baseUrl".len()), b.text().len());
        let b = buf("a_b-c");
        assert_eq!(b.word_right(0), 3);
    }

    #[test]
    fn select_word_at() {
        let mut b = buf("foo.bar baz");
        b.select_word_at(5);
        assert_eq!(b.selected_text(), "bar");
        b.select_word_at(3);
        assert_eq!(b.selected_text(), "foo");
    }

    #[test]
    fn line_and_doc_movement_multiline() {
        let mut b = TextBuffer::new("one\ntwo\nthree", true);
        b.move_to(5, false);
        b.to_line_start(false);
        assert_eq!(b.head(), 4);
        b.to_line_end(true);
        assert_eq!(b.selected_text(), "two");
        b.to_doc_start(false);
        assert_eq!(b.head(), 0);
        b.to_doc_end(false);
        assert_eq!(b.head(), 13);
        b.select_line_at(5);
        assert_eq!(b.selected_text(), "two");
        let mut s = buf("abc");
        s.select_line_at(1);
        assert_eq!(s.selected_text(), "abc");
    }

    #[test]
    fn delete_variants() {
        let mut b = buf("foo bar baz");
        assert!(b.delete_word_back());
        assert_eq!(b.text(), "foo bar ");
        b.move_to(0, false);
        assert!(b.delete_word_forward());
        assert_eq!(b.text(), " bar ");
        b.move_to(3, false);
        assert!(b.delete_to_line_start());
        assert_eq!(b.text(), "r ");
        assert!(!buf("").backspace());
        assert!(!buf("").delete());
    }

    #[test]
    fn copy_cut_need_a_selection() {
        let mut b = buf("hello");
        assert_eq!(b.copy(), None);
        assert_eq!(b.cut(), None);
        assert_eq!(b.text(), "hello");
        b.select(0, 2);
        assert_eq!(b.cut().as_deref(), Some("he"));
        assert_eq!(b.text(), "llo");
    }

    #[test]
    fn paste_normalises_newlines() {
        let mut b = buf("");
        b.paste("a\r\n\r\nb\u{7}c");
        assert_eq!(b.text(), "a bc");
        let mut m = TextBuffer::new("", true);
        m.paste("a\r\nb\rc\n");
        assert_eq!(m.text(), "a\nb\nc\n");
    }

    #[test]
    fn undo_coalesces_typing_and_restores_selection() {
        let mut b = buf("");
        for c in "abc".chars() {
            b.insert(&c.to_string());
        }
        assert_eq!(b.undo.len(), 1);
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        assert_eq!(b.text(), "abc");

        let mut b = buf("hello world");
        b.select(0, 5);
        b.insert("x");
        assert_eq!(b.text(), "x world");
        b.undo();
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.selection(), (0, 5));
    }

    #[test]
    fn caret_move_and_kind_change_end_group() {
        let mut b = buf("");
        b.insert("a");
        b.insert("b");
        b.move_to(0, false);
        b.insert("c");
        assert_eq!(b.undo.len(), 2);
        let mut b = buf("abcdef");
        b.backspace();
        b.backspace();
        b.insert("x");
        assert_eq!(b.undo.len(), 2);
        b.undo();
        assert_eq!(b.text(), "abcd");
        b.undo();
        assert_eq!(b.text(), "abcdef");
    }

    #[test]
    fn pause_ends_group() {
        let mut b = buf("");
        b.insert("a");
        b.last_edit = Some(Instant::now() - GROUP_PAUSE - Duration::from_millis(10));
        b.insert("b");
        assert_eq!(b.undo.len(), 2);
    }

    #[test]
    fn paste_and_cut_are_their_own_steps() {
        let mut b = buf("");
        b.insert("a");
        b.paste("zz");
        b.insert("b");
        assert_eq!(b.undo.len(), 3);
    }

    #[test]
    fn edit_clears_redo() {
        let mut b = buf("a");
        b.insert("b");
        b.undo();
        assert!(b.can_redo());
        b.insert("c");
        assert!(!b.can_redo());
    }

    #[test]
    fn ime_replace_range() {
        let mut b = buf("ab");
        b.replace_range_with(1, 1, "に");
        b.replace_range_with(1, 1 + "に".len(), "日本");
        assert_eq!(b.text(), "a日本b");
    }

    #[test]
    fn size_cap_refuses_insert() {
        let mut b = TextBuffer::new(&"a".repeat(UNDO_LIMIT_BYTES), true);
        assert!(!b.insert("b"));
    }
}
