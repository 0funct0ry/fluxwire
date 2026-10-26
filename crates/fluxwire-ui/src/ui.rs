//! Small shared widgets and the single-line text state used by the URL bar and modals.

use gpui::{
    AnyElement, Div, Hsla, IntoElement, KeyDownEvent, ParentElement, SharedString, Styled, div, px,
    svg,
};

use crate::theme::Theme;

/// A Lucide icon tinted by `color`.
pub fn icon(name: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Single-line editable text. The owner feeds it key events; it only stores the string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
}

impl TextEdit {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn insert(&mut self, s: &str) {
        self.text.extend(s.chars().filter(|c| !c.is_control()));
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }

    /// Applies typing and backspace. Returns true when the event edited the text.
    pub fn handle_key(&mut self, ev: &KeyDownEvent) -> bool {
        let ks = &ev.keystroke;
        if ks.modifiers.control || ks.modifiers.platform || ks.modifiers.alt {
            return false;
        }
        if ks.key == "backspace" {
            self.backspace();
            return true;
        }
        match ks.key_char.as_deref() {
            Some(s) if !s.is_empty() => {
                self.insert(s);
                true
            }
            _ => false,
        }
    }
}

/// Human-readable key chip text for a binding such as `secondary-shift-p`.
pub fn kbd_label(binding: &str) -> String {
    let mac = cfg!(target_os = "macos");
    binding
        .split('-')
        .map(|part| match (part, mac) {
            ("secondary", true) => "⌘".to_string(),
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

/// Splits `text` into plain and `{{variable}}` runs and styles the variables as chips.
pub fn with_variables(text: &str, t: &Theme) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = vec![];
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else {
            break;
        };
        let end = start + len + 2;
        if start > 0 {
            out.push(div().child(rest[..start].to_string()).into_any_element());
        }
        out.push(
            div()
                .text_color(t.accent)
                .bg(t.accent_bg())
                .rounded(px(3.))
                .px(px(2.))
                .child(rest[start..end].to_string())
                .into_any_element(),
        );
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        out.push(div().child(rest.to_string()).into_any_element());
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
pub fn url_runs(text: &str, sel: Option<(usize, usize)>, t: &Theme) -> Vec<AnyElement> {
    let chars: Vec<char> = text.chars().collect();
    let mut in_var = vec![false; chars.len()];
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '{'
            && chars[i + 1] == '{'
            && let Some(j) = (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '}' && chars[j + 1] == '}')
        {
            in_var[i..j + 2].iter_mut().for_each(|v| *v = true);
            i = j + 2;
            continue;
        }
        i += 1;
    }
    let sel = sel
        .map(|(a, b)| (a.min(b), a.max(b)))
        .filter(|(a, b)| a != b);
    let mut out = Vec::new();
    let mut start = 0;
    let class = |k: usize| (in_var[k], sel.is_some_and(|(a, b)| (a..b).contains(&k)));
    for k in 1..=chars.len() {
        if k == chars.len() || class(k) != class(start) {
            let (var, selected) = class(start);
            let run: String = chars[start..k].iter().collect();
            let mut d = div().child(run);
            if var {
                d = d.text_color(t.accent);
            }
            d = if selected {
                d.bg(t.element_selected)
            } else if var {
                d.bg(t.accent_bg())
            } else {
                d
            };
            out.push(d.into_any_element());
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
