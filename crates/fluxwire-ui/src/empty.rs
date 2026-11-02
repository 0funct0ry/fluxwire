//! Workspace-level views: the panel header with its close button and menu, and the empty state
//! of a Workspace with no Collections (SPEC §17.8).

use crate::dev_id::{DevClick, Role};
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    dialogs::SINGLE_COLLECTION_TIP,
    theme::Theme,
    ui::{VarTip, icon, row},
    workspace::{MenuKind, WorkspaceView},
};

fn tip(text: String) -> impl Fn(&mut Window, &mut gpui::App) -> gpui::AnyView {
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| VarTip { text }).into()
    }
}

impl WorkspaceView {
    /// Whether the open Workspace is itself a Collection (`fluxwire.toml` at its root).
    pub fn is_single_collection(&self) -> bool {
        self.scope
            .workspace
            .as_ref()
            .is_some_and(|w| w.root.join("fluxwire.toml").is_file())
    }

    /// Workspace name and git branch, with the "…" button. The button and a right-click open the
    /// same context menu (`MenuKind::Workspace`) as the Collection rows have.
    pub(crate) fn render_workspace_header(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let ws = self.scope.workspace.as_ref()?;
        let t = t.clone();
        Some(
            row()
                .id("ws-header")
                .h(px(30.))
                .px_2()
                .gap_2()
                .border_b_1()
                .border_color(t.border)
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                        this.open_tree_menu("", MenuKind::Workspace, ev.position, cx)
                    }),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(12.))
                        .truncate()
                        .child(ws.name.clone()),
                )
                .children(ws.branch.clone().map(|b| {
                    div()
                        .flex_1()
                        .truncate()
                        .font_family(BUFFER_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_muted)
                        .child(b)
                }))
                .when(ws.branch.is_none(), |d| d.child(div().flex_1()))
                .child(
                    div()
                        .id("ws-menu-btn")
                        .p_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.element_hover))
                        .dev_click(
                            "ws-menu-btn",
                            Role::Button,
                            "ws-menu-btn",
                            true,
                            cx.listener(|this, ev: &gpui::ClickEvent, _, cx| {
                                this.open_tree_menu("", MenuKind::Workspace, ev.position(), cx)
                            }),
                        )
                        .child(icon("ellipsis-vertical", 14., t.text_muted)),
                )
                .into_any_element(),
        )
    }

    /// The three Collection actions plus the collection error rows. `compact` is the sidebar form.
    pub(crate) fn render_collection_actions(
        &self,
        t: &Theme,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let t = t.clone();
        let single = self.is_single_collection();
        let errors: Vec<String> = self
            .scope
            .workspace
            .as_ref()
            .map(|w| {
                w.errors
                    .iter()
                    .map(|(dir, e)| format!("{}: {e}", dir.display()))
                    .collect()
            })
            .unwrap_or_default();
        let action = |id: &'static str, glyph: &'static str, label: &'static str, enabled: bool| {
            row()
                .id(id)
                .h(px(if compact { 26. } else { 34. }))
                .px_3()
                .gap_3()
                .rounded(px(6.))
                .text_size(px(if compact { 12. } else { 14. }))
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|s| s.bg(t.element_hover))
                })
                .when(!enabled, |d| d.text_color(t.text_placeholder))
                .child(icon(glyph, 16., t.text_muted))
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(!compact, |d| d.w(px(340.)))
            .children(errors.into_iter().enumerate().map(|(i, e)| {
                div()
                    .id(("coll-error", i))
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(t.deleted)
                    .child(SharedString::from(e))
            }))
            .child(
                action(
                    "ws-new-collection",
                    "folder-plus",
                    "New collection…",
                    !single,
                )
                .when(single, |d| d.tooltip(tip(SINGLE_COLLECTION_TIP.into())))
                .when(!single, |d| {
                    d.when(!compact, |d| d.bg(t.accent).text_color(t.editor))
                })
                .dev_click(
                    "empty-new-collection",
                    Role::Button,
                    "New collection",
                    true,
                    cx.listener(|this, _, w, cx| this.start_new_collection(None, w, cx)),
                ),
            )
            .child(
                action(
                    "ws-add-collection",
                    "folder-open",
                    "Add existing collection…",
                    !single,
                )
                .when(single, |d| d.tooltip(tip(SINGLE_COLLECTION_TIP.into())))
                .dev_click(
                    "empty.render_collection_actions",
                    Role::Button,
                    "empty.render_collection_actions",
                    true,
                    cx.listener(|this, _, w, cx| this.start_add_collection(w, cx)),
                ),
            )
            .child(
                action("ws-import", "arrow-down", "Import…", true).dev_click(
                    "empty.render_collection_actions",
                    Role::Button,
                    "empty.render_collection_actions",
                    true,
                    cx.listener(|this, _, w, cx| this.start_import(None, w, cx)),
                ),
            )
    }

    /// Center area when the Workspace has no Collections and no tab is open.
    pub(crate) fn render_workspace_empty(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = self
            .scope
            .workspace
            .as_ref()
            .map(|w| w.name.clone())
            .unwrap_or_default();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(format!("{name} has no collections")),
            )
            .child(
                div().text_size(px(13.)).text_color(t.text_muted).child(
                    "A collection holds requests, environments, and settings (fluxwire.toml).",
                ),
            )
            .child(self.render_collection_actions(t, false, cx))
    }
}
