//! Small shared widgets and the single-line text state used by the URL bar and modals.

use gpui::{
    AnyElement, AppContext, Div, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
    px, svg,
};

use crate::{envs::VarInfo, theme::Theme};

/// A Lucide icon tinted by `color`.
pub fn icon(name: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Single-line editable text with a caret and an optional selection (char indices). The owner
/// feeds it key events; render it with `field_text` and place the caret from a click with
/// `char_index_at` + `set_cursor`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
}

impl TextEdit {
    /// Text with the caret at its end.
    pub fn new(text: impl Into<String>) -> Self {
        let text: String = text.into();
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Caret position in chars.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected range in chars (start < end), if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(idx)
            .map_or(self.text.len(), |(b, _)| b)
    }

    /// Move the caret; `extend` keeps (or starts) a selection anchored where it was.
    pub fn set_cursor(&mut self, idx: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = idx.min(self.len());
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.len();
    }

    /// Select the chars `a..b` (the caret ends at `b`).
    pub fn select(&mut self, a: usize, b: usize) {
        self.anchor = Some(a.min(self.len()));
        self.cursor = b.min(self.len());
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, "");
        self.cursor = a;
        self.anchor = None;
        true
    }

    /// Insert at the caret, replacing the selection. Control characters are dropped.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        let at = self.byte(self.cursor);
        self.text.insert_str(at, &s);
        self.cursor += s.chars().count();
    }

    /// Delete the selection, or the char before the caret.
    pub fn backspace(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        let (a, b) = (self.byte(self.cursor - 1), self.byte(self.cursor));
        self.text.replace_range(a..b, "");
        self.cursor -= 1;
    }

    /// Delete the selection, or the char after the caret.
    pub fn delete_forward(&mut self) {
        if self.delete_selection() || self.cursor >= self.len() {
            return;
        }
        let (a, b) = (self.byte(self.cursor), self.byte(self.cursor + 1));
        self.text.replace_range(a..b, "");
    }

    fn word_boundary(&self, from: usize, forward: bool) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut i = from;
        if forward {
            while i < chars.len() && !word(chars[i]) {
                i += 1;
            }
            while i < chars.len() && word(chars[i]) {
                i += 1;
            }
        } else {
            while i > 0 && !word(chars[i - 1]) {
                i -= 1;
            }
            while i > 0 && word(chars[i - 1]) {
                i -= 1;
            }
        }
        i
    }

    /// Applies typing, Backspace/Delete, arrows, Home/End (Alt: by word, Cmd/Ctrl: to the ends),
    /// Shift to select and Cmd/Ctrl+A. Returns true when the event was consumed.
    pub fn handle_key(&mut self, ev: &KeyDownEvent) -> bool {
        let ks = &ev.keystroke;
        let (shift, alt, sec) = (
            ks.modifiers.shift,
            ks.modifiers.alt,
            ks.modifiers.secondary(),
        );
        let sel = self.selection();
        match ks.key.as_str() {
            "a" if sec => self.select_all(),
            "left" => {
                let to = match sel {
                    _ if sec => 0,
                    _ if alt => self.word_boundary(self.cursor, false),
                    Some((a, _)) if !shift => a,
                    _ => self.cursor.saturating_sub(1),
                };
                self.set_cursor(to, shift);
            }
            "right" => {
                let to = match sel {
                    _ if sec => self.len(),
                    _ if alt => self.word_boundary(self.cursor, true),
                    Some((_, b)) if !shift => b,
                    _ => (self.cursor + 1).min(self.len()),
                };
                self.set_cursor(to, shift);
            }
            "home" => self.set_cursor(0, shift),
            "end" => self.set_cursor(self.len(), shift),
            "backspace" => self.backspace(),
            "delete" => self.delete_forward(),
            _ if ks.modifiers.control || ks.modifiers.platform || alt => return false,
            _ => match ks.key_char.as_deref() {
                Some(s) if !s.is_empty() => self.insert(s),
                _ => return false,
            },
        }
        true
    }
}

/// The text of a single-line field with the caret at the cursor and the selection highlighted;
/// the placeholder when empty. The caret shows only while `focused`.
pub fn field_text(edit: &TextEdit, placeholder: &'static str, t: &Theme, focused: bool) -> Div {
    let caret = || div().w(px(1.)).h(px(15.)).flex_none().bg(t.accent);
    let base = div()
        .flex()
        .flex_row()
        .items_center()
        .whitespace_nowrap()
        .min_h(px(16.));
    if edit.text().is_empty() {
        return base
            .when(focused, |d| d.child(caret()))
            .child(div().text_color(t.text_placeholder).child(placeholder));
    }
    let chars: Vec<char> = edit.text().chars().collect();
    let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
    match edit.selection() {
        Some((a, b)) => base
            .child(part(0, a))
            .child(div().bg(t.accent_bg()).child(part(a, b)))
            .child(part(b, chars.len())),
        None => {
            let c = edit.cursor().min(chars.len());
            base.child(part(0, c))
                .when(focused, |d| d.child(caret()))
                .child(part(c, chars.len()))
        }
    }
}

