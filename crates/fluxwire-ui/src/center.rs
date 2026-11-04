//! Tab bar, request editor, response pane, placeholder pages and the bottom dock.

use fluxwire_model::BlockKind;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, deferred, div, prelude::*, px,
};

use crate::{
    actions::ACTIONS,
    assets::BUFFER_FONT,
    dev_id::{DevClick, DevTag, Role},
    editor::{Lang, code_view},
    tabs::{Tab, TabKind},
    theme::Theme,
    ui::{icon, kbd_label, row, with_variables},
    workspace::{BottomTab, Col, METHODS, TabUi, Table, WorkspaceView},
    workspace_model::{OpenDoc, Target},
};

/// Group key of the tab at `i` for kind-grouping dividers: pinned tabs form their own group.
fn group_of(order: &[(bool, u8)], i: usize) -> (bool, u8) {
    let (pinned, kind) = order[i];
    (pinned, if pinned { 0 } else { kind })
}

const SUBTABS: [&str; 8] = [
    "Params", "Headers", "Body", "Auth", "Scripts", "Tests", "Docs", "Settings",
];
const WS_SUBTABS: [&str; 5] = ["Messages", "Headers", "Auth", "Scripts", "Settings"];

pub(crate) fn pill(t: &Theme, label: impl Into<gpui::SharedString>) -> gpui::Div {
    row()
        .px_2()
        .h(px(22.))
        .rounded(px(4.))
        .border_1()
        .border_color(t.border)
        .text_size(px(12.))
        .text_color(t.text_muted)
        .child(label.into())
}

pub(crate) fn button(
    t: &Theme,
    id: &'static str,
    label: &'static str,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    row()
        .id(id)
        .px_3()
        .h(px(26.))
        .rounded(px(4.))
        .cursor_pointer()
        .text_size(px(12.))
        .border_1()
        .border_color(t.border)
        .when(primary, |d| {
            d.bg(t.accent)
                .border_color(t.accent)
                .text_color(t.editor)
                .font_weight(gpui::FontWeight::SEMIBOLD)
        })
        .when(!primary, |d| {
            let h = t.element_hover;
            d.hover(move |s| s.bg(h))
        })
        .child(label)
}

impl WorkspaceView {
    pub(crate) fn render_center(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.scope.tabs.active().cloned();
        let ui = tab
            .as_ref()
            .and_then(|t| self.scope.tab_ui.get(&t.id))
            .cloned();
        let content = match (&tab, &ui) {
            (Some(tab), Some(ui)) => match &tab.kind {
                TabKind::Http { .. } => self.render_request(t, tab, ui, cx).into_any_element(),
                TabKind::WebSocket => self.render_websocket(t, tab, ui, cx).into_any_element(),
                TabKind::FolderRun => self.render_run_tab(t, tab, cx),
                TabKind::Docs => self.render_docs(t).into_any_element(),
                TabKind::Environment => self.render_environment(t, tab, cx).into_any_element(),
                TabKind::History => self.render_history_tab(t, tab, cx),
                TabKind::Sync => self.render_sync_tab(t).into_any_element(),
                TabKind::Keybindings => self.render_keybindings(t, cx),
            },
            _ => self.render_empty(t, cx).into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_tab_bar(t, cx))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .when(self.scope.bottom_open, |d| {
                d.child(self.render_bottom(t, cx))
            })
    }

    fn render_empty(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let has_workspace = self.scope.workspace.is_some();
        if self
            .scope
            .workspace
            .as_ref()
            .is_some_and(|w| w.collections.is_empty())
        {
            return self.render_workspace_empty(t, cx).into_any_element();
        }
        // (icon, label, palette action label, shown only with a workspace)
        let rows: [(&str, &str, &str, bool); 5] = [
            ("plus", "New request", "workspace: new request", true),
            (
                "search",
                "Search requests and files",
                "quick search: open",
                false,
            ),
            (
                "terminal",
                "Command palette",
                "command palette: toggle",
                false,
            ),
            ("folder-open", "Open workspace", "workspace: open…", false),
            ("columns-2", "Toggle sidebar", "view: toggle sidebar", false),
        ];
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .child(
                div()
                    .size(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(10.))
                    .bg(t.element_hover)
                    .child(icon("layers", 24., t.text_muted)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child("No tab open"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(t.text_muted)
                            .child("Open a request from the sidebar, or start with one of these."),
                    ),
            )
            .child(div().w(px(340.)).flex().flex_col().gap(px(2.)).children(
                rows.into_iter().enumerate().filter_map(
                    |(i, (glyph, label, action, needs_workspace))| {
                        if needs_workspace && !has_workspace {
                            return None;
                        }
                        let ix = ACTIONS.iter().position(|a| a.label == action)?;
                        let binding = ACTIONS[ix].binding.map(kbd_label).unwrap_or_default();
                        let hover = t.element_hover;
                        Some(
                            row()
                                .id(("empty-action", i))
                                .h(px(34.))
                                .px_3()
                                .gap_3()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(14.))
                                .text_color(t.text)
                                .hover(move |s| s.bg(hover))
                                .dev_click(
                                    "center.render_empty",
                                    Role::Button,
                                    "center.render_empty",
                                    true,
                                    cx.listener(move |_, _, w, cx| {
                                        w.dispatch_action((ACTIONS[ix].build)(), cx)
                                    }),
                                )
                                .child(icon(glyph, 16., t.text_muted))
                                .child(div().flex_1().child(label))
                                .child(
                                    div()
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(t.border)
                                        .font_family(BUFFER_FONT)
                                        .text_size(px(11.))
                                        .text_color(t.text_muted)
                                        .child(binding),
                                ),
                        )
                    },
                ),
            ))
            .into_any_element()
    }

