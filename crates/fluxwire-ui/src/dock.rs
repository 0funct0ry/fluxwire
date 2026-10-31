//! Body layout, left dock panels (Workspace, History, Environments, Sync) and the welcome view.

use fluxwire_model::{RequestKind, TreeNode};
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored as popover, deferred, div,
    prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    tabs::TabKind,
    theme::Theme,
    ui::{icon, row},
    workspace::{MenuKind, Panel, WorkspaceView},
};

/// A tree row being dragged to another folder.
#[derive(Clone)]
pub struct DraggedItem {
    pub key: String,
    pub name: String,
}

struct DragPreview {
    name: String,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        div()
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .text_size(px(13.))
            .text_color(t.text)
            .child(self.name.clone())
    }
}

impl WorkspaceView {
    pub(crate) fn render_body(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let t = t.clone();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .when(self.scope.left_open, |d| {
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
        let title = match self.scope.panel {
            Panel::Workspace => "Workspace",
            Panel::History => "History",
            Panel::Environments => "Environments",
            Panel::Sync => "Sync",
        };
        let content = match self.scope.panel {
            Panel::Workspace => self.render_workspace_tree(t, cx).into_any_element(),
            Panel::History => self.render_history(t, cx).into_any_element(),
            Panel::Environments => self.render_envs(t, cx).into_any_element(),
            Panel::Sync => self.render_sync_panel(t, cx).into_any_element(),
        };
        div()
            .w(px(self.scope.left_width))
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
                    .child(title),
            )
            .child(
                div()
                    .id("dock-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
    }

    pub(crate) fn tree_row(
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

    /// Small icon-only button for tree rows. Stops mouse-down so the row's own click/drag does not fire.
    fn row_icon_button(
        &self,
        t: &Theme,
        id: impl Into<gpui::ElementId>,
        glyph: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        let hover = t.element_selected;
        div()
            .id(id)
            .size(px(20.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(icon(glyph, 14., t.text_muted))
    }

    /// The in-place name entry row for a new request or folder.
    fn inline_row(&self, t: &Theme, depth: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(inline) = &self.scope.inline else {
            return div().into_any_element();
        };
        let (glyph, placeholder) = if inline.folder {
            ("folder", "Folder name")
        } else if inline.websocket {
            ("globe", "WebSocket request name")
        } else {
            ("file-plus", "Request name")
        };
        self.tree_row(t, "inline-new".into(), depth, false)
            .track_focus(&self.inline_focus)
            .on_key_down(cx.listener(|this, ev, w, cx| this.on_inline_key(ev, w, cx)))
            .on_mouse_down_out(cx.listener(|this, _, w, cx| this.cancel_inline(w, cx)))
            .child(div().w(px(12.)).flex_none())
            .child(icon(glyph, 14., t.text_muted))
            .child(
                row()
                    .flex_1()
                    .min_w_0()
                    .h(px(20.))
                    .px_1()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(t.accent)
                    .bg(t.editor)
                    .overflow_hidden()
                    .child(crate::ui::field_text(
                        &inline.text,
                        placeholder,
                        t,
                        self.is_focused(&self.inline_focus),
                    )),
            )
            .into_any_element()
    }

    fn inline_here(&self, parent: &str) -> bool {
        self.scope
            .inline
            .as_ref()
            .is_some_and(|i| i.parent == parent)
    }

    /// Vertical-dots button shown on row hover (or while that row's menu is open).
    fn row_dots(
        &self,
        t: &Theme,
        group: &str,
        key: &str,
        kind: MenuKind,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.scope.tree_menu.as_ref().is_some_and(|m| m.key == key);
        let key = key.to_string();
        self.row_icon_button(t, ("row-dots", idx), "ellipsis-vertical")
            .when(!open, |d| {
                d.invisible()
                    .group_hover(group.to_string(), |s| s.visible())
            })
            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                this.open_tree_menu(&key, kind, ev.position(), cx)
            }))
            .into_any_element()
    }

    fn render_workspace_tree(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = vec![];
        let Some(p) = &self.scope.workspace else {
            return div().into_any_element();
        };
        let selected = self.scope.selected.clone();
        let mut idx = 0usize;
        if let Some(h) = self.render_workspace_header(t, cx) {
            rows.push(h);
        }
        if p.collections.is_empty() {
            rows.push(
                div()
                    .p_2()
                    .child(self.render_collection_actions(t, true, cx))
                    .into_any_element(),
            );
        }
        for c in &p.collections {
            let ckey = c.key.clone();
            let cname = if c.collection.config.collection.name.is_empty() {
                p.name.clone()
            } else {
                c.collection.config.collection.name.clone()
            };
            idx += 1;
            let drop_key = ckey.clone();
            let menu_key = ckey.clone();
            let (nk1, nk2, nk3) = (ckey.clone(), ckey.clone(), ckey.clone());
            let menu_open = self.scope.tree_menu.as_ref().is_some_and(|m| m.key == ckey);
            rows.push(
                row()
                    .id(("coll", idx))
                    .h(px(28.))
                    .px_2()
                    .gap_1()
                    .mt_1()
                    .when(menu_open, |d| d.bg(t.element_selected))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text_muted)
                    .on_drop(cx.listener(move |this, d: &DraggedItem, _, cx| {
                        this.move_item(&d.key, &drop_key, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.open_tree_menu(&menu_key, MenuKind::Collection, ev.position, cx)
                        }),
                    )
                    .child(div().flex_1().truncate().child(cname))
                    .child(
                        self.row_icon_button(t, ("coll-new-req", idx), "file-plus")
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.start_new_in(&nk1, false, false, w, cx)
                            })),
                    )
                    .child(
                        self.row_icon_button(t, ("coll-new-folder", idx), "folder-plus")
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.start_new_in(&nk2, false, true, w, cx)
                            })),
                    )
                    .child(
                        self.row_icon_button(t, ("coll-more", idx), "ellipsis-vertical")
                            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                                this.open_tree_menu(&nk3, MenuKind::Collection, ev.position(), cx)
                            })),
                    )
                    .into_any_element(),
            );
            if c.tree.is_empty() {
                let action = |id: &'static str, glyph: &'static str, label: &'static str| {
                    let hover = t.element_hover;
                    row()
                        .id(id)
                        .h(px(28.))
                        .px_2()
                        .gap_2()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(t.text)
                        .hover(move |s| s.bg(hover))
                        .child(icon(glyph, 14., t.text_muted))
                        .child(label)
                };
                let (k1, k2, k3) = (ckey.clone(), ckey.clone(), ckey.clone());
                rows.push(
                    div()
                        .px_3()
                        .pt_4()
                        .pb_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child("No requests yet"),
                        )
                        .child(
                            div()
                                .pb_2()
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child("Start with a new request, or bring existing ones in."),
                        )
                        .child(action("empty-new-request", "plus", "New request").on_click(
                            cx.listener(move |this, _, w, cx| {
                                this.start_new_in(&k1, false, false, w, cx)
                            }),
                        ))
                        .child(
                            action("empty-new-ws", "globe", "New WebSocket request").on_click(
                                cx.listener(move |this, _, w, cx| {
                                    this.start_new_in(&k2, true, false, w, cx)
                                }),
                            ),
                        )
                        .child(action("empty-new-folder", "folder", "New folder").on_click(
                            cx.listener(move |this, _, w, cx| {
                                this.start_new_in(&k3, false, true, w, cx)
                            }),
                        ))
                        .child(action("empty-import", "arrow-down", "Import…").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.notice =
                                    Some("Import arrives in a later milestone (M11).".into());
                                cx.notify();
                            }),
                        ))
                        .into_any_element(),
                );
            }
            for node in &c.tree {
                self.push_node(t, node, 0, &selected, &mut rows, &mut idx, cx);
            }
            if self.inline_here(&c.key) {
                rows.push(self.inline_row(t, 0, cx));
            }
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn push_node(
        &self,
        t: &Theme,
        node: &TreeNode,
        depth: usize,
        selected: &Option<String>,
        rows: &mut Vec<gpui::AnyElement>,
        idx: &mut usize,
        cx: &mut Context<Self>,
    ) {
        *idx += 1;
        let drag_t = t.clone();
        let _ = drag_t;
        match node {
            TreeNode::Folder {
                path,
                name,
                children,
            } => {
                let collapsed = self.scope.collapsed.contains(path);
                let group = format!("tree-row-{idx}");
                let dots = self.row_dots(t, &group, path, MenuKind::Folder, *idx, cx);
                let key = path.clone();
                let drop_key = path.clone();
                let menu_key = path.clone();
                let drag = DraggedItem {
                    key: path.clone(),
                    name: name.clone(),
                };
                rows.push(
                    self.tree_row(
                        t,
                        ("tree", *idx).into(),
                        depth,
                        selected.as_deref() == Some(path.as_str()),
                    )
                    .group(group.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scope.selected = Some(key.clone());
                        if !this.scope.collapsed.remove(&key) {
                            this.scope.collapsed.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.open_tree_menu(&menu_key, MenuKind::Folder, ev.position, cx)
                        }),
                    )
                    .on_drag(drag, |d, _, _, cx| {
                        cx.new(|_| DragPreview {
                            name: d.name.clone(),
                        })
                    })
                    .drag_over::<DraggedItem>({
                        let h = t.element_selected;
                        move |s, _, _, _| s.bg(h)
                    })
                    .on_drop(cx.listener(move |this, d: &DraggedItem, _, cx| {
                        this.move_item(&d.key, &drop_key, cx)
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
                    .child(div().flex_1().truncate().child(name.clone()))
                    .child(dots)
                    .into_any_element(),
                );
                if !collapsed {
                    for c in children {
                        self.push_node(t, c, depth + 1, selected, rows, idx, cx);
                    }
                    if self.inline_here(path) {
                        rows.push(self.inline_row(t, depth + 1, cx));
                    }
                }
            }
            TreeNode::Request {
                name,
                path,
                method,
                kind,
            } => {
                let key = path.clone();
                let menu_key = path.clone();
                let active = self.scope.tabs.active().is_some_and(|a| a.key == *path);
                let group = format!("tree-row-{idx}");
                let dots = self.row_dots(t, &group, path, MenuKind::Request, *idx, cx);
                let drag = DraggedItem {
                    key: path.clone(),
                    name: name.clone(),
                };
                let label = if method == "DELETE" {
                    "DEL".to_string()
                } else {
                    method.clone()
                };
                let color = match kind {
                    RequestKind::WebSocket => t.method_color("WS"),
                    RequestKind::Other => t.text_placeholder,
                    RequestKind::Http => t.method_color(method),
                };
                rows.push(
                    self.tree_row(
                        t,
                        ("tree", *idx).into(),
                        depth,
                        active || selected.as_deref() == Some(path.as_str()),
                    )
                    .group(group.clone())
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, w, cx| {
                        this.open_request(&key, ev.click_count() >= 2, w, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.open_tree_menu(&menu_key, MenuKind::Request, ev.position, cx)
                        }),
                    )
                    .on_drag(drag, |d, _, _, cx| {
                        cx.new(|_| DragPreview {
                            name: d.name.clone(),
                        })
                    })
                    .child(
                        div()
                            .w(px(34.))
                            .flex_none()
                            .text_right()
                            .text_size(px(10.))
                            .font_family(BUFFER_FONT)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(color)
                            .child(label),
                    )
                    .child(div().flex_1().truncate().child(name.clone()))
                    .child(dots)
                    .into_any_element(),
                );
            }
            TreeNode::Doc { name, .. } => {
                rows.push(
                    self.tree_row(t, ("tree", *idx).into(), depth, false)
                        .text_color(t.text_muted)
                        .child(icon("file-text", 14., t.text_placeholder))
                        .child(name.clone())
                        .into_any_element(),
                );
            }
        }
    }

    /// Right-click menu for tree rows, anchored at the click position.
    pub(crate) fn render_tree_menu(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let m = self.scope.tree_menu.clone()?;
        let key = m.key.clone();
        type Act = fn(&mut WorkspaceView, &str, &mut Window, &mut Context<WorkspaceView>);
        let mut items: Vec<(&'static str, Act)> = vec![];
        match m.kind {
            MenuKind::Folder | MenuKind::Collection => {
                items.push(("New request", |w, k, win, cx| {
                    w.start_new_in(k, false, false, win, cx)
                }));
                items.push(("New WebSocket request", |w, k, win, cx| {
                    w.start_new_in(k, true, false, win, cx)
                }));
                items.push(("New folder", |w, k, win, cx| {
                    w.start_new_in(k, false, true, win, cx)
                }));
                items.push(("Run folder", |w, k, _win, cx| w.open_run_tab(k, cx)));
            }
            MenuKind::Request => {
                items.push(("Open", |w, k, win, cx| w.open_request(k, true, win, cx)))
            }
            MenuKind::Doc => {}
            MenuKind::Workspace => {
                items.push(("New collection…", |w, _, win, cx| {
                    w.start_new_collection(None, win, cx)
                }));
                items.push(("Add existing collection…", |w, _, win, cx| {
                    w.start_add_collection(win, cx)
                }));
                items.push(("Reveal in file manager", |w, _, _, cx| {
                    if let Some(root) = w.scope.workspace.as_ref().map(|ws| ws.root.clone()) {
                        cx.reveal_path(&root);
                    }
                }));
                items.push(("Copy path", |w, _, _, cx| {
                    if let Some(root) = w.scope.workspace.as_ref().map(|ws| ws.root.clone()) {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                            root.display().to_string(),
                        ));
                    }
                }));
                items.push(("Close workspace", |w, _, win, cx| {
                    w.request_close(crate::dialogs::AfterClose::Welcome, win, cx)
                }));
            }
        }
        if m.kind == MenuKind::Collection {
            items.push(("Rename", |w, k, win, cx| {
                w.start_rename_collection(k, win, cx)
            }));
            items.push(("Remove from workspace", |w, k, _, cx| {
                w.remove_collection(k, cx)
            }));
            items.push(("Delete (move to trash)", |w, k, win, cx| {
                w.confirm_delete_collection(k, win, cx)
            }));
        }
        if !matches!(m.kind, MenuKind::Collection | MenuKind::Workspace) {
            items.push(("Rename", |w, k, win, cx| {
                w.scope.selected = Some(k.to_string());
                w.start_rename(win, cx)
            }));
            items.push(("Duplicate", |w, k, win, cx| {
                w.scope.selected = Some(k.to_string());
                w.duplicate_selected(win, cx)
            }));
            items.push(("Delete (move to trash)", |w, k, win, cx| {
                w.scope.selected = Some(k.to_string());
                w.delete_selected(win, cx)
            }));
        }
        if m.kind != MenuKind::Workspace {
            items.push(("Reveal in file manager", |w, k, _, cx| {
                w.scope.selected = Some(k.to_string());
                w.reveal_selected(cx)
            }));
        }
        let t = t.clone();
        Some(
            deferred(
                popover().position(m.pos).child(
                    div()
                        .id("tree-menu")
                        .w(px(220.))
                        .py_1()
                        .flex()
                        .flex_col()
                        .bg(t.elevated)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(6.))
                        .shadow_lg()
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.scope.tree_menu = None;
                            cx.notify();
                        }))
                        .children(items.into_iter().enumerate().map(|(i, (label, act))| {
                            let key = key.clone();
                            let hover = t.element_hover;
                            row()
                                .id(("tree-menu-item", i))
                                .h(px(26.))
                                .mx_1()
                                .px_2()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.scope.tree_menu = None;
                                    act(this, &key, w, cx);
                                    cx.notify();
                                }))
                                .child(label)
                        })),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
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
                    self.scope.tabs.active().is_some_and(|a| a.key == "sync"),
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
        let button = |id: &'static str, label: &'static str, keys: &'static str, primary: bool| {
            let t = t.clone();
            row()
                .id(id)
                .px_4()
                .h(px(30.))
                .gap_2()
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
                .child(
                    div()
                        .text_size(px(11.))
                        .opacity(0.7)
                        .child(crate::ui::kbd_label(keys)),
                )
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(icon("logo", 48., t.accent))
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Fluxwire"),
            )
            .child(
                row()
                    .gap_2()
                    .child(
                        button("w-open", "Open workspace…", "secondary-o", true)
                            .on_click(cx.listener(|this, _, w, cx| this.open_folder_dialog(w, cx))),
                    )
                    .child(
                        button("w-new", "New workspace…", "secondary-shift-n", false).on_click(
                            cx.listener(|this, _, w, cx| this.start_new_workspace(w, cx)),
                        ),
                    ),
            )
            .children(self.open_error.clone().map(|e| {
                div()
                    .max_w(px(560.))
                    .text_size(px(12.))
                    .text_color(t.deleted)
                    .child(e)
            }))
            .when(!self.recent.workspaces.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .items_start()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(t.text_placeholder)
                                .child("Recent workspaces"),
                        )
                        .children(self.recent.workspaces.clone().into_iter().enumerate().map(
                            |(i, w)| {
                                let path = std::path::PathBuf::from(&w);
                                let missing = !path.is_dir();
                                let forget = w.clone();
                                let name = path
                                    .file_name()
                                    .map_or(w.clone(), |n| n.to_string_lossy().into_owned());
                                let group: SharedString = format!("recent-{i}").into();
                                row()
                                    .id(("recent", i))
                                    .group(group.clone())
                                    .w(px(520.))
                                    .px_2()
                                    .h(px(28.))
                                    .gap_2()
                                    .rounded(px(4.))
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .hover(|s| s.bg(t.element_hover))
                                    .when(missing, |d| d.text_color(t.text_placeholder))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.open_workspace(&path, w, cx)
                                    }))
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .when(!missing, |d| d.text_color(t.accent))
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .font_family(BUFFER_FONT)
                                            .text_color(t.text_placeholder)
                                            .child(middle_truncate(&w, 56)),
                                    )
                                    .when(missing, |d| d.child("Not found"))
                                    .child(
                                        div()
                                            .id(("recent-forget", i))
                                            .px_1()
                                            .invisible()
                                            .group_hover(group, |s| s.visible())
                                            .text_color(t.text_muted)
                                            .hover(|s| s.text_color(t.text))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.forget_recent(&forget);
                                                cx.stop_propagation();
                                                cx.notify();
                                            }))
                                            .child("×"),
                                    )
                            },
                        )),
                )
            })
    }
}

/// Shorten the middle of a long path: `/Users/a/…/repo`.
pub(crate) fn middle_truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}
