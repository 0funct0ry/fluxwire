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
    fn kbd_labels_use_platform_modifier() {
        let l = kbd_label("secondary-shift-p");
        if cfg!(target_os = "macos") {
            assert_eq!(l, "⌘⇧P");
        } else {
            assert_eq!(l, "Ctrl+Shift+P");
        }
    }
}
