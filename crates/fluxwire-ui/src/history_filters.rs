//! History tab header, toolbar (search, chips, time range) and stats strip (SPEC §13.1).

use fluxwire_store::{ExportFormat, Scope, Stats};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, deferred, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    history::{CLASS_CHIPS, METHOD_CHIPS, short_method},
    history_query::TimeRange,
    theme::Theme,
    ui::{icon, row},
    workspace::WorkspaceView,
};

/// "98%" style share.
pub fn percent(r: f64) -> String {
    format!("{:.0}%", r * 100.0)
}

pub fn ms_label(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

/// The text of the stats strip, or `None` for an empty set.
pub fn stats_text(s: &Stats) -> Option<String> {
    (s.count > 0).then(|| {
        format!(
            "{} entr{} · {} success · p50 {} · p95 {}",
            s.count,
            if s.count == 1 { "y" } else { "ies" },
            percent(s.success_rate),
            ms_label(s.p50_ms),
            ms_label(s.p95_ms)
        )
    })
}

impl WorkspaceView {
    pub(crate) fn chip(
        &self,
        t: &Theme,
        id: String,
        label: &str,
        on: bool,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(gpui::ElementId::Name(id.into()))
            .px_1p5()
            .h(px(20.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .cursor_pointer()
            .text_size(px(11.))
            .font_family(BUFFER_FONT)
            .when(on, |d| d.bg(t.element_selected).text_color(t.text))
            .when(!on, |d| {
                let h = t.element_hover;
                d.text_color(t.text_muted).hover(move |s| s.bg(h))
            })
            .child(label.to_string())
    }

    pub(crate) fn render_history_header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let menu_item = |id: &'static str, label: &'static str| {
            let hover = t.element_hover;
            row()
                .id(id)
                .h(px(26.))
                .px_2()
                .cursor_pointer()
                .text_size(px(12.))
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        let dropdown = |id: &'static str| {
            div()
                .id(id)
                .absolute()
                .top(px(26.))
                .right_0()
                .w(px(190.))
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(t.border)
                .bg(t.elevated)
                .shadow_lg()
                .occlude()
        };
        let head_btn = |id: &'static str, label: &'static str, on: bool| {
            let hover = t.element_hover;
            row()
                .id(id)
                .h(px(24.))
                .px_2()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .when(on, |d| d.bg(t.element_selected))
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        let n = self.history.stats.count;
        row()
            .flex_none()
            .h(px(40.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("History"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child(format!("{n} result{}", if n == 1 { "" } else { "s" })),
            )
            .child(div().flex_1())
            .child(
                row()
                    .id("hist-refresh")
                    .size(px(24.))
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.history_reload(cx)))
                    .child(icon("refresh-cw", 14., t.text_muted)),
            )
            .child(
                div()
                    .relative()
                    .child(
                        head_btn("hist-export", "Export ▾", self.history.export_menu).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.history.export_menu = !this.history.export_menu;
                                this.history.clear_menu = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .when(self.history.export_menu, |d| {
                        d.child(
                            deferred(
                                dropdown("hist-export-menu")
                                    .child(menu_item("hist-export-json", "JSON").on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.history_export(ExportFormat::Json, cx)
                                        }),
                                    ))
                                    .child(menu_item("hist-export-csv", "CSV").on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.history_export(ExportFormat::Csv, cx)
                                        }),
                                    )),
                            )
                            .with_priority(1),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .child(
                        head_btn("hist-clear", "Clear ▾", self.history.clear_menu).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.history.clear_menu = !this.history.clear_menu;
                                this.history.export_menu = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .when(self.history.clear_menu, |d| {
                        d.child(
                            deferred(
                                dropdown("hist-clear-menu")
                                    .child(
                                        menu_item("hist-clear-req", "Selected request").on_click(
                                            cx.listener(|this, _, w, cx| {
                                                this.history.clear_menu = false;
                                                if let Some((c, p, name)) =
                                                    this.history_selected_request()
                                                {
                                                    this.confirm_clear_history(
                                                        Scope::Request {
                                                            collection: c,
                                                            path: p,
                                                        },
                                                        format!("“{name}”"),
                                                        w,
                                                        cx,
                                                    );
                                                } else {
                                                    this.notice =
                                                        Some("Select an entry first.".into());
                                                    cx.notify();
                                                }
                                            }),
                                        ),
                                    )
                                    .child(
                                        menu_item("hist-clear-coll", "Selected collection")
                                            .on_click(cx.listener(|this, _, w, cx| {
                                                this.history.clear_menu = false;
                                                if let Some((c, _, _)) =
                                                    this.history_selected_request()
                                                {
                                                    this.confirm_clear_history(
                                                        Scope::Collection(c.clone()),
                                                        format!("the collection “{c}”"),
                                                        w,
                                                        cx,
                                                    );
                                                } else {
                                                    this.notice =
                                                        Some("Select an entry first.".into());
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(menu_item("hist-clear-all", "All history").on_click(
                                        cx.listener(|this, _, w, cx| {
                                            this.history.clear_menu = false;
                                            this.confirm_clear_history(
                                                Scope::All,
                                                "this workspace".into(),
                                                w,
                                                cx,
                                            );
                                        }),
                                    )),
                            )
                            .with_priority(1),
                        )
                    }),
            )
            .into_any_element()
    }

    /// `(collection, request path, name)` of the selected entry.
    pub(crate) fn history_selected_request(&self) -> Option<(String, String, String)> {
        let id = self.history.selected?;
        let e = self.history.entries.iter().find(|e| e.id == id)?;
        Some((
            e.collection.clone(),
            e.request_path.clone(),
            e.request_name.clone(),
        ))
    }

    pub(crate) fn render_history_toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let q = &self.history.query;
        let search = row()
            .id("history-search")
            .w(px(240.))
            .flex_none()
            .h(px(24.))
            .px_2()
            .rounded(px(4.))
            .border_1()
            .border_color(t.border)
            .focus({
                let a = t.accent;
                move |s| s.border_color(a)
            })
            .font_family(BUFFER_FONT)
            .text_size(px(12.))
            .overflow_hidden()
            .children(self.history.input.clone());
        let methods = row().gap_1().children(METHOD_CHIPS.iter().map(|m| {
            let m = *m;
            let on = q.method.as_deref() == Some(m);
            self.chip(t, format!("hm-{m}"), short_method(m), on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let cur = this.history.query.method.take();
                    this.history.query.method = (cur.as_deref() != Some(m)).then(|| m.to_string());
                    this.history.query.nav = None;
                    this.history_reload(cx);
                }))
        }));
        let classes = row().gap_1().children(CLASS_CHIPS.iter().map(|(c, label)| {
            let c = *c;
            let on = q.class == Some(c);
            self.chip(t, format!("hc-{c}"), label, on)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let q = &mut this.history.query;
                    q.class = (q.class != Some(c)).then_some(c);
                    q.nav = None;
                    this.history_reload(cx);
                }))
        }));
        let ranges = row().gap_1().children(TimeRange::ALL.iter().map(|r| {
            let r = *r;
            self.chip(
                t,
                format!("hr-{}", r.label()),
                r.label(),
                q.range == Some(r),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let q = &mut this.history.query;
                q.range = (q.range != Some(r)).then_some(r);
                q.nav = None;
                this.history_reload(cx);
            }))
        }));
        let toggles = row()
            .gap_1()
            .child(
                self.chip(t, "hf-unsaved".into(), "unsaved", q.unsaved)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let q = &mut this.history.query;
                        q.unsaved = !q.unsaved;
                        q.nav = None;
                        this.history_reload(cx);
                    })),
            )
            .child(
                self.chip(t, "hf-pinned".into(), "pinned", q.pinned)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let q = &mut this.history.query;
                        q.pinned = !q.pinned;
                        q.nav = None;
                        this.history_reload(cx);
                    })),
            );
        let active = q.chips();
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(t.border_variant)
            .child(
                row()
                    .gap_3()
                    .flex_wrap()
                    .child(search)
                    .child(methods)
                    .child(classes)
                    .child(ranges)
                    .child(toggles),
            )
            .when(!active.is_empty(), |d| {
                d.child(
                    row()
                        .gap_1()
                        .flex_wrap()
                        .children(active.into_iter().enumerate().map(|(i, (kind, label))| {
                            self.chip(t, format!("hx-{i}"), &format!("{label} ×"), true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.history.query.remove(kind);
                                    this.history_reload(cx);
                                }))
                        }))
                        .child(
                            self.chip(t, "hx-clear".into(), "Clear filters", false)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.history_clear_filters(cx)),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_history_stats(&self, t: &Theme) -> Option<AnyElement> {
        let text = stats_text(&self.history.stats)?;
        Some(
            row()
                .flex_none()
                .h(px(24.))
                .px_3()
                .border_b_1()
                .border_color(t.border_variant)
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child(text)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_strip_text() {
        assert_eq!(stats_text(&Stats::default()), None);
        let s = Stats {
            count: 10,
            success_rate: 0.8,
            p50_ms: 50.0,
            p95_ms: 1500.0,
        };
        assert_eq!(
            stats_text(&s).unwrap(),
            "10 entries · 80% success · p50 50 ms · p95 1.50 s"
        );
        let one = Stats { count: 1, ..s };
        assert!(stats_text(&one).unwrap().starts_with("1 entry"));
    }
}
