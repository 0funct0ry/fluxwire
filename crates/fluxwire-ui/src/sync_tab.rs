//! The Sync tab, dock panel and status item (SPEC §15.3). The model is `sync_state.rs`; this
//! file draws it and wires Pull, Push, Refresh, include/exclude and the conflict resolver.

use fluxwire_openapi::link::Resolution;
use fluxwire_openapi::push::Deletion;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px,
};

use crate::{
    assets::BUFFER_FONT,
    dev_id::{DevClick, Role},
    dialogs::button,
    sync_state::{change_key, describe},
    tabs::TabKind,
    theme::Theme,
    ui::{icon, row},
    workspace::WorkspaceView,
};

pub const TAB_KEY: &str = "sync";

fn short(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::Null => "(none)".to_string(),
        serde_json::Value::String(s) => s.clone(),
        o => o.to_string(),
    };
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

impl WorkspaceView {
    /// Reload every Spec link of the open Workspace (on open, on file changes, on Refresh).
    pub fn refresh_sync(&mut self) {
        let cols: Vec<_> = self
            .scope
            .workspace
            .as_ref()
            .map(|w| {
                w.collections
                    .iter()
                    .filter(|c| !c.collection.config.openapi.is_empty())
                    .map(|c| (c.root.clone(), c.collection.config.openapi.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if cols.is_empty() && !self.sync.has_links() {
            return;
        }
        self.sync.refresh(&cols);
    }

    pub fn open_sync_tab(&mut self, cx: &mut Context<Self>) {
        self.refresh_sync();
        self.open_tab(TAB_KEY, "Sync", TabKind::Sync, true);
        cx.notify();
    }

    pub fn sync_pull(&mut self, link: usize, cx: &mut Context<Self>) {
        let r = self.sync.pull(link);
        self.finish_sync(r, cx);
    }

    pub fn sync_push(&mut self, link: usize, cx: &mut Context<Self>) {
        let r = self.sync.push(link);
        self.finish_sync(r, cx);
    }

    fn finish_sync(&mut self, r: Result<String, String>, cx: &mut Context<Self>) {
        self.sync.notice = Some(match r {
            Ok(s) => s,
            Err(e) => e,
        });
        self.refresh_sync();
        cx.notify();
    }

    /// Ask before unlinking link `i`.
    pub fn ask_remove_spec_link(
        &mut self,
        i: usize,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self.sync.rows.get(i) else {
            return;
        };
        let action = crate::dialogs::ConfirmAction::RemoveSpecLink {
            root: r.collection_root.clone(),
            name: r.link.name.clone(),
        };
        self.dialog = Some(crate::dialogs::Dialog::Confirm(crate::dialogs::Confirm {
            title: format!("Remove the Spec link “{}”?", r.link.name),
            lines: vec![
                "Syncing stops. The spec file and your Requests are kept.".into(),
                "The base snapshot in .fluxwire/sync is moved to the trash.".into(),
            ],
            buttons: vec![
                ("Remove link".into(), action),
                ("Cancel".into(), crate::dialogs::ConfirmAction::Cancel),
            ],
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    pub fn remove_spec_link(&mut self, root: &std::path::Path, name: &str, cx: &mut Context<Self>) {
        let snapshot = root
            .join(".fluxwire/sync")
            .join(format!("{name}.base.json"));
        self.sync.notice = Some(match fluxwire_format::remove_link(root, name) {
            Ok(true) => {
                if snapshot.exists() {
                    let _ = fluxwire_format::delete(&snapshot);
                }
                format!("Removed the Spec link “{name}”")
            }
            Ok(false) => format!("No Spec link “{name}” in fluxwire.toml"),
            Err(e) => format!("Could not update fluxwire.toml: {e}"),
        });
        self.on_fs_changes(&[root.join("fluxwire.toml")], cx);
    }

    pub(crate) fn render_sync_tab(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut page = self.page(t, "Sync", "OpenAPI Spec links of this Workspace");
        if !self.sync.has_links() {
            return page.child(div().text_color(t.text_muted).child(
                "No Spec link yet. Import an OpenAPI document with “Create a Spec link” to start syncing.",
            ));
        }
        if let Some(n) = &self.sync.notice {
            page = page.child(div().text_color(t.text_muted).child(n.clone()));
        }
        for (i, r) in self.sync.rows.iter().enumerate() {
            let open = self.sync.open_conflicts(i);
            let s = r.status;
            page = page.child(
                row()
                    .gap_3()
                    .flex_wrap()
                    .items_center()
                    .child(icon("refresh-cw", 14., t.text_muted))
                    .child(div().font_family(BUFFER_FONT).child(r.link.source.clone()))
                    .child(div().text_color(t.text_muted).child(format!(
                        "{} incoming, {} outgoing, {} conflicts",
                        s.incoming, s.outgoing, s.conflicts
                    )))
                    .child(div().flex_1())
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-refresh-{i}")),
                            "Refresh".into(),
                            false,
                            true,
                        )
                        .dev_click(
                            "sync.refresh",
                            Role::Button,
                            "Refresh",
                            true,
                            cx.listener(|this, _, _, cx| {
                                this.refresh_sync();
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-pull-{i}")),
                            "Pull".into(),
                            true,
                            open == 0 && s.incoming + s.conflicts > 0,
                        )
                        .dev_click(
                            "sync.pull",
                            Role::Button,
                            "Pull",
                            open == 0,
                            cx.listener(move |this, _, _, cx| this.sync_pull(i, cx)),
                        ),
                    )
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-push-{i}")),
                            "Push".into(),
                            false,
                            open == 0 && s.outgoing + s.conflicts > 0,
                        )
                        .dev_click(
                            "sync.push",
                            Role::Button,
                            "Push",
                            open == 0,
                            cx.listener(move |this, _, _, cx| this.sync_push(i, cx)),
                        ),
                    )
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-unlink-{i}")),
                            "Remove link".into(),
                            false,
                            true,
                        )
                        .dev_click(
                            "sync.unlink",
                            Role::Button,
                            "Remove link",
                            true,
                            cx.listener(move |this, _, window, cx| {
                                this.ask_remove_spec_link(i, window, cx)
                            }),
                        ),
                    ),
            );
            if let Some(e) = &r.error {
                page = page.child(div().text_color(t.deleted).child(e.clone()));
                continue;
            }
            for (ci, c) in r.changes.iter().enumerate() {
                let (arrow, op, what) = describe(c);
                let excluded = self.sync.is_excluded(i, c);
                let key = change_key(c);
                page = page.child(
                    row()
                        .id(SharedString::from(format!("sync-change-{i}-{ci}")))
                        .h(px(28.))
                        .gap_3()
                        .items_center()
                        .border_b_1()
                        .border_color(t.border_variant)
                        .cursor_pointer()
                        .child(icon(
                            if excluded { "square" } else { "check" },
                            14.,
                            t.text_muted,
                        ))
                        .child(
                            div()
                                .text_color(if arrow == "↓" { t.accent } else { t.modified })
                                .child(arrow),
                        )
                        .child(
                            div()
                                .w(px(220.))
                                .truncate()
                                .font_family(BUFFER_FONT)
                                .text_size(px(12.))
                                .child(op),
                        )
                        .child(div().text_color(t.text_muted).child(what))
                        .dev_click(
                            "sync.change",
                            Role::Button,
                            "Include change",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                this.sync.toggle(i, &key);
                                cx.notify();
                            }),
                        ),
                );
            }
            for (ki, c) in r.conflicts.iter().enumerate() {
                page = page.child(self.render_conflict(t, i, ki, c, cx));
            }
            if !r.manual.is_empty() {
                page = page.child(
                    div()
                        .pt_2()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Needs manual edit"),
                );
                for m in &r.manual {
                    let loc = m
                        .pointer
                        .clone()
                        .map(|p| format!(" ({}{})", r.link.source, p))
                        .unwrap_or_default();
                    page = page.child(
                        div()
                            .text_color(t.text_muted)
                            .child(format!("{} · {} — {}{loc}", m.operation, m.field, m.reason)),
                    );
                }
            }
            if r.changes.iter().any(|c| {
                matches!(c.kind, fluxwire_openapi::ChangeKind::Delete)
                    && c.direction == fluxwire_openapi::Direction::Outgoing
            }) {
                let dep = self.sync.deletion() == Deletion::Deprecate;
                page = page.child(
                    row()
                        .gap_2()
                        .items_center()
                        .child("When pushing, deleted operations are:")
                        .child(
                            button(t, "sync-del-dep", "Marked deprecated".into(), dep, true)
                                .dev_click(
                                    "sync.deprecate",
                                    Role::Button,
                                    "Deprecate",
                                    true,
                                    cx.listener(|this, _, _, cx| {
                                        this.sync.deletion = Some(Deletion::Deprecate);
                                        cx.notify();
                                    }),
                                ),
                        )
                        .child(
                            button(t, "sync-del-rm", "Removed from the spec".into(), !dep, true)
                                .dev_click(
                                    "sync.remove",
                                    Role::Button,
                                    "Remove",
                                    true,
                                    cx.listener(|this, _, _, cx| {
                                        this.sync.deletion = Some(Deletion::Remove);
                                        cx.notify();
                                    }),
                                ),
                        ),
                );
            }
        }
        page
    }

    /// Three panes (base, spec, collection) and the ways to settle one conflict.
    fn render_conflict(
        &self,
        t: &Theme,
        i: usize,
        ki: usize,
        c: &fluxwire_openapi::Conflict,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let field = c.field.clone();
        let settled = self
            .sync
            .resolutions
            .get(&(i, c.operation.clone(), c.field.clone()))
            .map(|r| match r {
                Resolution::TakeSpec => "Taking the spec",
                Resolution::TakeCollection => "Taking the collection",
                Resolution::Edit(_) => "Edited",
            });
        let pane = |title: &str, v: &serde_json::Value| {
            div()
                .flex_1()
                .p_2()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border_variant)
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_muted)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .child(short(v)),
                )
        };
        let (op1, f1, op2, f2) = (
            c.operation.clone(),
            field.clone(),
            c.operation.clone(),
            field.clone(),
        );
        let (op3, f3) = (c.operation.clone(), field.clone());
        let base_val = c.collection.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded(px(4.))
            .border_1()
            .border_color(t.modified)
            .child(div().child(format!(
                "Conflict · {} · {}",
                c.operation,
                c.field.clone().unwrap_or_else(|| "operation".into())
            )))
            .child(
                row()
                    .gap_2()
                    .child(pane("Base", &c.base))
                    .child(pane("Spec", &c.spec))
                    .child(pane("Collection", &c.collection)),
            )
            .child(
                row()
                    .gap_2()
                    .items_center()
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-spec-{i}-{ki}")),
                            "Take spec".into(),
                            false,
                            true,
                        )
                        .dev_click(
                            "sync.take-spec",
                            Role::Button,
                            "Take spec",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                this.sync
                                    .resolve(i, &op1, f1.as_deref(), Resolution::TakeSpec);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-coll-{i}-{ki}")),
                            "Take collection".into(),
                            false,
                            true,
                        )
                        .dev_click(
                            "sync.take-collection",
                            Role::Button,
                            "Take collection",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                this.sync.resolve(
                                    i,
                                    &op2,
                                    f2.as_deref(),
                                    Resolution::TakeCollection,
                                );
                                cx.notify();
                            }),
                        ),
                    )
                    // Edit settles both sides to one value; start from the collection's text.
                    .child(
                        button(
                            t,
                            SharedString::from(format!("sync-edit-{i}-{ki}")),
                            "Edit…".into(),
                            false,
                            field.is_some(),
                        )
                        .dev_click(
                            "sync.edit",
                            Role::Button,
                            "Edit",
                            field.is_some(),
                            cx.listener(move |this, _, _, cx| {
                                this.sync.resolve(
                                    i,
                                    &op3,
                                    f3.as_deref(),
                                    Resolution::Edit(base_val.clone()),
                                );
                                cx.notify();
                            }),
                        ),
                    )
                    .children(settled.map(|s| div().text_color(t.accent).child(s))),
            )
    }

    pub(crate) fn render_sync_panel(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().flex().flex_col();
        if !self.sync.has_links() {
            return col.child(
                div()
                    .px_2()
                    .pt_2()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("No Spec links."),
            );
        }
        for (i, r) in self.sync.rows.iter().enumerate() {
            let s = r.status;
            col = col.child(
                self.tree_row(
                    t,
                    SharedString::from(format!("sync-link-{i}")).into(),
                    0,
                    self.scope.tabs.active().is_some_and(|a| a.key == TAB_KEY),
                )
                .dev_click(
                    "dock.render_sync_panel",
                    Role::Button,
                    "dock.render_sync_panel",
                    true,
                    cx.listener(|this, ev: &gpui::ClickEvent, _, cx| {
                        this.refresh_sync();
                        this.open_tab(TAB_KEY, "Sync", TabKind::Sync, ev.click_count() >= 2);
                        cx.notify();
                    }),
                )
                .child(icon("refresh-cw", 14., t.text_muted))
                .child(div().flex_1().truncate().child(r.link.source.clone()))
                .child(
                    div()
                        .font_family(BUFFER_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_muted)
                        .child(format!("↓{} ↑{}", s.incoming, s.outgoing)),
                ),
            );
        }
        col
    }
}
