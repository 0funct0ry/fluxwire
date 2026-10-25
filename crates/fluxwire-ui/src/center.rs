//! Tab bar, request editor, response pane, placeholder pages and the bottom dock.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, deferred, div, prelude::*, px, relative,
};

use crate::{
    assets::BUFFER_FONT,
    editor::{Lang, code_view},
    mock::{Kv, MockRequest},
    tabs::{Tab, TabKind},
    theme::Theme,
    ui::{icon, row, with_variables},
    workspace::{BottomTab, METHODS, TabUi, Workspace},
};

const SUBTABS: [&str; 8] = [
    "Params", "Headers", "Body", "Auth", "Scripts", "Tests", "Docs", "Settings",
];
const WS_SUBTABS: [&str; 5] = ["Messages", "Headers", "Auth", "Scripts", "Settings"];
const RESP_TABS: [&str; 6] = ["Body", "Headers", "Cookies", "Timeline", "Tests", "Console"];

const PRE_SCRIPT: &str = "const body = fw.request.body;\nfw.request.headers.set(\"X-Signature\", sign(body));\nfw.vars.set(\"requestedAt\", Date.now());";
const POST_SCRIPT: &str =
    "fw.vars.set(\"paymentId\", fw.response.json().id);\nconsole.log(\"captured paymentId\");";
const TEST_SCRIPT: &str = "fw.test(\"creates a payment\", () => {\n  fw.expect(fw.response.status).toBe(201);\n  fw.expect(fw.response.json().currency).toBe(\"usd\");\n});";

pub fn kv_table(t: &Theme, rows: &[Kv], secret_col: bool) -> AnyElement {
    let t = t.clone();
    let head = |label: &'static str, w: Option<f32>| {
        let d = div()
            .text_size(px(11.))
            .text_color(t.text_placeholder)
            .child(label);
        match w {
            Some(w) => d.w(px(w)).flex_none(),
            None => d.flex_1(),
        }
    };
    div()
        .flex()
        .flex_col()
        .child(
            row()
                .h(px(24.))
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(t.border_variant)
                .child(head("", Some(16.)))
                .child(head("Key", Some(200.)))
                .child(head("Value", None))
                .when(secret_col, |d| d.child(head("Secret", Some(60.)))),
        )
        .children(rows.iter().map(|r| {
            row()
                .h(px(28.))
                .px_3()
                .gap_3()
                .border_b_1()
                .border_color(t.border_variant)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .when(!r.enabled, |d| d.text_color(t.text_placeholder))
                .child(
                    div()
                        .size(px(14.))
                        .flex_none()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(t.border)
                        .when(r.enabled, |d| d.bg(t.accent).border_color(t.accent)),
                )
                .child(div().w(px(200.)).flex_none().truncate().child(r.key))
                .child(
                    row()
                        .flex_1()
                        .truncate()
                        .children(with_variables(r.value, &t)),
                )
                .when(secret_col, |d| {
                    d.child(div().w(px(60.)).flex_none().text_color(t.text_muted).child(
                        if r.key.to_lowercase().contains("token") {
                            "yes"
                        } else {
                            ""
                        },
                    ))
                })
        }))
        .into_any_element()
}

