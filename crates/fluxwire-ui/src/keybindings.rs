//! The Keybindings tab: every action with its effective binding, searchable (SPEC §17.6).

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, div, prelude::*,
    px,
};

use crate::{
    actions::{ACTIONS, BindingSource, KeymapState, OpenKeymapFile},
    assets::BUFFER_FONT,
    theme::Theme,
    ui::{kbd_label, row},
    workspace::{WorkspaceView, fuzzy},
};

/// One line of the table.
pub struct KeyRow {
    pub label: &'static str,
    pub keys: String,
    pub context: String,
    pub source: &'static str,
}

impl WorkspaceView {
    /// Rows matching the search box: bound actions first-class, unbound actions with no keys.
    pub fn keybinding_rows(&self, state: &KeymapState) -> Vec<KeyRow> {
        let q = self.keybind_query.text().to_string();
        let mut rows: Vec<KeyRow> = Vec::new();
        for a in ACTIONS {
            let bound: Vec<_> = state.rows.iter().filter(|r| r.action == a.name).collect();
            if bound.is_empty() {
                rows.push(KeyRow {
                    label: a.label,
                    keys: String::new(),
                    context: String::new(),
                    source: "—",
                });
            }
            for r in bound {
                rows.push(KeyRow {
                    label: a.label,
                    keys: kbd_label(&r.keys),
                    context: r.context.clone().unwrap_or_default(),
                    source: match r.source {
                        BindingSource::Default => "default",
                        BindingSource::User => "user",
                    },
                });
            }
        }
        rows.retain(|r| fuzzy(&q, &format!("{} {} {}", r.label, r.keys, r.context)));
        rows.sort_by(|a, b| a.label.cmp(b.label));
        rows
    }

    pub(crate) fn render_keybindings(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let state = cx.try_global::<KeymapState>().cloned().unwrap_or_default();
        let rows = self.keybinding_rows(&state);
        let t = t.clone();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                row()
                    .h(px(36.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(
                        row()
                            .id("keybind-search")
                            .track_focus(&self.keybind_focus)
                            .on_key_down(cx.listener(|this, ev, _, cx| {
                                if this.keybind_query.handle_key(ev) {
                                    cx.notify();
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, w, _| w.focus(&this.keybind_focus)),
                            )
                            .flex_1()
                            .child(crate::ui::field_text(
                                &self.keybind_query,
                                "Search keybindings…",
                                &t,
                                self.is_focused(&self.keybind_focus),
                            )),
                    )
                    .child(
                        div()
                            .id("open-keymap")
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(t.accent)
                            .on_click(cx.listener(|_, _, w, cx| {
                                w.dispatch_action(Box::new(OpenKeymapFile), cx)
                            }))
                            .child("Open keymap.toml"),
                    ),
            )
            .children((!state.issues.is_empty()).then(|| {
                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .text_size(px(12.))
                    .text_color(t.modified)
                    .children(state.issues.iter().map(|i| {
                        div().child(format!("keymap.toml:{} {} {}", i.line, i.code, i.message))
                    }))
            }))
            .child(
                div()
                    .id("keybind-rows")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows.into_iter().enumerate().map(|(i, r)| {
                        row()
                            .id(("keybind", i))
                            .h(px(26.))
                            .px_3()
                            .gap_3()
                            .text_size(px(13.))
                            .hover({
                                let h = t.element_hover;
                                move |s| s.bg(h)
                            })
                            .child(div().flex_1().child(r.label))
                            .child(
                                div()
                                    .w(px(120.))
                                    .font_family(BUFFER_FONT)
                                    .text_color(t.text)
                                    .child(r.keys),
                            )
                            .child(div().w(px(110.)).text_color(t.text_muted).child(r.context))
                            .child(
                                div()
                                    .w(px(60.))
                                    .text_color(t.text_placeholder)
                                    .child(r.source),
                            )
                    })),
            )
            .into_any_element()
    }
}
