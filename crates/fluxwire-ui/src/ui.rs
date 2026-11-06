//! Small shared widgets: icons, key chips, `{{variable}}` chips.

use gpui::{
    AnyElement, AppContext, Div, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px, svg,
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

/// `{{variable}}` chips for the URL input: accent when it resolves, an underline in the error
/// colour when it does not, and the resolved value and its source on hover.
pub fn url_decorations(
    text: &str,
    t: &Theme,
    look: &dyn Fn(&str) -> VarInfo,
) -> Vec<crate::text_input::Decoration> {
    fluxwire_vars::refs(text)
        .into_iter()
        .map(|r| {
            let info = look(&r.name);
            let (color, background, underline) = if info.resolved {
                (t.accent, Some(t.accent_bg()), None)
            } else {
                (t.deleted, None, Some(t.deleted))
            };
            crate::text_input::Decoration {
                range: r.range,
                color: Some(color),
                background,
                underline,
                tip: Some(info.tip.into()),
            }
        })
        .collect()
}

/// `:name` path parameter chips for the URL input: accent when the parameter has a value (or a
/// variable of the same name resolves), error colour when it has neither, with the value on hover.
pub fn path_decorations(
    text: &str,
    t: &Theme,
    values: &[(String, String)],
    look: &dyn Fn(&str) -> VarInfo,
) -> Vec<crate::text_input::Decoration> {
    fluxwire_format::path_param_ranges(text)
        .into_iter()
        .map(|(name, range)| {
            let set = values
                .iter()
                .find(|(k, v)| *k == name && !v.is_empty())
                .map(|(_, v)| v.as_str());
            let (resolved, tip) = match set {
                Some(v) => (true, format!(":{name} = {v} · path parameter")),
                None => {
                    let info = look(&name);
                    if info.resolved {
                        (true, format!(":{name} · {}", info.tip))
                    } else {
                        (
                            false,
                            format!(
                                ":{name} has no value · set it under Params > Path or define a variable named {name}"
                            ),
                        )
                    }
                }
            };
            let (color, background, underline) = if resolved {
                (t.accent, Some(t.accent_bg()), None)
            } else {
                (t.deleted, None, Some(t.deleted))
            };
            crate::text_input::Decoration {
                range,
                color: Some(color),
                background,
                underline,
                tip: Some(tip.into()),
            }
        })
        .collect()
}

/// A horizontal row container with the shared item gap.
pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

#[cfg(test)]
mod tests {
    use super::*;

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
