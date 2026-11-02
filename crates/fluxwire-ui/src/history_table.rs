//! The History tab's virtualized table (SPEC §13.1): resizable, sortable columns, day headers,
//! folder-run group rows, tick boxes, pin and note glyphs, and the row context menu.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, anchored as popover, deferred, div, prelude::*, px,
    uniform_list,
};

use crate::{
    assets::BUFFER_FONT,
    history::{HistResize, now_ms, short_method, time_label},
    history_query::{COLUMNS, HistRow},
    theme::Theme,
    ui::{icon, row},
    workspace::WorkspaceView,
};

pub const ROW_H: f32 = 26.0;
const TICK_W: f32 = 26.0;

/// "just now", "5 m ago", "3 h ago", then the clock time.
pub fn relative_time(ts_ms: i64, now: i64) -> String {
    let s = (now - ts_ms).max(0) / 1000;
    match s {
        0..=4 => "just now".into(),
        5..=59 => format!("{s} s ago"),
        60..=3599 => format!("{} m ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => time_label(ts_ms),
    }
}

impl WorkspaceView {
    pub(crate) fn render_history_table(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let sort = self.history.query.sort;
        let header = row()
            .flex_none()
            .h(px(ROW_H))
            .pl(px(TICK_W))
            .border_b_1()
            .border_color(t.border_variant)
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(t.text_muted)
            .children(COLUMNS.iter().enumerate().map(|(i, (name, col))| {
                let col = *col;
                let w = self.history.widths[i];
                let arrow = if sort.col == col {
                    if sort.asc { " ↑" } else { " ↓" }
                } else {
                    ""
                };
                let hover = t.element_hover;
                row()
                    .relative()
                    .w(px(w))
                    .h_full()
                    .flex_none()
                    .child(
                        div()
                            .id(("hist-col", i))
                            .flex_1()
                            .h_full()
                            .px_2()
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.history_set_sort(col, cx)),
                            )
                            .child(format!("{name}{arrow}")),
                    )
                    .child(
                        div()
                            .id(("hist-col-edge", i))
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .w(px(5.))
                            .cursor_col_resize()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                                    this.history.resizing = Some(HistResize::Col(
                                        i,
                                        f32::from(ev.position.x),
                                        this.history.widths[i],
                                    ));
                                    cx.notify();
                                }),
                            ),
                    )
            }));
        let body: AnyElement = if self.history.rows.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(t.text_placeholder)
                .child("No entries match these filters")
                .child(
                    self.chip(t, "hist-empty-clear".into(), "Clear filters", true)
                        .on_click(cx.listener(|this, _, _, cx| this.history_clear_filters(cx))),
                )
                .into_any_element()
        } else {
            uniform_list(
                "history-table",
                self.history.rows.len(),
                cx.processor(
                    move |this: &mut WorkspaceView, range: std::ops::Range<usize>, _w, cx| {
                        let t = crate::theme::theme(cx).clone();
                        let rows = this.history.rows.clone();
                        let now = now_ms();
                        range
                            .filter_map(|i| rows.get(i).cloned().map(|r| (i, r)))
                            .map(|(i, r)| this.history_table_row(&t, i, r, now, cx))
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(self.history.scroll.clone())
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        div()
            .id("history-table-wrap")
            .key_context("HistoryTab")
            .track_focus(&self.history_focus)
            .on_key_down(cx.listener(|this, ev, w, cx| this.on_history_key(ev, w, cx)))
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn history_table_row(
        &self,
        t: &Theme,
        i: usize,
        r: HistRow,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match r {
            HistRow::Day(d) => div()
                .h(px(ROW_H))
                .px_3()
                .flex()
                .items_end()
                .pb_1()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_placeholder)
                .bg(t.surface)
                .child(d)
                .into_any_element(),
            HistRow::More => div()
                .id("hist-more")
                .h(px(ROW_H))
                .px_3()
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(t.accent)
                .child("Load more…")
                .on_click(cx.listener(|this, _, _, cx| this.history_more(cx)))
                .into_any_element(),
            HistRow::Run {
                id,
                count,
                failed,
                expanded,
                first,
            } => {
                let ts = self.history.entries.get(first).map_or(0, |e| e.ts_ms);
                let short = id.rsplit('-').next().unwrap_or(&id).to_string();
                let rid = id.clone();
                row()
                    .id(("hist-run", i))
                    .h(px(ROW_H))
                    .px_3()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.history_toggle_run(&rid, cx)))
                    .child(icon(
                        if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        12.,
                        t.text_muted,
                    ))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("Folder run {short}")),
                    )
                    .child(div().text_color(t.text_muted).child(format!(
                        "{count} sends · {} passed · {failed} failed",
                        count - failed
                    )))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(t.text_placeholder)
                            .child(relative_time(ts, now)),
                    )
                    .into_any_element()
            }
            HistRow::Entry(ix) => {
                let Some(e) = self.history.entries.get(ix).cloned() else {
                    return div().into_any_element();
                };
                let id = e.id;
                let on = self.history.selected == Some(id);
                let ticked = self.history.ticked.contains(&id);
                let w = &self.history.widths;
                let cell = |n: usize| div().w(px(w[n])).flex_none().px_2().truncate();
                let mono = |d: gpui::Div| d.font_family(BUFFER_FONT).text_size(px(11.));
                let nested = e.run_id.is_some();
                row()
                    .id(("hist-row", i))
                    .h(px(ROW_H))
                    .pl(px(if nested { 14.0 } else { 0.0 }))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(on, |d| d.bg(t.element_selected))
                    .when(!on, |d| {
                        let h = t.element_hover;
                        d.hover(move |s| s.bg(h))
                    })
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                        window.focus(&this.history_focus);
                        this.history_select(id, cx);
                        if ev.click_count() >= 2 {
                            this.open_history_entry(id, true, cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            this.history_select(id, cx);
                            this.history.menu = Some((id, ev.position));
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .id(("hist-tick", i))
                            .w(px(TICK_W))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(12.))
                                    .rounded(px(2.))
                                    .border_1()
                                    .border_color(t.border)
                                    .when(ticked, |d| d.bg(t.accent).border_color(t.accent)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.history_toggle_tick(id, cx);
                            })),
                    )
                    .child(
                        cell(0)
                            .text_color(t.text_muted)
                            .child(relative_time(e.ts_ms, now)),
                    )
                    .child(
                        mono(cell(1))
                            .text_color(t.method_color(&e.method))
                            .child(short_method(&e.method).to_string()),
                    )
                    .child(
                        cell(2).child(
                            row()
                                .gap_1()
                                .child(div().truncate().child(e.request_name.clone()))
                                .when(is_agent(&e), |d| {
                                    d.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .text_xs()
                                            .text_color(t.accent)
                                            .child(AGENT_TAG),
                                    )
                                })
                                .when(e.pinned, |d| d.child(icon("pin", 11., t.accent)))
                                .when(e.note.is_some(), |d| {
                                    d.child(icon("pencil", 11., t.text_muted))
                                }),
                        ),
                    )
                    .child(mono(cell(3)).text_color(t.text_muted).child(e.url.clone()))
                    .child(
                        mono(cell(4))
                            .text_color(match e.status {
                                Some(s) => t.status_color(s as u16),
                                None => t.deleted,
                            })
                            .child(e.status.map_or("err".to_string(), |s| s.to_string())),
                    )
                    .child(
                        mono(cell(5))
                            .text_color(t.text_muted)
                            .child(format!("{:.0} ms", e.total_ms)),
                    )
                    .child(
                        mono(cell(6))
                            .text_color(t.text_muted)
                            .child(e.env.clone().unwrap_or_default()),
                    )
                    .into_any_element()
            }
        }
    }

    /// Context menu of a table row.
    pub(crate) fn render_history_menu(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (id, pos) = self.history.menu?;
        let e = self.history.entries.iter().find(|e| e.id == id)?.clone();
        type Act = fn(&mut WorkspaceView, i64, &mut gpui::Window, &mut Context<WorkspaceView>);
        let mut items: Vec<(String, Act)> = vec![
            ("Open in tab".into(), |w, id, _, cx| {
                w.open_history_entry(id, true, cx)
            }),
            ("Restore into request".into(), |w, id, win, cx| {
                if let Some(e) = w.history.entries.iter().find(|e| e.id == id) {
                    let key = e.request_path.clone();
                    w.history_restore(id, key, win, cx);
                }
            }),
            ("Copy as cURL".into(), |w, id, _, cx| {
                w.history_copy_curl(id, cx)
            }),
            ("Go to request".into(), |w, id, win, cx| {
                if let Some(e) = w.history.entries.iter().find(|e| e.id == id) {
                    let key = e.request_path.clone();
                    w.open_request(&key, true, win, cx);
                }
            }),
            (
                if e.pinned { "Unpin" } else { "Pin" }.into(),
                |w, id, _, cx| {
                    let pinned = w
                        .history
                        .entries
                        .iter()
                        .find(|e| e.id == id)
                        .is_some_and(|e| e.pinned);
                    w.history_set_pinned(vec![id], !pinned, cx);
                },
            ),
            ("Edit note".into(), |w, _, win, cx| {
                let i = w.history_note_input(cx);
                i.update(cx, |i, _| i.focus(win));
            }),
            ("Compare to previous".into(), |w, id, _, cx| {
                w.history_compare_previous(id, cx)
            }),
            ("Mark for diff".into(), |w, id, _, cx| {
                w.history_toggle_tick(id, cx);
                if w.history.ticked.len() == 2 {
                    w.history_diff_ticked(cx);
                }
            }),
        ];
        if e.env.is_some() {
            items.push(("Only this environment".into(), |w, id, _, cx| {
                let env = w
                    .history
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .and_then(|e| e.env.clone());
                w.history.query.env = env;
                w.history.query.nav = None;
                w.history_reload(cx);
            }));
        }
        items.push(("Only this protocol".into(), |w, id, _, cx| {
            let p = w
                .history
                .entries
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.protocol.clone());
            w.history.query.protocol = p;
            w.history.query.nav = None;
            w.history_reload(cx);
        }));
        items.push(("Only this collection".into(), |w, id, _, cx| {
            let c = w
                .history
                .entries
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.collection.clone());
            w.history.query.collection = c;
            w.history.query.nav = None;
            w.history_reload(cx);
        }));
        items.push(("Delete".into(), |w, _, win, cx| {
            w.history_confirm_delete(win, cx)
        }));
        let t = t.clone();
        Some(
            deferred(
                popover().position(pos).child(
                    div()
                        .id("hist-menu")
                        .w(px(210.))
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
                            this.history.menu = None;
                            cx.notify();
                        }))
                        .children(items.into_iter().enumerate().map(|(i, (label, act))| {
                            let hover = t.element_hover;
                            row()
                                .id(("hist-menu-item", i))
                                .h(px(26.))
                                .mx_1()
                                .px_2()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.history.menu = None;
                                    act(this, id, w, cx);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_buckets() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now - 1000, now), "just now");
        assert_eq!(relative_time(now - 30_000, now), "30 s ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5 m ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 h ago");
        assert_eq!(relative_time(now + 5000, now), "just now");
    }
}

/// Tag shown on rows written through MCP (ADR-009).
pub(crate) const AGENT_TAG: &str = "agent";

pub(crate) fn is_agent(e: &fluxwire_store::Summary) -> bool {
    e.origin.as_deref() == Some(AGENT_TAG)
}

#[cfg(test)]
mod agent_tag_tests {
    use super::*;

    #[test]
    fn only_agent_origin_is_tagged() {
        let mut e = fluxwire_store::Summary {
            id: 1,
            ts_ms: 0,
            collection: String::new(),
            request_path: String::new(),
            request_name: String::new(),
            protocol: "http".into(),
            method: "GET".into(),
            url: String::new(),
            status: None,
            total_ms: 0.0,
            env: None,
            dirty: false,
            summary: String::new(),
            run_id: None,
            pinned: false,
            note: None,
            origin: None,
        };
        assert!(!is_agent(&e));
        e.origin = Some("agent".into());
        assert!(is_agent(&e));
    }
}
