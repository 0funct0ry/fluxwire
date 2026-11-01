//! The left-dock History panel (SPEC §13.1): navigation only. It lists the timeline, the
//! built-in saved views and the requests that have history, with counts; every item opens the
//! History tab with the matching filter applied.

use fluxwire_store::Scope;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, StatefulInteractiveElement, Styled, Window, anchored as popover, deferred, div,
    prelude::*, px,
};

use crate::{
    dialogs::{Confirm, ConfirmAction, Dialog},
    history_query::{NavItem, SavedView, Timeline},
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
};

/// What a Requests-tree context menu acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavTarget {
    Request {
        collection: String,
        path: String,
        name: String,
    },
    Collection(String),
}

impl NavTarget {
    pub fn item(&self) -> NavItem {
        match self {
            NavTarget::Request {
                collection, path, ..
            } => NavItem::Request {
                collection: collection.clone(),
                path: path.clone(),
            },
            NavTarget::Collection(c) => NavItem::Collection(c.clone()),
        }
    }

    pub fn scope(&self) -> Scope {
        match self {
            NavTarget::Request {
                collection, path, ..
            } => Scope::Request {
                collection: collection.clone(),
                path: path.clone(),
            },
            NavTarget::Collection(c) => Scope::Collection(c.clone()),
        }
    }

    fn label(&self) -> String {
        match self {
            NavTarget::Request { name, .. } => format!("“{name}”"),
            NavTarget::Collection(c) => format!("the collection “{c}”"),
        }
    }
}

impl WorkspaceView {
    /// Apply a navigator item to the History tab and show the tab.
    pub fn history_show(&mut self, item: NavItem, cx: &mut Context<Self>) {
        self.history.query.apply_nav(item);
        self.history.nav_menu = None;
        self.history.selected = None;
        self.history.ticked.clear();
        if let Some(i) = &self.history.input {
            i.update(cx, |i, cx| i.set_text("", cx));
        }
        self.open_history_tab(cx);
        self.history_reload(cx);
    }