/// The char index under `x` (pixels from the start of the text) for text laid out in `font`
/// at `size`.
pub fn char_index_at(
    window: &gpui::Window,
    text: &str,
    font: &'static str,
    size: f32,
    x: f32,
) -> usize {
    if text.is_empty() {
        return 0;
    }
    let run = gpui::TextRun {
        len: text.len(),
        font: gpui::font(font),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.to_string().into(), px(size), &[run], None);
    let byte = line.closest_index_for_x(px(x.max(0.))).min(text.len());
    text[..byte].chars().count()
}

/// Human-readable key chip text for a binding such as `secondary-shift-p`.
pub fn kbd_label(binding: &str) -> String {
    let mac = cfg!(target_os = "macos");
    binding
        .split('-')
        .map(|part| match (part, mac) {
            ("secondary", true) | ("cmd", true) | ("platform", true) => "⌘".to_string(),
            ("cmd", false) | ("platform", false) => "Super+".to_string(),
            ("secondary", false) => "Ctrl+".to_string(),
            ("shift", true) => "⇧".to_string(),
            ("shift", false) => "Shift+".to_string(),
            ("alt", true) => "⌥".to_string(),
            ("alt", false) => "Alt+".to_string(),
            ("ctrl", true) => "⌃".to_string(),
            ("ctrl", false) => "Ctrl+".to_string(),
            ("enter", _) => "↵".to_string(),
            ("escape", _) => "Esc".to_string(),
            ("tab", _) => "Tab".to_string(),
            (k, _) => k.to_uppercase(),
        })
        .collect()
}

/// Hover text for a variable: a small floating card.
pub(crate) struct VarTip {
    pub(crate) text: String,
}

impl gpui::Render for VarTip {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        div()
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .text_size(px(12.))
            .text_color(t.text)
            .child(self.text.clone())
    }
}

/// Style of one `{{name}}` reference: accent when it resolves, red underline when it does not,
/// with the resolved value and its source on hover.
fn var_chip(
    inner: String,
    name: &str,
    id: usize,
    t: &Theme,
    look: Option<&dyn Fn(&str) -> VarInfo>,
) -> AnyElement {
    let Some(look) = look else {
        return div()
            .text_color(t.accent)
            .bg(t.accent_bg())
            .rounded(px(3.))
            .child(inner)
            .into_any_element();
    };
    let info = look(name);
    let tip = info.tip.clone();
    let d = div()
        .id(("var-ref", id))
        .rounded(px(3.))
        .child(inner)
        .tooltip(move |_, cx| {
            let tip = tip.clone();
            cx.new(|_| VarTip { text: tip }).into()
        });
    if info.resolved {
        d.text_color(t.accent).bg(t.accent_bg())
    } else {
        d.text_color(t.deleted).border_b_1().border_color(t.deleted)
    }
    .into_any_element()
}

/// Splits `text` into plain and `{{variable}}` runs and styles the variables as chips.
pub fn with_variables(
    text: &str,
    t: &Theme,
    look: Option<&dyn Fn(&str) -> VarInfo>,
) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = vec![];
    let mut at = 0;
    for (i, r) in fluxwire_vars::refs(text).into_iter().enumerate() {
        if r.range.start > at {
            out.push(
                div()
                    .child(text[at..r.range.start].to_string())
                    .into_any_element(),
            );
        }
        out.push(var_chip(
            text[r.range.clone()].to_string(),
            &r.name,
            i,
            t,
            look,
        ));
        at = r.range.end;
    }
    if at < text.len() {
        out.push(div().child(text[at..].to_string()).into_any_element());
    }
    out
}

/// Char-index range of the "word" under `idx` (a whole `{{variable}}`, else a run between URL
/// separators). Used for double-click selection in the URL bar.
pub fn word_range(text: &str, idx: usize) -> (usize, usize) {
    let c: Vec<char> = text.chars().collect();
    if c.is_empty() {
        return (0, 0);
    }
    let i = idx.min(c.len() - 1);
    // inside {{...}}?
    let s: String = c.iter().collect();
    let mut from = 0;
    while let Some(a) = s[from..].find("{{") {
        let a = from + a;
        let Some(len) = s[a..].find("}}") else { break };
        let (ca, cb) = (s[..a].chars().count(), s[..a + len + 2].chars().count());
        if (ca..cb).contains(&i) {
            return (ca, cb);
        }
        from = a + len + 2;
    }
    let sep = |ch: char| "/?&=:#.@,;{}".contains(ch) || ch.is_whitespace();
    if sep(c[i]) {
        return (i, i + 1);
    }
    let mut a = i;
    while a > 0 && !sep(c[a - 1]) {
        a -= 1;
    }
    let mut b = i + 1;
    while b < c.len() && !sep(c[b]) {
        b += 1;
    }
    (a, b)
}