    fn render_tab_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.scope.tabs.active_id();
        let tabs: Vec<Tab> = self
            .scope
            .tabs
            .display_order()
            .into_iter()
            .cloned()
            .collect();
        let order: Vec<(bool, u8)> = tabs
            .iter()
            .map(|t| (t.pinned, t.kind.group_order()))
            .collect();
        row()
            .id("tab-bar")
            .h(px(28.))
            .flex_none()
            .bg(t.surface)
            .overflow_x_scroll()
            .children(tabs.into_iter().enumerate().map(|(n, tab)| {
                let id = tab.id;
                let is_active = active == Some(id);
                // With grouping on, a heavier divider marks where the kind changes.
                let group_start = self.scope.tabs.group_by_kind
                    && n > 0
                    && group_of(&order, n) != group_of(&order, n - 1);
                let color = t.method_color(tab.kind.color_key());
                let glyph_color = if tab.kind.color_key().is_empty() {
                    t.text_muted
                } else {
                    color
                };
                let hover = t.element_hover;
                row()
                    .id(("tab", id as usize))
                    .h_full()
                    .px_2()
                    .gap_2()
                    .flex_none()
                    .border_r_1()
                    .border_color(t.border_variant)
                    .when(group_start, |d| d.border_l_2().border_color(t.border))
                    .text_size(px(14.))
                    .cursor_pointer()
                    // Zed: the active tab shares the editor background and drops the bottom
                    // border, so it reads as one surface with the content below it.
                    .when(is_active, |d| d.bg(t.editor).text_color(t.text))
                    .when(!is_active, |d| {
                        d.border_b_1()
                            .border_color(t.border)
                            .text_color(t.text_muted)
                            .hover(move |s| s.bg(hover))
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            this.scope.tab_menu = Some((id, ev.position));
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_family(BUFFER_FONT)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(glyph_color)
                            .child(tab.kind.glyph()),
                    )
                    .child(
                        div()
                            .when(tab.preview, |d| d.italic())
                            .child(tab.title.clone()),
                    )
                    .child(
                        div()
                            .id(("tab-close", id as usize))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .hover({
                                let s = t.element_selected;
                                move |st| st.bg(s)
                            })
                            .dev_click(
                                "center.render_tab_bar",
                                Role::Button,
                                "center.render_tab_bar",
                                true,
                                cx.listener(move |this, _, w, cx| {
                                    // A pinned tab shows a pin here: clicking it unpins.
                                    if this.scope.tabs.get(id).is_some_and(|t| t.pinned) {
                                        this.scope.tabs.toggle_pin(id);
                                        this.session_dirty = true;
                                    } else if this.is_env_tab(id) {
                                        this.close_env_tab(id, w, cx);
                                    } else {
                                        this.scope.tabs.close(id);
                                    }
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child(if tab.dirty {
                                div()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(t.modified)
                                    .into_any_element()
                            } else if tab.pinned {
                                icon("pin", 12., t.text_muted).into_any_element()
                            } else {
                                icon("x", 12., t.text_placeholder).into_any_element()
                            }),
                    )
                    .dev_click(
                        &tab.key,
                        Role::Tab,
                        &tab.title,
                        true,
                        cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                            this.scope.tabs.activate(id);
                            if ev.click_count() >= 2 {
                                this.scope.tabs.make_permanent(id);
                            }
                            cx.notify();
                        }),
                    )
            }))
            .child(div().flex_1().h_full().border_b_1().border_color(t.border))
    }

    pub(crate) fn tab_strip(
        &self,
        t: &Theme,
        prefix: &'static str,
        labels: Vec<(String, usize)>,
        active: usize,
        set: fn(&mut TabUi, usize),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        row()
            .px_2()
            .gap(px(2.))
            .flex_none()
            .border_b_1()
            .border_color(t.border_variant)
            .children(labels.into_iter().enumerate().map(|(i, (label, count))| {
                let on = i == active;
                let hover_text = t.text;
                div()
                    .id((prefix, i))
                    .px_2()
                    .py(px(6.))
                    .mb(px(-1.))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if on {
                        t.accent
                    } else {
                        gpui::transparent_black()
                    })
                    .text_color(if on { t.text } else { t.text_muted })
                    .hover(move |s| s.text_color(hover_text))
                    .dev_click(
                        "center.tab_strip",
                        Role::Button,
                        "center.tab_strip",
                        true,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(ui) = this.active_ui() {
                                set(ui, i);
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        row()
                            .gap_1p5()
                            .child(label)
                            // Count badge: a small filled pill, not part of the label text.
                            .when(count > 0, |d| {
                                d.child(
                                    div()
                                        .min_w(px(16.))
                                        .h(px(16.))
                                        .px(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(t.element_selected)
                                        .text_size(px(11.))
                                        .text_color(if on { t.text } else { t.text_muted })
                                        .child(count.to_string()),
                                )
                            }),
                    )
            }))
    }

    /// The verb dropdown: a popover under the method button, painted above everything else and
    /// dismissed by choosing an item, clicking outside, or Esc.
    fn method_menu_view(
        &self,
        t: &Theme,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        deferred(
            div()
                .id("method-menu")
                .absolute()
                .top(px(34.))
                .left_0()
                .w(px(140.))
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
                    this.scope.method_menu = false;
                    cx.notify();
                }))
                .children(METHODS.iter().enumerate().map(|(i, m)| {
                    let hover = t.element_hover;
                    row()
                        .id(("method-item", i))
                        .h(px(26.))
                        .mx_1()
                        .px_2()
                        .gap_2()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .font_family(BUFFER_FONT)
                        .text_size(px(13.))
                        .text_color(t.method_color(m))
                        .when(i == selected, |d| d.bg(t.element_selected))
                        .hover(move |s| s.bg(hover))
                        .dev_click(
                            "method-menu",
                            Role::Button,
                            "method-menu",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                this.scope.method_menu = false;
                                this.set_method(i, cx);
                                cx.notify();
                            }),
                        )
                        .child(*m)
                })),
        )
        .with_priority(1)
    }

    fn url_bar(
        &self,
        t: &Theme,
        ui: &TabUi,
        method: &str,
        url: &str,
        sendable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let method_label = method.to_string();
        let sending = self.is_sending();
        let key = self.active_key().unwrap_or_default();
        let look = self.var_look(&key);
        let url_input = self.url_input(&key, cx);
        if url_input.read(cx).text() != url {
            url_input.update(cx, |i, cx| i.set_text(url, cx));
        }
        let decorations = crate::ui::url_decorations(url, t, &look);
        let url_focus = gpui::Focusable::focus_handle(url_input.read(cx), cx);
        let url_focused = self.focused.as_ref() == Some(&url_focus);
        let completing = self.scope.envs.completion.is_some();
        url_input.update(cx, |i, cx| {
            i.set_decorations(decorations, cx);
            i.set_consume_vertical(!completing);
        });
        row()
            .flex_none()
            .gap_2()
            .px_2()
            .py_2()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .relative()
                    .child(
                        row()
                            .id("method")
                            .h(px(30.))
                            .px_3()
                            .gap_1()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(if self.scope.method_menu {
                                t.accent
                            } else {
                                t.border
                            })
                            .bg(t.surface)
                            .cursor_pointer()
                            .font_family(BUFFER_FONT)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.method_color(&method_label))
                            .dev_click(
                                "method",
                                Role::Button,
                                "method",
                                true,
                                cx.listener(|this, _, _, cx| {
                                    this.scope.method_menu = !this.scope.method_menu;
                                    cx.notify();
                                }),
                            )
                            .child(method_label)
                            .child(icon("chevron-down", 12., t.text_muted)),
                    )
                    .when(self.scope.method_menu, |d| {
                        d.child(self.method_menu_view(t, ui.method, cx))
                    }),
            )
            .child(
                row()
                    .id("url")
                    .on_key_down(cx.listener(|this, ev, _, cx| this.on_url_key(ev, cx)))
                    .flex_1()
                    .h(px(30.))
                    .px_3()
                    .overflow_hidden()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if url_focused { t.accent } else { t.border })
                    .bg(t.editor)
                    .font_family(BUFFER_FONT)
                    .text_size(px(14.))
                    .child(div().flex_1().child(url_input).dev_tag(
                        "toolbar.url",
                        Role::Input,
                        "URL",
                        true,
                    )),
            )
            .child(
                button(
                    t,
                    "send",
                    if sending { "Cancel" } else { "Send" },
                    sendable || sending,
                )
                .h(px(30.))
                .px_4()
                .when(!sendable && !sending, |d| d.opacity(0.5).cursor_default())
                .dev_click(
                    "toolbar.send",
                    Role::Button,
                    if sending { "Cancel" } else { "Send" },
                    sendable || sending,
                    cx.listener(|this, _, _, cx| this.send_or_cancel(cx)),
                ),
            )
            .child(
                button(t, "text-mode", "Open as text", false)
                    .h(px(30.))
                    .dev_click(
                        "toolbar.text-mode",
                        Role::Button,
                        "Open as text",
                        true,
                        cx.listener(|this, _, _, cx| this.toggle_text_mode(cx)),
                    ),
            )
    }

    fn banner(&self, t: &Theme, text: String, color: gpui::Hsla) -> gpui::Div {
        row()
            .flex_none()
            .px_3()
            .py_1()
            .gap_2()
            .border_b_1()
            .border_color(t.border_variant)
            .bg(gpui::Hsla { a: 0.12, ..color })
            .text_size(px(12.))
            .text_color(t.text)
            .child(div().size(px(7.)).rounded_full().bg(color))
            .child(text)
    }

    /// Banners above the editor: unsupported features, disk conflicts, missing file, other content.
    fn doc_banners(&self, t: &Theme, doc: &OpenDoc, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = vec![];
        if !doc.parsed.unsupported.is_empty() {
            let names = doc.parsed.unsupported.join(", ");
            out.push(
                self.banner(
                    t,
                    format!(
                        "This request needs support for {names}, which this version does not have. You can edit and save it, but not send it."
                    ),
                    t.modified,
                )
                .into_any_element(),
            );
        }
        if self.scope.trust_prompt.is_some()
            && self.scope.trust_prompt == self.scope.tabs.active_id()
        {
            out.push(
                self.banner(
                    t,
                    "This collection contains scripts. Scripts can send requests and read variables. Trust this collection?".into(),
                    t.modified,
                )
                .child(div().flex_1())
                .child(
                    button(t, "trust-yes", "Trust", true).dev_click("trust-yes", Role::Button, "trust-yes", true, 
                        cx.listener(|this, _, _, cx| this.set_collection_trust(true, cx)),
                    ),
                )
                .child(
                    button(t, "trust-no", "Not now", false).dev_click("trust-no", Role::Button, "trust-no", true, cx.listener(
                        |this, _, _, cx| {
                            this.scope.trust_prompt = None;
                            cx.notify();
                        },
                    )),
                )
                .into_any_element(),
            );
        }
        if doc.conflict.is_some() {
            out.push(
                self.banner(t, "File changed on disk.".into(), t.deleted)
                    .child(div().flex_1())
                    .child(button(t, "conflict-reload", "Reload", false).dev_click(
                        "conflict-reload",
                        Role::Button,
                        "conflict-reload",
                        true,
                        cx.listener(|this, _, _, cx| this.resolve_conflict(true, cx)),
                    ))
                    .child(button(t, "conflict-keep", "Keep mine", false).dev_click(
                        "conflict-keep",
                        Role::Button,
                        "conflict-keep",
                        true,
                        cx.listener(|this, _, _, cx| this.resolve_conflict(false, cx)),
                    ))
                    .into_any_element(),
            );
        }
        if doc.missing {
            out.push(
                self.banner(
                    t,
                    "This file was deleted or moved on disk. Save to recreate it.".into(),
                    t.deleted,
                )
                .into_any_element(),
            );
        }
        let other = doc.parsed.other_content();
        if !other.is_empty() {
            out.push(
                row()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child("Other content")
                    .children(other.into_iter().map(|o| pill(t, o)))
                    .into_any_element(),
            );
        }
        out
    }

    /// An editable, highlighted code surface bound to `target` of document `key`.
    #[allow(clippy::too_many_arguments)]
    fn editor_surface(
        &self,
        id: &'static str,
        key: &str,
        text: &str,
        lang: Lang,
        target: Target,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ed = self.code_editor(key, target, lang, text, cx);
        div()
            .id(id)
            .flex_1()
            .min_h_0()
            .px_2()
            .py_1()
            .child(ed)
            .into_any_element()
    }

    fn render_request(
        &self,
        t: &Theme,
        tab: &Tab,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&tab.key)) else {
            return self.render_empty(t, cx).into_any_element();
        };
        let key = tab.key.clone();
        let req = doc.parsed.request();
        let method = req.map_or("GET".to_string(), |r| r.method.clone());
        let url = req.map_or(String::new(), |r| r.target.clone());
        let params_on = fluxwire_format::query_rows(&doc.parsed)
            .iter()
            .filter(|q| q.enabled)
            .count();
        let headers = req.map_or(0, |r| r.headers.len());
        let has_body = !doc.target_text(Target::Body).is_empty();
        let count_of = |k: BlockKind| doc.parsed.blocks_of(k).count();
        let scripts = count_of(BlockKind::Pre) + count_of(BlockKind::Post);
        let tests = count_of(BlockKind::Test) + usize::from(count_of(BlockKind::Expect) > 0);
        let labels: Vec<(String, usize)> = SUBTABS
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let n = match i {
                    0 => params_on,
                    1 => headers,
                    2 => usize::from(has_body),
                    4 => scripts,
                    5 => tests,
                    _ => 0,
                };
                ((*l).to_string(), if i == 2 { 0 } else { n })
            })
            .collect();
        let mut editor = div()
            .key_context("RequestEditor")
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .overflow_hidden();
        editor.style().flex_grow = Some(self.scope.split_frac);
        editor.style().flex_basis = Some(gpui::relative(0.).into());
        if doc.text_mode {
            editor = editor
                .child(
                    row()
                        .flex_none()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .border_b_1()
                        .border_color(t.border_variant)
                        .child(
                            div()
                                .flex_1()
                                .text_color(t.text_muted)
                                .child(format!("{} (raw .flux.md)", key)),
                        )
                        .child(
                            button(t, "form-mode", "Form view", false)
                                .h(px(30.))
                                .dev_click(
                                    "form-mode",
                                    Role::Button,
                                    "form-mode",
                                    true,
                                    cx.listener(|this, _, _, cx| this.toggle_text_mode(cx)),
                                ),
                        ),
                )
                .children(self.doc_banners(t, doc, cx))
                .child(self.editor_surface(
                    "text-editor",
                    &doc.key,
                    &doc.text,
                    Lang::FluxMd,
                    Target::Text,
                    cx,
                ));
        } else {
            editor = editor
                .child(self.url_bar(t, ui, &method, &url, doc.parsed.runnable(), cx))
                .children(self.completion_strip(t, cx))
                .children(self.doc_banners(t, doc, cx))
                .child(self.tab_strip(t, "sub", labels, ui.sub, |u, i| u.sub = i, cx))
                .child(
                    div()
                        .id("editor-scroll")
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .child(self.request_sub_content(t, doc, &key, ui, cx)),
                );
        }
        let response = self.render_response(t, ui, cx);
        let vertical = self.scope.split_vertical;
        let frac = self.scope.split_frac;
        let mut response_pane = div();
        response_pane.style().flex_grow = Some(1.0 - frac);
        response_pane.style().flex_basis = Some(gpui::relative(0.).into());
        let divider = div()
            .id("split-divider")
            .flex_none()
            .when(vertical, |d| d.h(px(5.)).w_full().cursor_row_resize())
            .when(!vertical, |d| d.w(px(5.)).h_full().cursor_col_resize())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    let p = if this.scope.split_vertical {
                        f32::from(ev.position.y)
                    } else {
                        f32::from(ev.position.x)
                    };
                    this.resizing_split = Some((p, this.scope.split_frac));
                    cx.notify();
                }),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(vertical, |d| d.flex_col())
            .when(!vertical, |d| d.flex_row())
            .child(editor)
            .child(divider)
            .child(
                response_pane
                    .overflow_hidden()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .border_color(t.border)
                    .when(vertical, |d| d.border_t_1())
                    .when(!vertical, |d| d.border_l_1())
                    .child(response),
            )
            .into_any_element()
    }

    /// Key/value table whose cells edit the document in place.
    fn editable_table(
        &self,
        t: &Theme,
        table: Table,
        key: &str,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let look = self.var_look(key);
        let rows = self.table_rows(key, table);
        let editing = self
            .scope
            .cell
            .as_ref()
            .filter(|c| c.table == table && c.key == key)
            .cloned();
        let n = rows.len();
        let cell_el = |idx: usize, col: Col, text: &str, enabled: bool, cx: &mut Context<Self>| {
            let active = editing
                .as_ref()
                .is_some_and(|c| c.row == idx && c.col == col);
            let shown = if active {
                editing
                    .as_ref()
                    .map_or(text.to_string(), |c| c.text.clone())
            } else {
                text.to_string()
            };
            let completing = self.scope.envs.completion.is_some();
            let base = row()
                .id((
                    if col == Col::Key { "cell-k" } else { "cell-v" },
                    idx + usize::from(table == Table::Headers) * 1000,
                ))
                .h(px(24.))
                .px_1()
                .rounded(px(3.))
                .overflow_hidden()
                .cursor_text()
                .border_1()
                .border_color(if active {
                    t.accent
                } else {
                    gpui::transparent_black()
                })
                .when(!enabled, |d| d.text_color(t.text_placeholder))
                .when(col == Col::Key, |d| d.w(px(200.)).flex_none())
                .when(col == Col::Value, |d| d.flex_1().min_w_0());
            if active {
                let input = editing.as_ref().map(|c| c.input.clone());
                let decorations = crate::ui::url_decorations(&shown, t, &look);
                if let Some(i) = &input {
                    i.update(cx, |i, cx| {
                        i.set_decorations(decorations, cx);
                        i.set_consume_vertical(!completing);
                    });
                }
                base.on_key_down(cx.listener(|this, ev, w, cx| this.on_cell_key(ev, w, cx)))
                    .children(input)
            } else {
                base.dev_click(
                    "center.editable_table",
                    Role::Button,
                    "center.editable_table",
                    true,
                    cx.listener(move |this, _, w, cx| this.start_cell(table, idx, col, w, cx)),
                )
                .children(with_variables(&shown, t, Some(&look)))
            }
        };
        let mut all: Vec<(bool, String, String)> = rows;
        if let Some(c) = &editing
            && c.row >= n
        {
            all.push((
                true,
                if c.col == Col::Key {
                    c.text.clone()
                } else {
                    String::new()
                },
                if c.col == Col::Value {
                    c.text.clone()
                } else {
                    String::new()
                },
            ));
        }
        let new_row_idx = n;
        div()
            .id(id)
            .flex()
            .flex_col()
            .child(
                row()
                    .h(px(24.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .text_size(px(11.))
                    .text_color(t.text_placeholder)
                    .child(div().w(px(16.)).flex_none())
                    .child(div().w(px(200.)).flex_none().child("Key"))
                    .child(div().flex_1().child("Value"))
                    .child(div().w(px(16.)).flex_none()),
            )
            .children(all.into_iter().enumerate().map(|(i, (enabled, k, v))| {
                let id_off = i + usize::from(table == Table::Headers) * 1000;
                row()
                    .h(px(28.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .child(
                        div()
                            .id(("row-check", id_off))
                            .size(px(14.))
                            .flex_none()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(t.border)
                            .cursor_pointer()
                            .when(enabled, |d| d.bg(t.accent).border_color(t.accent))
                            .dev_click(
                                id,
                                Role::Button,
                                id,
                                true,
                                cx.listener(move |this, _, _, cx| this.toggle_row(table, i, cx)),
                            ),
                    )
                    .child(cell_el(i, Col::Key, &k, enabled, cx))
                    .child(cell_el(i, Col::Value, &v, enabled, cx))
                    .child(
                        div()
                            .id(("row-del", id_off))
                            .w(px(16.))
                            .flex_none()
                            .cursor_pointer()
                            .dev_click(
                                "center.editable_table",
                                Role::Button,
                                "center.editable_table",
                                true,
                                cx.listener(move |this, _, _, cx| this.delete_row(table, i, cx)),
                            )
                            .child(icon("x", 12., t.text_placeholder)),
                    )
            }))
            .child(
                row()
                    .id(("add-row", usize::from(table == Table::Headers)))
                    .h(px(28.))
                    .px_3()
                    .gap_1()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .dev_click(
                        "center.editable_table",
                        Role::Button,
                        "center.editable_table",
                        true,
                        cx.listener(move |this, _, w, cx| {
                            this.start_cell(table, new_row_idx, Col::Key, w, cx)
                        }),
                    )
                    .child(icon("plus", 12., t.text_muted))
                    .child(if table == Table::Headers {
                        "Add header"
                    } else {
                        "Add parameter"
                    }),
            )
            .into_any_element()
    }

    /// Script/test editor for the first block of `kind`, or an "add" button.
    fn block_editor(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        kind: BlockKind,
        info: &'static str,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match doc.find_block(kind.clone(), None) {
            Some(i) => {
                let b = &doc.parsed.blocks[i];
                let lang = if b.info.language == "js" {
                    Lang::Js
                } else {
                    Lang::Ts
                };
                let text = doc.target_text(Target::Block(i));
                self.editor_surface(id, &doc.key, &text, lang, Target::Block(i), cx)
            }
            None => div()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .items_start()
                .text_color(t.text_muted)
                .child(format!("No `{info}` block in this file."))
                .child(button(t, id, "Add block", true).dev_click(
                    id,
                    Role::Button,
                    id,
                    true,
                    cx.listener(move |this, _, _, cx| this.add_block(kind.clone(), info, cx)),
                ))
                .into_any_element(),
        }
    }

    /// Tests tab: the declarative `http expect` block, or the `test` script (SPEC §7.7, §11).
    /// Docs sub-tab: *Preview* (rendered Markdown, read-only, the default) or *Source* (the
    /// Markdown editor).
    fn render_docs_tab(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let source = ui.docs_source;
        let toggle =
            row()
                .gap_2()
                .p_2()
                .flex_none()
                .children([("Preview", false), ("Source", true)].map(|(l, s)| {
                    button(
                        t,
                        if s { "docs-source" } else { "docs-preview" },
                        l,
                        source == s,
                    )
                    .dev_click(
                        "center.render_docs_tab",
                        Role::Button,
                        "center.render_docs_tab",
                        true,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(ui) = this.active_ui() {
                                ui.docs_source = s;
                            }
                            cx.notify();
                        }),
                    )
                }));
        let text = doc.target_text(Target::Docs);
        let body: AnyElement = if source {
            self.editor_surface(
                "docs-editor",
                &doc.key,
                &text,
                Lang::FluxMd,
                Target::Docs,
                cx,
            )
        } else {
            self.render_docs_preview(t, &doc.key, &text, cx)
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toggle)
            .child(body)
            .into_any_element()
    }

    fn render_tests_tab(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scripts = ui.tests_script;
        let toggle = row().gap_2().p_2().flex_none().children(
            [("Expect", false), ("Test script", true)].map(|(l, s)| {
                button(
                    t,
                    if s { "tests-script" } else { "tests-expect" },
                    l,
                    scripts == s,
                )
                .dev_click(
                    "center.render_tests_tab",
                    Role::Button,
                    "center.render_tests_tab",
                    true,
                    cx.listener(move |this, _, _, cx| {
                        if let Some(ui) = this.active_ui() {
                            ui.tests_script = s;
                        }
                        cx.notify();
                    }),
                )
            }),
        );
        let body: AnyElement = if scripts {
            self.block_editor(t, doc, BlockKind::Test, "ts test", "add-test", cx)
        } else if doc.find_block(BlockKind::Expect, None).is_some() {
            let status_chips = row()
                .gap_2()
                .px_2()
                .pb_2()
                .flex_none()
                .child(
                    div()
                        .text_color(t.text_placeholder)
                        .text_size(px(12.))
                        .child("Status"),
                )
                .children(["200", "201", "204", "2xx", "4xx", "5xx", "*"].map(|c| {
                    button(t, c, c, false).dev_click(
                        c,
                        Role::Button,
                        c,
                        true,
                        cx.listener(move |this, _, _, cx| this.set_expect_status(c, cx)),
                    )
                }));
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(status_chips)
                .child(self.block_editor(
                    t,
                    doc,
                    BlockKind::Expect,
                    "http expect",
                    "add-expect",
                    cx,
                ))
                .into_any_element()
        } else {
            div()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .items_start()
                .text_color(t.text_muted)
                .child("No `http expect` block. It lists what the response must contain: status, headers, and a partial JSON body.")
                .child(
                    row()
                        .gap_2()
                        .child(button(t, "add-expect", "Add expect", true).dev_click("add-expect", Role::Button, "add-expect", true, 
                            cx.listener(|this, _, _, cx| this.add_expect(false, cx)),
                        ))
                        .child(button(t, "add-expect-last", "From last response", false).dev_click("add-expect-last", Role::Button, "add-expect-last", true, 
                            cx.listener(|this, _, _, cx| this.add_expect(true, cx)),
                        )),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toggle)
            .child(body)
            .into_any_element()
    }

    pub(crate) fn request_sub_content(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        key: &str,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match ui.sub {
            0 => self.editable_table(t, Table::Params, key, "params", cx),
            1 => self.editable_table(t, Table::Headers, key, "headers", cx),
            2 => {
                let text = doc.target_text(Target::Body);
                let lang = body_lang(doc, &text);
                self.editor_surface("body-editor", &doc.key, &text, lang, Target::Body, cx)
            }
            3 => self.render_auth_tab(t, key, cx),
            4 => {
                let (kind, info) = if ui.post_script {
                    (BlockKind::Post, "ts post")
                } else {
                    (BlockKind::Pre, "ts pre")
                };
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(
                        row()
                            .gap_2()
                            .p_2()
                            .flex_none()
                            .children([("Pre", false), ("Post", true)].map(|(l, post)| {
                                let on = ui.post_script == post;
                                button(t, if post { "post" } else { "pre" }, l, on).dev_click(
                                    "center.request_sub_content",
                                    Role::Button,
                                    "center.request_sub_content",
                                    true,
                                    cx.listener(move |this, _, _, cx| {
                                        if let Some(ui) = this.active_ui() {
                                            ui.post_script = post;
                                        }
                                        cx.notify();
                                    }),
                                )
                            }))
                            .child(div().flex_1())
                            .when_some(doc.find_block(kind.clone(), None), |d, i| {
                                let lang: &'static str =
                                    if doc.parsed.blocks[i].info.language == "js" {
                                        "js"
                                    } else {
                                        "ts"
                                    };
                                let k = kind.clone();
                                d.child(button(t, "script-lang", lang, false).dev_click(
                                    "script-lang",
                                    Role::Button,
                                    "script-lang",
                                    true,
                                    cx.listener(move |this, _, _, cx| {
                                        this.toggle_block_language(k.clone(), cx)
                                    }),
                                ))
                            }),
                    )
                    .child(self.block_editor(
                        t,
                        doc,
                        kind,
                        info,
                        if ui.post_script {
                            "add-post"
                        } else {
                            "add-pre"
                        },
                        cx,
                    ))
                    .into_any_element()
            }
            5 => self.render_tests_tab(t, doc, ui, cx),
            6 => self.render_docs_tab(t, doc, ui, cx),
            _ => {
                let rows: Vec<(String, String)> =
                    match doc.parsed.front.get("settings").map(|e| &e.value) {
                        Some(fluxwire_model::FmValue::Map(m)) => m.clone(),
                        _ => vec![],
                    };
                div()
                    .flex()
                    .flex_col()
                    .p_3()
                    .gap_1()
                    .text_color(t.text_muted)
                    .child(if rows.is_empty() {
                        "No per-request settings; collection defaults apply.".to_string()
                    } else {
                        "Settings (front matter)".to_string()
                    })
                    .children(rows.into_iter().map(|(k, v)| {
                        div()
                            .font_family(BUFFER_FONT)
                            .text_size(px(12.))
                            .child(format!("{k}: {v}"))
                    }))
                    .into_any_element()
            }
        }
    }

    fn render_websocket(
        &self,
        t: &Theme,
        tab: &Tab,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&tab.key)) else {
            return self.render_empty(t, cx).into_any_element();
        };
        let key = tab.key.clone();
        let tab_id = tab.id;
        let look = self.var_look(&key);
        let url = doc
            .parsed
            .request()
            .map_or(String::new(), |r| r.target.clone());
        let headers = doc.parsed.request().map_or(0, |r| r.headers.len());
        let messages = doc.parsed.messages().len();
        let scripts = doc.parsed.blocks_of(BlockKind::Pre).count()
            + doc.parsed.blocks_of(BlockKind::Post).count();
        let labels: Vec<(String, usize)> = WS_SUBTABS
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    (*s).to_string(),
                    match i {
                        0 => messages,
                        1 => headers,
                        3 => scripts,
                        _ => 0,
                    },
                )
            })
            .collect();
        let connected = self.ws_connected(tab_id);
        let connecting = self.ws_connecting(tab_id);
        // The form's Auth/Scripts/Settings tabs are the HTTP ones; map the WS index onto them.
        let http_sub = |sub: usize| TabUi { sub, ..ui.clone() };
        let content = match ui.sub {
            0 => self.render_ws_messages(t, tab_id, ui.msg_sel, cx),
            1 => self.editable_table(t, Table::Headers, &key, "ws-headers", cx),
            2 => self.request_sub_content(t, doc, &key, &http_sub(3), cx),
            3 => self.request_sub_content(t, doc, &key, &http_sub(4), cx),
            _ => self.request_sub_content(t, doc, &key, &http_sub(7), cx),
        };
        let status = self.ws_status_line(t, tab_id);
        let log = self.render_session_log(t, crate::session::LogRef::Live(tab_id), status, cx);
        div()
            .key_context("RequestEditor")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                row()
                    .flex_none()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(
                        row()
                            .h(px(30.))
                            .px_3()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.surface)
                            .font_family(BUFFER_FONT)
                            .text_color(t.syntax_type)
                            .child("WS"),
                    )
                    .child(
                        row()
                            .id("url")
                            .flex_1()
                            .h(px(30.))
                            .px_3()
                            .overflow_hidden()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .font_family(BUFFER_FONT)
                            .text_size(px(14.))
                            .children(with_variables(&url, t, Some(&look))),
                    )
                    .child(
                        button(
                            t,
                            "ws-connect",
                            if connected {
                                "Disconnect"
                            } else if connecting {
                                "Cancel"
                            } else {
                                "Connect"
                            },
                            !connected,
                        )
                        .h(px(30.))
                        .px_4()
                        .dev_click(
                            "url",
                            Role::Button,
                            "url",
                            true,
                            cx.listener(|this, _, _, cx| this.ws_toggle(cx)),
                        ),
                    ),
            )
            .children(self.doc_banners(t, doc, cx))
            .child(self.tab_strip(t, "wsub", labels, ui.sub, |u, i| u.sub = i, cx))
            .child(
                div()
                    .id("ws-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(t.border_variant)
                    .child(log),
            )
            .into_any_element()
    }

    pub(crate) fn page(&self, t: &Theme, title: &str, subtitle: &str) -> gpui::Stateful<gpui::Div> {
        div()
            .id("page")
            .flex_1()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .child(div().text_color(t.text_muted).child(subtitle.to_string()))
    }

    fn render_docs(&self, t: &Theme) -> impl IntoElement {
        self.page(t, "payments", "Folder documentation")
            .child("The payments folder creates, reads, updates and cancels payments. Authenticate with a bearer token from the auth folder first.")
            .child(row().gap_2().child(pill(t, "POST /v1/payments")))
            .child(
                div().rounded(px(4.)).border_1().border_color(t.border_variant).child(code_view(
                    "{\n  \"amount\": 4200,\n  \"currency\": \"usd\"\n}",
                    Lang::Json,
                    t,
                )),
            )
    }

    fn render_sync_tab(&self, t: &Theme) -> impl IntoElement {
        self.page(t, "openapi/payments.yaml", "Spec link → payments folder")
            .child(
                row()
                    .gap_2()
                    .child(button(t, "pull", "Pull", true))
                    .child(button(t, "push", "Push", false)),
            )
            .children(self.mock.sync.iter().map(|c| {
                row()
                    .h(px(28.))
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(
                        div()
                            .text_color(if c.arrow == "↓" {
                                t.accent
                            } else {
                                t.modified
                            })
                            .child(c.arrow),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .font_family(BUFFER_FONT)
                            .text_size(px(12.))
                            .child(c.operation),
                    )
                    .child(div().text_color(t.text_muted).child(c.field))
            }))
    }

    fn render_bottom(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [
            (BottomTab::Console, "Console"),
            (BottomTab::Problems, "Problems"),
        ];
        let body: AnyElement = match self.scope.bottom_tab {
            BottomTab::Console => self.console_view(t),
            BottomTab::Problems => {
                let problems = self.all_problems();
                div()
                    .flex()
                    .flex_col()
                    .children(if problems.is_empty() {
                        vec![
                            div()
                                .p_3()
                                .text_color(t.text_muted)
                                .child("No problems found.")
                                .into_any_element(),
                        ]
                    } else {
                        problems
                            .into_iter()
                            .enumerate()
                            .map(|(i, p)| {
                                let color = match p.severity {
                                    fluxwire_model::Severity::Error => t.deleted,
                                    fluxwire_model::Severity::Warning => t.modified,
                                    fluxwire_model::Severity::Info => t.text_placeholder,
                                };
                                let target = p.clone();
                                row()
                                    .id(("problem", i))
                                    .h(px(26.))
                                    .px_3()
                                    .gap_3()
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .hover({
                                        let h = t.element_hover;
                                        move |s| s.bg(h)
                                    })
                                    .dev_click(
                                        "center.render_bottom",
                                        Role::Button,
                                        "center.render_bottom",
                                        true,
                                        cx.listener(move |this, _, w, cx| {
                                            this.goto_problem(&target, w, cx)
                                        }),
                                    )
                                    .child(icon("triangle-alert", 12., color))
                                    .child(p.message.clone())
                                    .child(
                                        div()
                                            .text_color(t.text_placeholder)
                                            .child(format!("{}:{}:{}", p.file, p.line, p.col)),
                                    )
                                    .child(
                                        div()
                                            .font_family(BUFFER_FONT)
                                            .text_color(t.text_placeholder)
                                            .child(p.code),
                                    )
                                    .into_any_element()
                            })
                            .collect()
                    })
                    .into_any_element()
            }
        };
        div()
            .h(px(self.scope.bottom_height))
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(
                div()
                    .id("bottom-resize")
                    .h(px(3.))
                    .w_full()
                    .flex_none()
                    .cursor_row_resize()
                    .hover({
                        let h = t.element_selected;
                        move |s| s.bg(h)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.resizing_bottom = true;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                row()
                    .h(px(28.))
                    .px_2()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .children(tabs.map(|(b, label)| {
                        let on = self.scope.bottom_tab == b;
                        div()
                            .id(label)
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(if on { t.text } else { t.text_muted })
                            .dev_click(
                                label,
                                Role::Button,
                                label,
                                true,
                                cx.listener(move |this, _, _, cx| {
                                    this.scope.bottom_tab = b;
                                    this.session_dirty = true;
                                    cx.notify();
                                }),
                            )
                            .child(label)
                    })),
            )
            .child(
                div()
                    .id("bottom-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

/// Prose of a document: everything outside front matter and fenced blocks.
/// Language of a request body: the `Content-Type` header, then the first character.
pub(crate) fn body_lang(doc: &OpenDoc, text: &str) -> Lang {
    let ct = doc
        .parsed
        .request()
        .and_then(|r| {
            r.headers
                .iter()
                .find(|h| h.enabled && h.name.eq_ignore_ascii_case("content-type"))
        })
        .map(|h| h.value.to_ascii_lowercase());
    match ct.as_deref() {
        Some(c) if c.contains("json") => Lang::Json,
        Some(c) if c.contains("xml") => Lang::Xml,
        Some(c) if c.contains("yaml") => Lang::Yaml,
        _ if text.trim_start().starts_with(['{', '[']) => Lang::Json,
        _ if text.trim_start().starts_with('<') => Lang::Xml,
        _ => Lang::Plain,
    }
}
