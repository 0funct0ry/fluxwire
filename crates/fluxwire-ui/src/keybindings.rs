//! The Keybindings tab: every action with its effective binding, searchable (SPEC §17.6).

use crate::dev_id::{DevClick, Role};
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::*, px};

use crate::{
    actions::{ACTIONS, BindingSource, KeymapState, OpenKeymapFile},
    assets::BUFFER_FONT,
    theme::Theme,
    ui::{kbd_label, row},
    workspace::{WorkspaceView, fuzzy},
};

/// One line of the table.
pub struct KeyRow {
    pub action: &'static str,
    pub user: bool,
    pub label: &'static str,
    pub keys: String,
    pub context: String,
    pub source: &'static str,
}

impl WorkspaceView {
    /// Rows matching the search box: bound actions first-class, unbound actions with no keys.
    pub fn keybinding_rows(&self, state: &KeymapState) -> Vec<KeyRow> {
        let q = self.keybind_query.clone();
        let mut rows: Vec<KeyRow> = Vec::new();
        for a in ACTIONS {
            let bound: Vec<_> = state.rows.iter().filter(|r| r.action == a.name).collect();
            if bound.is_empty() {
                rows.push(KeyRow {
                    action: a.name,
                    user: false,
                    label: a.label,
                    keys: String::new(),
                    context: String::new(),
                    source: "—",
                });
            }
            for r in bound {
                rows.push(KeyRow {
                    action: a.name,
                    user: r.source == BindingSource::User,
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
        let editing = self.keybind_editing;
        let pending = self
            .keybind_edit
            .as_ref()
            .map(|p| (p.action, p.keys.clone(), p.conflict.clone()));
        let t = t.clone();
        let search = crate::text_input::lazy_input(
            &self.keybind_input,
            "Search keybindings…",
            cx,
            |this: &mut WorkspaceView, text, cx| {
                this.keybind_query = text;
                cx.notify();
            },
        );
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
                    .child(row().id("keybind-search").flex_1().child(search))
                    .child(
                        div()
                            .id("keybind-edit-toggle")
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(t.accent)
                            .dev_click(
                                "keybind-edit-toggle",
                                Role::Button,
                                "keybind-edit-toggle",
                                true,
                                cx.listener(|this, _, _, cx| {
                                    this.keybind_editing = !this.keybind_editing;
                                    this.keybind_edit = None;
                                    cx.notify();
                                }),
                            )
                            .child(if editing { "Done" } else { "Edit" }),
                    )
                    .child(
                        div()
                            .id("open-keymap")
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(t.accent)
                            .dev_click(
                                "open-keymap",
                                Role::Button,
                                "open-keymap",
                                true,
                                cx.listener(|_, _, w, cx| {
                                    w.dispatch_action(Box::new(OpenKeymapFile), cx)
                                }),
                            )
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
                                    .child(match &pending {
                                        Some((a, None, _)) if *a == r.action => {
                                            "Press keys…".to_string()
                                        }
                                        _ => r.keys,
                                    }),
                            )
                            .child(div().w(px(110.)).text_color(t.text_muted).child(r.context))
                            .child(
                                div()
                                    .w(px(60.))
                                    .text_color(t.text_placeholder)
                                    .child(r.source),
                            )
                            .children(editing.then(|| {
                                let action = r.action;
                                let link = |id: &'static str, text: &'static str| {
                                    div()
                                        .id((id, i))
                                        .cursor_pointer()
                                        .text_color(t.accent)
                                        .child(text)
                                };
                                row()
                                    .gap_2()
                                    .child(link("kb-edit", "Edit").dev_click(
                                        "keybindings.render_keybindings",
                                        Role::Button,
                                        "keybindings.render_keybindings",
                                        true,
                                        cx.listener(move |this, _, _, cx| {
                                            this.begin_capture(action, cx)
                                        }),
                                    ))
                                    .child(link("kb-clear", "Clear").dev_click(
                                        "keybindings.render_keybindings",
                                        Role::Button,
                                        "keybindings.render_keybindings",
                                        true,
                                        cx.listener(move |this, _, _, cx| {
                                            this.clear_keybinding(action, cx)
                                        }),
                                    ))
                                    .child(link("kb-reset", "Reset").dev_click(
                                        "keybindings.render_keybindings",
                                        Role::Button,
                                        "keybindings.render_keybindings",
                                        true,
                                        cx.listener(move |this, _, _, cx| {
                                            this.reset_keybinding(action, cx)
                                        }),
                                    ))
                            }))
                            .children(match &pending {
                                Some((a, Some(keys), Some(c))) if *a == r.action => Some(
                                    row()
                                        .gap_2()
                                        .text_color(t.modified)
                                        .child(format!(
                                            "{} is already {}",
                                            kbd_label(keys),
                                            c.label
                                        ))
                                        .child(
                                            div()
                                                .id(("kb-replace", i))
                                                .cursor_pointer()
                                                .text_color(t.accent)
                                                .dev_click(
                                                    "keybindings.render_keybindings",
                                                    Role::Button,
                                                    "keybindings.render_keybindings",
                                                    true,
                                                    cx.listener(|this, _, _, cx| {
                                                        this.confirm_replace(cx)
                                                    }),
                                                )
                                                .child("Replace"),
                                        )
                                        .child(
                                            div()
                                                .id(("kb-cancel", i))
                                                .cursor_pointer()
                                                .text_color(t.accent)
                                                .dev_click(
                                                    "keybindings.render_keybindings",
                                                    Role::Button,
                                                    "keybindings.render_keybindings",
                                                    true,
                                                    cx.listener(|this, _, _, cx| {
                                                        this.cancel_edit(cx)
                                                    }),
                                                )
                                                .child("Cancel"),
                                        ),
                                ),
                                _ => None,
                            })
                    })),
            )
            .into_any_element()
    }
}