    /// Ask before clearing a scope; the second button also removes pinned entries.
    pub fn confirm_clear_history(
        &mut self,
        scope: Scope,
        what: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pinned = self.history.counts.pinned;
        let mut buttons = vec![(
            "Clear".to_string(),
            ConfirmAction::ClearHistory {
                scope: scope.clone(),
                include_pinned: false,
            },
        )];
        if pinned > 0 || matches!(scope, Scope::All) {
            buttons.push((
                "Clear including pinned".to_string(),
                ConfirmAction::ClearHistory {
                    scope,
                    include_pinned: true,
                },
            ));
        }
        buttons.push(("Cancel".to_string(), ConfirmAction::Cancel));
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: format!("Clear history for {what}?"),
            lines: vec![
                "Pinned entries are kept unless you choose “Clear including pinned”.".into(),
            ],
            buttons,
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    pub(crate) fn render_history_nav(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.history.ws_key.is_none() {
            return div()
                .p_3()
                .text_size(px(12.))
                .text_color(t.text_placeholder)
                .child("Open a workspace to see its history.")
                .into_any_element();
        }
        let c = self.history.counts.clone();
        let active = self.history.query.nav.clone();
        let item = |id: String,
                    label: String,
                    count: i64,
                    target: NavItem,
                    depth: usize,
                    menu: Option<NavTarget>,
                    cx: &mut Context<Self>| {
            let on = active.as_ref() == Some(&target);
            let t2 = target.clone();
            self.tree_row(t, gpui::ElementId::Name(id.into()), depth, on)
                .gap_2()
                .on_click(cx.listener(move |this, _, _, cx| this.history_show(t2.clone(), cx)))
                .when_some(menu, |d, m| {
                    d.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            this.history.nav_menu = Some((m.clone(), ev.position));
                            cx.notify();
                        }),
                    )
                })
                .child(div().flex_1().truncate().child(label))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(count.to_string()),
                )
                .into_any_element()
        };
        let heading = |s: &str| {
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(s.to_string())
                .into_any_element()
        };
        let total = c.today + c.yesterday + c.this_week + c.older;
        let mut out: Vec<AnyElement> = vec![item(
            "hn-all".into(),
            "All history".into(),
            total,
            NavItem::All,
            0,
            None,
            cx,
        )];
        out.push(heading("Timeline"));
        for tl in Timeline::ALL {
            let n = match tl {
                Timeline::Today => c.today,
                Timeline::Yesterday => c.yesterday,
                Timeline::Week => c.this_week,
                Timeline::Older => c.older,
            };
            out.push(item(
                format!("hn-tl-{}", tl.label()),
                tl.label().into(),
                n,
                NavItem::Timeline(tl),
                0,
                None,
                cx,
            ));
        }
        out.push(heading("Saved views"));
        for v in SavedView::ALL {
            let n = match v {
                SavedView::Failures => c.failures,
                SavedView::Slow => c.slow,
                SavedView::Unsaved => c.unsaved,
                SavedView::Pinned => c.pinned,
                SavedView::Runs => c.runs,
            };
            out.push(item(
                format!("hn-v-{}", v.label()),
                v.label().into(),
                n,
                NavItem::View(v),
                0,
                None,
                cx,
            ));
        }
        out.push(heading("Requests"));
        if c.requests.is_empty() {
            out.push(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("No history yet. Send a request.")
                    .into_any_element(),
            );
        }
        let mut last = String::new();
        for (i, (coll, path, name, n)) in c.requests.iter().enumerate() {
            if *coll != last {
                last = coll.clone();
                let total: i64 = c
                    .requests
                    .iter()
                    .filter(|r| r.0 == *coll)
                    .map(|r| r.3)
                    .sum();
                out.push(item(
                    format!("hn-c-{coll}"),
                    if coll.is_empty() {
                        "(no collection)".into()
                    } else {
                        coll.clone()
                    },
                    total,
                    NavItem::Collection(coll.clone()),
                    0,
                    Some(NavTarget::Collection(coll.clone())),
                    cx,
                ));
            }
            let target = NavTarget::Request {
                collection: coll.clone(),
                path: path.clone(),
                name: name.clone(),
            };
            out.push(item(
                format!("hn-r-{i}"),
                name.clone(),
                *n,
                target.item(),
                1,
                Some(target),
                cx,
            ));
        }
        div()
            .flex()
            .flex_col()
            .pb_2()
            .children(out)
            .into_any_element()
    }

    /// Context menu of a Requests-tree item.
    pub(crate) fn render_history_nav_menu(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (target, pos): (NavTarget, Point<Pixels>) = self.history.nav_menu.clone()?;
        let what = target.label();
        let mut items: Vec<(&'static str, u8)> = vec![("Show history", 0)];
        items.push(match target {
            NavTarget::Request { .. } => ("Clear history for this request", 1),
            NavTarget::Collection(_) => ("Clear history for this collection", 1),
        });
        let t = t.clone();
        Some(
            deferred(
                popover().position(pos).child(
                    div()
                        .id("hist-nav-menu")
                        .w(px(250.))
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
                            this.history.nav_menu = None;
                            cx.notify();
                        }))
                        .children(items.into_iter().enumerate().map(|(i, (label, kind))| {
                            let hover = t.element_hover;
                            let target = target.clone();
                            let what = what.clone();
                            row()
                                .id(("hist-nav-menu-item", i))
                                .h(px(26.))
                                .mx_1()
                                .px_2()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(t.text)
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.history.nav_menu = None;
                                    match kind {
                                        0 => this.history_show(target.item(), cx),
                                        _ => this.confirm_clear_history(
                                            target.scope(),
                                            what.clone(),
                                            w,
                                            cx,
                                        ),
                                    }
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
