//! Body layout, left dock panels (Project, History, Environments, Sync) and the welcome view.

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    mock::TreeNode,
    tabs::TabKind,
    theme::Theme,
    ui::{icon, row},
    workspace::{Panel, Workspace},
};

impl Workspace {
    pub(crate) fn render_body(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let t = t.clone();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .when(self.left_open, |d| {
                d.child(self.render_left_dock(&t, cx)).child(
                    div()
                        .id("dock-resize")
                        .w(px(4.))
                        .flex_none()
                        .cursor_col_resize()
                        .bg(t.border)
                        .hover(|s| s.bg(t.accent))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing = true;
                                cx.notify();
                            }),
                        ),
                )
            })
            .child(self.render_center(&t, cx))
    }

    fn render_left_dock(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.panel {
            Panel::Project => "Project",
            Panel::History => "History",
            Panel::Environments => "Environments",
            Panel::Sync => "Sync",
        };
        let content = match self.panel {
            Panel::Project => self.render_project(t, cx).into_any_element(),
            Panel::History => self.render_history(t, cx).into_any_element(),
            Panel::Environments => self.render_envs(t, cx).into_any_element(),
            Panel::Sync => self.render_sync_panel(t, cx).into_any_element(),
        };
        div()
            .w(px(self.left_width))
            .flex_none()
            .flex()
            .flex_col()
            .bg(t.surface)
            .child(
                row()
                    .h(px(28.))
                    .px_2()
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(title)
                    .child(icon("plus", 12., t.text_muted)),
            )
            .child(
                div()
                    .id("dock-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
    }

    fn tree_row(
        &self,
        t: &Theme,
        id: gpui::ElementId,
        depth: usize,
        active: bool,
    ) -> gpui::Stateful<gpui::Div> {
        row()
            .id(id)
            .relative()
            .h(px(24.))
            .pl(px(10. + depth as f32 * 14.))
            .pr_2()
            .gap_1()
            .cursor_pointer()
            .text_size(px(14.))
            .when(active, |d| d.bg(t.element_selected))
            .hover({
                let h = t.element_hover;
                move |s| s.bg(h)
            })
            .children((0..depth).map(|level| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(16. + level as f32 * 14.))
                    .w(px(1.))
                    .bg(t.border_variant)
            }))
    }

    fn render_project(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = vec![];
        let active_key = self.tabs.active().map(|a| a.key.clone());
        rows.push(
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(self.mock.workspace)
                .into_any_element(),
        );
        let mut idx = 0usize;
        for node in &self.mock.tree {
            self.push_node(t, node, 0, &active_key, &mut rows, &mut idx, cx);
        }
        div().flex().flex_col().children(rows)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_node(
        &self,
        t: &Theme,
        node: &TreeNode,
        depth: usize,
        active_key: &Option<String>,
        rows: &mut Vec<gpui::AnyElement>,
        idx: &mut usize,
        cx: &mut Context<Self>,
    ) {
        *idx += 1;
        match node {
            TreeNode::Folder {
                path,
                name,
                children,
            } => {
                let collapsed = self.collapsed.contains(path);
                let path = *path;
                rows.push(
                    self.tree_row(t, ("tree", *idx).into(), depth, false)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.collapsed.remove(path) {
                                this.collapsed.insert(path);
                            }
                            cx.notify();
                        }))
                        .child(icon(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                            t.text_placeholder,
                        ))
                        .child(icon(
                            if collapsed { "folder" } else { "folder-open" },
                            14.,
                            t.text_muted,
                        ))
                        .child(*name)
                        .into_any_element(),
                );
                if !collapsed {
                    for c in children {
                        self.push_node(t, c, depth + 1, active_key, rows, idx, cx);
                    }
                }
            }
            TreeNode::Request(id) => {
                let Some(r) = self.mock.request(id) else {
                    return;
                };
                let id = *id;
                let active = active_key.as_deref() == Some(id);
                rows.push(
                    self.tree_row(t, ("tree", *idx).into(), depth, active)
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, w, cx| {
                            this.open_request(id, ev.click_count() >= 2, w, cx);
                        }))
                        .child(
                            div()
                                .w(px(34.))
                                .flex_none()
                                .text_right()
                                .text_size(px(10.))
                                .font_family(BUFFER_FONT)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.method_color(r.method))
                                .child(if r.method == "DELETE" {
                                    "DEL"
                                } else {
                                    r.method
                                }),
                        )
                        .child(div().truncate().child(r.name))
                        .into_any_element(),
                );
            }
        }
    }

    fn render_history(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = vec![];
        let mut last_day = "";
        for (i, h) in self.mock.history.iter().enumerate() {
            if h.day != last_day {
                last_day = h.day;
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(h.day)
                        .into_any_element(),
                );
            }
            let name = h.name;
            let key = format!("history/{i}");
            rows.push(
                self.tree_row(
                    t,
                    ("hist", i).into(),
                    0,
                    self.tabs.active().is_some_and(|a| a.key == key),
                )
                .gap_2()
                .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                    this.open_tab(&key, name, TabKind::History, ev.click_count() >= 2);
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(40.))
                        .text_size(px(10.))
                        .font_family(BUFFER_FONT)
                        .text_color(t.method_color(h.method))
                        .child(if h.method == "DELETE" {
                            "DEL"
                        } else {
                            h.method
                        }),
                )
                .child(div().flex_1().truncate().child(h.name))
                .child(
                    div()
                        .font_family(BUFFER_FONT)
                        .text_size(px(11.))
                        .text_color(t.status_color(h.status))
                        .child(h.status.to_string()),
                )
                .child(
                    div()
                        .font_family(BUFFER_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(h.time),
                )
                .into_any_element(),
            );
        }
        div().flex().flex_col().children(rows)
    }

    fn render_envs(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let env = &self.mock.environments[self.env];
        div()
            .flex()
            .flex_col()
            .children(self.mock.environments.iter().enumerate().map(|(i, e)| {
                let name = e.name;
                self.tree_row(t, ("env", i).into(), 0, i == self.env)
                    .gap_2()
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                        this.env = i;
                        if ev.click_count() >= 2 {
                            this.open_tab(&format!("env/{name}"), name, TabKind::Environment, true);
                        }
                        cx.notify();
                    }))
                    .child(div().size(px(7.)).rounded_full().bg(if i == self.env {
                        t.created
                    } else {
                        t.border
                    }))
                    .child(e.name)
            }))
            .child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child(format!("Variables · {}", env.name)),
            )
            .children(env.vars.iter().map(|(k, v, secret)| {
                row()
                    .px_2()
                    .py(px(2.))
                    .gap_2()
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(div().w(px(80.)).flex_none().text_color(t.text).child(*k))
                    .child(div().truncate().child(if *secret {
                        "••••••••"
                    } else {
                        *v
                    }))
            }))
    }

    fn render_sync_panel(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let down = self.mock.sync.iter().filter(|c| c.arrow == "↓").count();
        let up = self.mock.sync.len() - down;
        div()
            .flex()
            .flex_col()
            .child(
                self.tree_row(
                    t,
                    "sync-link".into(),
                    0,
                    self.tabs.active().is_some_and(|a| a.key == "sync"),
                )
                .on_click(cx.listener(|this, ev: &gpui::ClickEvent, _, cx| {
                    this.open_tab("sync", "Sync", TabKind::Sync, ev.click_count() >= 2);
                    cx.notify();
                }))
                .child(icon("refresh-cw", 14., t.text_muted))
                .child(div().flex_1().truncate().child("openapi/payments.yaml"))
                .child(
                    div()
                        .font_family(BUFFER_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_muted)
                        .child(format!("↓{down} ↑{up}")),
                ),
            )
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("Linked to the payments folder"),
            )
    }

    pub(crate) fn render_welcome(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let t = t.clone();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            let t = t.clone();
            div()
                .id(id)
                .px_4()
                .h(px(30.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .cursor_pointer()
                .border_1()
                .border_color(t.border)
                .when(primary, |d| {
                    d.bg(t.accent).text_color(t.editor).border_color(t.accent)
                })
                .when(!primary, |d| d.hover(|s| s.bg(t.element_hover)))
                .child(label)
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Fluxwire"),
            )
            .child(
                div()
                    .text_color(t.text_muted)
                    .child("API requests that live next to your code."),
            )
            .child(
                row()
                    .gap_2()
                    .child(button("w-open", "Open folder", true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.workspace_open = true;
                            cx.notify();
                        },
                    )))
                    .child(button("w-new", "New collection", false))
                    .child(button("w-import", "Import", false)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child("Recent workspaces"),
                    )
                    .children(
                        self.mock
                            .recent_workspaces
                            .iter()
                            .enumerate()
                            .map(|(i, w)| {
                                div()
                                    .id(("recent", i))
                                    .px_2()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .font_family(BUFFER_FONT)
                                    .text_size(px(12.))
                                    .text_color(t.accent)
                                    .hover(|s| s.bg(t.element_hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.workspace_open = true;
                                        cx.notify();
                                    }))
                                    .child(*w)
                            }),
                    ),
            )
    }
}
