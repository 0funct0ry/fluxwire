//! Small shared widgets and the single-line text state used by the URL bar and modals.

use gpui::{
    AnyElement, AppContext, Div, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, svg,
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
struct VarTip {
    text: String,
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