/// Like `with_variables`, but with an optional selection (char indices) and no chip padding, so
/// every character is exactly one monospace cell wide and pixel positions map back to indices.
pub fn url_runs(
    text: &str,
    sel: Option<(usize, usize)>,
    t: &Theme,
    look: Option<&dyn Fn(&str) -> VarInfo>,
) -> Vec<AnyElement> {
    let chars: Vec<char> = text.chars().collect();
    // Which variable reference (index) each character belongs to.
    let mut var_at: Vec<Option<usize>> = vec![None; chars.len()];
    let mut names: Vec<String> = vec![];
    for (n, r) in fluxwire_vars::refs(text).into_iter().enumerate() {
        let a = text[..r.range.start].chars().count();
        let b = text[..r.range.end].chars().count();
        var_at[a..b].iter_mut().for_each(|v| *v = Some(n));
        names.push(r.name);
    }
    let sel = sel
        .map(|(a, b)| (a.min(b), a.max(b)))
        .filter(|(a, b)| a != b);
    let mut out = Vec::new();
    let mut start = 0;
    let class = |k: usize| (var_at[k], sel.is_some_and(|(a, b)| (a..b).contains(&k)));
    for k in 1..=chars.len() {
        if k == chars.len() || class(k) != class(start) {
            let (var, selected) = class(start);
            let run: String = chars[start..k].iter().collect();
            let mut el = match var {
                Some(n) => var_chip(run, &names[n], n, t, look),
                None => div().child(run).into_any_element(),
            };
            if selected {
                el = div().bg(t.element_selected).child(el).into_any_element();
            }
            out.push(el);
            start = k;
        }
    }
    out
}

/// A horizontal row container with the shared item gap.
pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_edit_inserts_and_deletes() {
        let mut e = TextEdit::new("ab");
        e.insert("c\n");
        assert_eq!(e.text(), "abc");
        e.backspace();
        assert_eq!(e.text(), "ab");
    }

    fn key(k: &str, ch: Option<&str>, shift: bool, secondary: bool) -> KeyDownEvent {
        let mut m = gpui::Modifiers {
            shift,
            ..Default::default()
        };
        if secondary {
            if cfg!(target_os = "macos") {
                m.platform = true;
            } else {
                m.control = true;
            }
        }
        KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers: m,
                key: k.into(),
                key_char: ch.map(Into::into),
            },
            is_held: false,
        }
    }

    #[test]
    fn text_edit_moves_the_caret_and_edits_there() {
        let mut e = TextEdit::new("abcd");
        assert_eq!(e.cursor(), 4);
        e.handle_key(&key("left", None, false, false));
        e.handle_key(&key("left", None, false, false));
        e.handle_key(&key("x", Some("x"), false, false));
        assert_eq!((e.text(), e.cursor()), ("abxcd", 3));
        e.handle_key(&key("home", None, false, false));
        e.handle_key(&key("delete", None, false, false));
        assert_eq!(e.text(), "bxcd");
        e.handle_key(&key("backspace", None, false, false));
        assert_eq!(e.text(), "bxcd", "nothing before the caret");
        e.handle_key(&key("end", None, false, false));
        e.handle_key(&key("left", None, true, false));
        e.handle_key(&key("left", None, true, false));
        assert_eq!(e.selection(), Some((2, 4)));
        e.handle_key(&key("y", Some("y"), false, false));
        assert_eq!(e.text(), "bxy");
        e.handle_key(&key("a", Some("a"), false, true));
        assert_eq!(e.selection(), Some((0, 3)));
        e.handle_key(&key("right", None, false, false));
        assert_eq!((e.selection(), e.cursor()), (None, 3));
        e.set_cursor(1, false);
        e.insert("é");
        assert_eq!((e.text(), e.cursor()), ("béxy", 2), "multi-byte chars");
        e.backspace();
        assert_eq!(e.text(), "bxy");
    }

    #[test]
    fn double_click_picks_a_word_or_a_variable() {
        let u = "https://api.test/v1/{{id}}/x";
        assert_eq!(word_range(u, 9), (8, 11)); // `api`: '.' separates
        assert_eq!(word_range(u, 2), (0, 5)); // https
        let v = u.find("{{").unwrap();
        assert_eq!(word_range(u, v + 3), (v, v + 6));
        assert_eq!(word_range("", 0), (0, 0));
    }

    #[test]
    fn kbd_labels_use_platform_modifier() {
        let l = kbd_label("secondary-shift-p");
        if cfg!(target_os = "macos") {
            assert_eq!(l, "⌘⇧P");
        } else {
            assert_eq!(l, "Ctrl+Shift+P");
        }
    }
}