fn pill(t: &Theme, label: impl Into<gpui::SharedString>) -> gpui::Div {
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

fn button(
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

impl Workspace {
    pub(crate) fn render_center(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tabs.active().cloned();
        let ui = tab.as_ref().and_then(|t| self.tab_ui.get(&t.id)).cloned();
        let content = match (&tab, &ui) {
            (Some(tab), Some(ui)) => match &tab.kind {
                TabKind::Http { .. } => self.render_request(t, tab, ui, cx).into_any_element(),
                TabKind::WebSocket => self.render_websocket(t, tab, ui, cx).into_any_element(),
                TabKind::FolderRun => self.render_folder_run(t).into_any_element(),
                TabKind::Docs => self.render_docs(t).into_any_element(),
                TabKind::Environment => self.render_environment(t).into_any_element(),
                TabKind::History => self.render_history_entry(t, tab).into_any_element(),
                TabKind::Sync => self.render_sync_tab(t).into_any_element(),
            },
            _ => self.render_empty(t).into_any_element(),
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_tab_bar(t, cx))
            .child(div().flex_1().min_h_0().flex().flex_col().child(content))
            .when(self.bottom_open, |d| d.child(self.render_bottom(t, cx)))
    }

    fn render_empty(&self, t: &Theme) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(t.text_muted)
            .child("No tab open")
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("Open a request from the Project panel, or press Cmd/Ctrl+K to search."),
            )
    }

    fn render_tab_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tabs.active_id();
        let tabs: Vec<Tab> = self.tabs.display_order().into_iter().cloned().collect();
        row()
            .id("tab-bar")
            .h(px(28.))
            .flex_none()
            .bg(t.surface)
            .overflow_x_scroll()
            .children(tabs.into_iter().map(|tab| {
                let id = tab.id;
                let is_active = active == Some(id);
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
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                        this.tabs.activate(id);
                        if ev.click_count() >= 2 {
                            this.tabs.make_permanent(id);
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.tabs.toggle_pin(id);
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tabs.close(id);
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .child(if tab.dirty {
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(t.text_muted)
                                    .into_any_element()
                            } else if tab.pinned {
                                icon("pin", 12., t.text_muted).into_any_element()
                            } else {
                                icon("x", 12., t.text_placeholder).into_any_element()
                            }),
                    )
            }))
            .child(div().flex_1().h_full().border_b_1().border_color(t.border))
    }

    fn tab_strip(
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ui) = this.active_ui() {
                            set(ui, i);
                        }
                        cx.notify();
                    }))
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
                    this.method_menu = false;
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.method_menu = false;
                            if let Some(ui) = this.active_ui() {
                                ui.method = i;
                            }
                            if let Some(id) = this.tabs.active_id() {
                                this.tabs.set_dirty(id, true);
                            }
                            cx.notify();
                        }))
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
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let method_label = method.to_string();
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
                            .border_color(if self.method_menu { t.accent } else { t.border })
                            .bg(t.surface)
                            .cursor_pointer()
                            .font_family(BUFFER_FONT)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.method_color(&method_label))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.method_menu = !this.method_menu;
                                cx.notify();
                            }))
                            .child(method_label)
                            .child(icon("chevron-down", 12., t.text_muted)),
                    )
                    .when(self.method_menu, |d| {
                        d.child(self.method_menu_view(t, ui.method, cx))
                    }),
            )
            .child(
                row()
                    .id("url")
                    .track_focus(&self.url_focus)
                    .on_key_down(cx.listener(|this, ev, _, cx| this.on_url_key(ev, cx)))
                    .flex_1()
                    .h(px(30.))
                    .px_3()
                    .overflow_hidden()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .focus({
                        let a = t.accent;
                        move |s| s.border_color(a)
                    })
                    .bg(t.editor)
                    .font_family(BUFFER_FONT)
                    .text_size(px(14.))
                    .children(with_variables(ui.url.text(), t))
                    .child(div().w(px(1.)).h(px(15.)).bg(t.accent)),
            )
            .child(button(t, "send", "Send", true).h(px(30.)).px_4())
    }

    fn render_request(
        &self,
        t: &Theme,
        tab: &Tab,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let r = self.mock.request(&tab.key).cloned();
        let Some(r) = r else {
            return self.render_empty(t).into_any_element();
        };
        let method = METHODS[ui.method];
        let params_on = r.params.iter().filter(|p| p.enabled).count();
        let labels: Vec<(String, usize)> = SUBTABS
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let n = match i {
                    0 => params_on,
                    1 => r.headers.len(),
                    2 => usize::from(!r.body.is_empty()),
                    4 => r.scripts,
                    5 => r.tests,
                    _ => 0,
                };
                // The Body tab has no count; the others badge a non-zero count.
                ((*l).to_string(), if i == 2 { 0 } else { n })
            })
            .collect();
        let editor = div()
            .key_context("RequestEditor")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(self.url_bar(t, ui, method, cx))
            .child(self.tab_strip(t, "sub", labels, ui.sub, |u, i| u.sub = i, cx))
            .child(
                div()
                    .id("editor-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.request_sub_content(t, &r, ui, cx)),
            );
        let response = self.render_response(t, &r, ui, cx);
        let vertical = self.split_vertical;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .when(vertical, |d| d.flex_col())
            .when(!vertical, |d| d.flex_row())
            .child(editor)
            .child(
                div()
                    .flex_1()
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

    fn request_sub_content(
        &self,
        t: &Theme,
        r: &MockRequest,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match ui.sub {
            0 => kv_table(t, &r.params, false),
            1 => kv_table(t, &r.headers, false),
            2 => code_view(r.body, Lang::Json, t),
            3 => div()
                .p_3()
                .text_color(t.text_muted)
                .child(format!("Auth: {}", r.auth))
                .into_any_element(),
            4 => div()
                .flex()
                .flex_col()
                .child(
                    row()
                        .gap_2()
                        .p_2()
                        .children([("Pre", false), ("Post", true)].map(|(l, post)| {
                            let on = ui.post_script == post;
                            button(t, if post { "post" } else { "pre" }, l, on).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(ui) = this.active_ui() {
                                        ui.post_script = post;
                                    }
                                    cx.notify();
                                }),
                            )
                        }))
                        .child(pill(t, "ts")),
                )
                .child(code_view(
                    if ui.post_script {
                        POST_SCRIPT
                    } else {
                        PRE_SCRIPT
                    },
                    Lang::Plain,
                    t,
                ))
                .into_any_element(),
            5 => code_view(TEST_SCRIPT, Lang::Plain, t),
            6 => div()
                .p_3()
                .text_color(t.text_muted)
                .child(r.docs)
                .into_any_element(),
            _ => div()
                .flex()
                .flex_col()
                .p_3()
                .gap_2()
                .text_color(t.text_muted)
                .child("Timeout: 30000 ms")
                .child("Follow redirects: yes")
                .child("Verify TLS: yes")
                .into_any_element(),
        }
    }

    fn render_response(
        &self,
        t: &Theme,
        r: &MockRequest,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let resp = &r.response;
        let labels: Vec<(String, usize)> =
            RESP_TABS.iter().map(|s| ((*s).to_string(), 0)).collect();
        let body: AnyElement = match ui.resp {
            0 => code_view(resp.body, Lang::Json, t),
            1 => kv_table(t, &resp.headers, false),
            2 => kv_table(t, &resp.cookies, false),
            3 => self.timeline(t, r),
            4 => div()
                .flex()
                .flex_col()
                .p_2()
                .children(resp.tests.iter().map(|(name, ok)| {
                    row()
                        .h(px(24.))
                        .gap_2()
                        .child(
                            div()
                                .text_color(if *ok { t.created } else { t.deleted })
                                .child(if *ok { "✓" } else { "✗" }),
                        )
                        .child(*name)
                }))
                .into_any_element(),
            _ => div()
                .flex()
                .flex_col()
                .p_2()
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .text_color(t.text_muted)
                .children(resp.console.iter().map(|l| div().child(*l)))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                row()
                    .h(px(30.))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.status_color(resp.status))
                            .child(format!("{} {}", resp.status, resp.status_text)),
                    )
                    .child(format!("{} ms", resp.time_ms))
                    .child(resp.size)
                    .child(resp.protocol),
            )
            .child(self.tab_strip(t, "resp", labels, ui.resp, |u, i| u.resp = i, cx))
            .child(
                div()
                    .id("resp-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    fn timeline(&self, t: &Theme, r: &MockRequest) -> AnyElement {
        let total: u32 = r.response.timeline.iter().map(|(_, ms)| ms).sum();
        let mut offset = 0u32;
        let rows: Vec<AnyElement> = r
            .response
            .timeline
            .iter()
            .map(|(label, ms)| {
                let left = offset as f32 / total as f32;
                let width = *ms as f32 / total as f32;
                offset += ms;
                row()
                    .h(px(24.))
                    .gap_3()
                    .text_size(px(12.))
                    .child(
                        div()
                            .w(px(70.))
                            .flex_none()
                            .text_color(t.text_muted)
                            .child(*label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(8.))
                            .relative()
                            .bg(t.element_hover)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .h_full()
                                    .left(relative(left))
                                    .w(relative(width))
                                    .rounded(px(2.))
                                    .bg(t.accent),
                            ),
                    )
                    .child(
                        div()
                            .w(px(50.))
                            .flex_none()
                            .text_right()
                            .font_family(BUFFER_FONT)
                            .text_color(t.text_muted)
                            .child(format!("{ms} ms")),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .p_3()
            .children(rows)
            .child(
                div()
                    .pt_2()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("Redirects: none"),
            )
            .into_any_element()
    }

    fn render_websocket(
        &self,
        t: &Theme,
        tab: &Tab,
        ui: &TabUi,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(r) = self.mock.request(&tab.key).cloned() else {
            return self.render_empty(t).into_any_element();
        };
        let labels: Vec<(String, usize)> =
            WS_SUBTABS.iter().map(|s| ((*s).to_string(), 0)).collect();
        let connected = ui.connected;
        let content = match ui.sub {
            0 => div()
                .flex()
                .flex_col()
                .child(
                    row()
                        .p_2()
                        .gap_2()
                        .child(button(t, "ws-send", "Send", false))
                        .child(button(t, "ws-save", "Save as message", false))
                        .child(pill(t, "json")),
                )
                .child(code_view(r.body, Lang::Json, t))
                .child(
                    row()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child("Message log")
                        .child(pill(t, "All"))
                        .child(pill(t, "Sent"))
                        .child(pill(t, "Received")),
                )
                .children(self.mock.ws_log.iter().map(|(dir, ts, payload)| {
                    row()
                        .h(px(24.))
                        .px_3()
                        .gap_3()
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .child(
                            div()
                                .text_color(if *dir == "out" { t.accent } else { t.created })
                                .child(if *dir == "out" { "↑" } else { "↓" }),
                        )
                        .child(div().text_color(t.text_placeholder).child(*ts))
                        .child(div().truncate().child(*payload))
                }))
                .into_any_element(),
            1 => kv_table(t, &r.headers, false),
            _ => div()
                .p_3()
                .text_color(t.text_muted)
                .child(WS_SUBTABS[ui.sub])
                .into_any_element(),
        };
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
                            .flex_1()
                            .h(px(30.))
                            .px_3()
                            .overflow_hidden()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .font_family(BUFFER_FONT)
                            .text_size(px(14.))
                            .children(with_variables(r.url, t)),
                    )
                    .child(
                        button(
                            t,
                            "ws-connect",
                            if connected { "Disconnect" } else { "Connect" },
                            !connected,
                        )
                        .h(px(30.))
                        .px_4()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(ui) = this.active_ui() {
                                ui.connected = !ui.connected;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(self.tab_strip(t, "wsub", labels, ui.sub, |u, i| u.sub = i, cx))
            .child(
                div()
                    .id("ws-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
            .into_any_element()
    }

    fn page(&self, t: &Theme, title: &str, subtitle: &str) -> gpui::Stateful<gpui::Div> {
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

    fn render_folder_run(&self, t: &Theme) -> impl IntoElement {
        let passed = self.mock.run.iter().filter(|r| r.passed).count();
        let failed = self.mock.run.len() - passed;
        self.page(t, "Run: payments", "Folder run (mock data)")
            .child(
                row()
                    .gap_2()
                    .child(pill(t, "Sequential"))
                    .child(pill(t, "Concurrency 1"))
                    .child(pill(
                        t,
                        format!("Env: {}", self.mock.environments[self.env].name),
                    ))
                    .child(pill(t, "Iterations 1"))
                    .child(button(t, "run-go", "Run", true)),
            )
            .child(
                row()
                    .gap_3()
                    .child(
                        div()
                            .text_color(t.created)
                            .child(format!("{passed} passed")),
                    )
                    .child(
                        div()
                            .text_color(t.deleted)
                            .child(format!("{failed} failed")),
                    ),
            )
            .children(self.mock.run.iter().map(|r| {
                row()
                    .h(px(28.))
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(
                        div()
                            .text_color(if r.passed { t.created } else { t.deleted })
                            .child(if r.passed { "✓" } else { "✗" }),
                    )
                    .child(
                        div()
                            .w(px(50.))
                            .font_family(BUFFER_FONT)
                            .text_size(px(11.))
                            .text_color(t.method_color(r.method))
                            .child(r.method),
                    )
                    .child(div().w(px(160.)).child(r.name))
                    .child(
                        div()
                            .w(px(200.))
                            .h(px(6.))
                            .relative()
                            .bg(t.element_hover)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .h_full()
                                    .w(relative(r.ms as f32 / 200.0))
                                    .rounded(px(2.))
                                    .bg(if r.passed { t.created } else { t.deleted }),
                            ),
                    )
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_color(t.text_muted)
                            .child(format!("{} ms", r.ms)),
                    )
            }))
    }

    fn render_docs(&self, t: &Theme) -> impl IntoElement {
        self.page(t, "payments", "Folder documentation")
            .child("The payments folder creates, reads, updates and cancels payments. Authenticate with a bearer token from the auth folder first.")
            .child(row().gap_2().child(pill(t, "POST /v1/payments")).child(pill(t, "3 tests")))
            .child(
                div().rounded(px(4.)).border_1().border_color(t.border_variant).child(code_view(
                    "{\n  \"amount\": 4200,\n  \"currency\": \"usd\"\n}",
                    Lang::Json,
                    t,
                )),
            )
    }

    fn render_environment(&self, t: &Theme) -> impl IntoElement {
        let env = &self.mock.environments[self.env];
        let rows: Vec<Kv> = env
            .vars
            .iter()
            .map(|(k, v, _)| Kv {
                key: k,
                value: v,
                enabled: true,
            })
            .collect();
        self.page(t, env.name, "Environment variables")
            .child(kv_table(t, &rows, true))
    }

    fn render_history_entry(&self, t: &Theme, tab: &Tab) -> impl IntoElement {
        let idx: usize = tab.key.trim_start_matches("history/").parse().unwrap_or(0);
        let h = &self.mock.history[idx.min(self.mock.history.len() - 1)];
        let r = self.mock.request("payments/create").cloned();
        self.page(t, h.name, "History entry · read-only")
            .child(
                row()
                    .gap_3()
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_color(t.method_color(h.method))
                            .child(h.method),
                    )
                    .child(
                        div()
                            .text_color(t.status_color(h.status))
                            .child(h.status.to_string()),
                    )
                    .child(format!("{} ms · {}", h.ms, h.time)),
            )
            .children(r.map(|r| code_view(r.response.body, Lang::Json, t)))
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
        let body: AnyElement = match self.bottom_tab {
            BottomTab::Console => div()
                .flex()
                .flex_col()
                .p_2()
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .text_color(t.text_muted)
                .children(
                    self.mock.requests[2]
                        .response
                        .console
                        .iter()
                        .map(|l| div().child(*l)),
                )
                .into_any_element(),
            BottomTab::Problems => div()
                .flex()
                .flex_col()
                .children(self.mock.problems.iter().map(|p| {
                    row()
                        .h(px(26.))
                        .px_3()
                        .gap_3()
                        .text_size(px(12.))
                        .child(icon("triangle-alert", 12., t.modified))
                        .child(p.message)
                        .child(
                            div()
                                .text_color(t.text_placeholder)
                                .child(format!("{}:{}", p.file, p.line)),
                        )
                        .child(
                            div()
                                .font_family(BUFFER_FONT)
                                .text_color(t.text_placeholder)
                                .child(p.code),
                        )
                }))
                .into_any_element(),
        };
        div()
            .h(px(200.))
            .flex_none()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(t.border)
            .bg(t.surface)
            .child(
                row()
                    .h(px(28.))
                    .px_2()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .children(tabs.map(|(b, label)| {
                        let on = self.bottom_tab == b;
                        div()
                            .id(label)
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(if on { t.text } else { t.text_muted })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.bottom_tab = b;
                                cx.notify();
                            }))
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
