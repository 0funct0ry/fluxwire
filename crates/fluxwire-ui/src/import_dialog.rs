//! The Import dialog (M11, SPEC §14): pick a file (or drop one on the window), confirm the
//! detected source, choose the target folder, read the preview tree and report, confirm
//! overwrites per file, then Import. The logic is in `import_wizard.rs`.

use std::path::PathBuf;

use fluxwire_import::{PlannedKind, Source};
use gpui::{
    Context, InteractiveElement, ParentElement, PathPromptOptions, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    dev_id::{DevClick, Role},
    dialogs::{Dialog, button},
    import_wizard::{ALL_SOURCES, ImportWizard},
    location_picker::{self, Location},
    segmented::segmented,
    theme::Theme,
    ui::{icon, row},
    workspace::WorkspaceView,
};

pub struct ImportDialog {
    pub wizard: ImportWizard,
    pub picker_open: bool,
    /// Error from the last Import attempt; the dialog stays open.
    pub error: Option<String>,
}

fn source_label(s: Source) -> &'static str {
    s.label()
}

impl WorkspaceView {
    /// Open the Import dialog, optionally with `path` already chosen (drag-and-drop).
    pub fn start_import(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            self.notice = Some("Open a workspace before importing.".into());
            cx.notify();
            return;
        };
        let roots: Vec<PathBuf> = ws.collections.iter().map(|c| c.root.clone()).collect();
        let target = self.import_default_target(&roots, ws.root.clone());
        let mut wizard = ImportWizard::new(ws.root.clone(), roots, target);
        if let Some(p) = path {
            wizard.set_src(p);
        }
        window.focus(&self.dialog_focus);
        self.dialog = Some(Dialog::Import(Box::new(ImportDialog {
            wizard,
            picker_open: false,
            error: None,
        })));
        cx.notify();
    }

    fn import_default_target(&self, roots: &[PathBuf], ws_root: PathBuf) -> PathBuf {
        let selected = self.scope.selected.as_deref().and_then(|k| {
            let ws = self.scope.workspace.as_ref()?;
            let abs = ws.abs_of(k);
            let dir = if abs.is_dir() {
                abs
            } else {
                abs.parent()?.to_path_buf()
            };
            roots.iter().any(|r| dir.starts_with(r)).then_some(dir)
        });
        selected
            .or_else(|| roots.first().cloned())
            .unwrap_or(ws_root)
    }

    fn imp(&mut self) -> Option<&mut ImportDialog> {
        match &mut self.dialog {
            Some(Dialog::Import(d)) => Some(d),
            _ => None,
        }
    }

    fn import_choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(p) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, _w, cx| {
                if let Some(d) = this.imp() {
                    d.wizard.set_src(p);
                    d.error = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A file dropped on the window: open the wizard on it (or re-point an open one).
    pub fn import_drop(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = paths.first().cloned() else {
            return;
        };
        if let Some(d) = self.imp() {
            d.wizard.set_src(p);
            d.error = None;
            cx.notify();
        } else {
            self.start_import(Some(p), window, cx);
        }
    }

    /// Write the plan, rescan, and open the first imported request.
    pub fn submit_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.imp() else { return };
        if !d.wizard.can_import() {
            return;
        }
        match d.wizard.import() {
            Err(e) => {
                d.error = Some(e);
                cx.notify();
            }
            Ok(out) => {
                let (ws_root, lossy) = (d.wizard.ws_root.clone(), d.wizard.lossy_count());
                self.dialog = None;
                window.focus(&self.root_focus);
                if let Some(p) = self.scope.workspace.as_mut() {
                    p.rescan();
                }
                let mut msg = format!(
                    "Imported {} request{} ({} files written)",
                    out.requests,
                    if out.requests == 1 { "" } else { "s" },
                    out.written.len()
                );
                if lossy > 0 {
                    msg.push_str(&format!(", {lossy} lossy conversions; see the report"));
                }
                if !out.skipped.is_empty() {
                    msg.push_str(&format!(", {} existing files kept", out.skipped.len()));
                }
                self.notice = Some(msg);
                if let Some(first) = out.first_request
                    && let Some(key) = self.key_for(&ws_root.join(first))
                {
                    let mut parent = key.as_str();
                    while let Some((up, _)) = parent.rsplit_once('/') {
                        self.scope.collapsed.remove(up);
                        parent = up;
                    }
                    self.scope.left_open = true;
                    self.scope.panel = crate::workspace::Panel::Workspace;
                    self.open_request(&key, false, window, cx);
                }
                cx.notify();
            }
        }
    }

    pub(crate) fn render_import(
        &self,
        d: &ImportDialog,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let w = &d.wizard;
        let label = |s: &'static str| {
            div()
                .text_size(px(11.))
                .text_color(t.text_placeholder)
                .child(s)
        };
        let boxed = || {
            row()
                .px_2()
                .min_h(px(28.))
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .bg(t.editor)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
        };
        let mut panel = div().flex().flex_col().gap_3();
        panel = panel.child(
            row()
                .justify_between()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child("Import"),
                )
                .child(
                    div()
                        .id("imp-close")
                        .cursor_pointer()
                        .child(icon("x", 14., t.text_muted))
                        .dev_click(
                            "imp-close",
                            Role::Button,
                            "imp-close",
                            true,
                            cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
                        ),
                ),
        );

        // 1. file
        let shown = w
            .src
            .as_ref()
            .map(|p| crate::dock::middle_truncate(&p.display().to_string(), 52))
            .unwrap_or_else(|| "Drop a file here or choose one".to_string());
        panel = panel.child(
            div().flex().flex_col().gap_1().child(label("File")).child(
                row()
                    .gap_2()
                    .child(
                        boxed()
                            .flex_1()
                            .truncate()
                            .text_color(if w.src.is_some() {
                                t.text
                            } else {
                                t.text_placeholder
                            })
                            .child(shown),
                    )
                    .child(
                        button(t, "imp-choose", "Choose…".to_string(), false, true).dev_click(
                            "imp-choose",
                            Role::Button,
                            "imp-choose",
                            true,
                            cx.listener(|this, _, w, cx| this.import_choose_file(w, cx)),
                        ),
                    ),
            ),
        );

        // 2. source
        let items: Vec<String> = ALL_SOURCES
            .iter()
            .map(|s| {
                if w.detected == Some(*s) {
                    format!("{} ✓", source_label(*s))
                } else {
                    source_label(*s).to_string()
                }
            })
            .collect();
        let selected = w
            .source
            .and_then(|s| ALL_SOURCES.iter().position(|x| *x == s))
            .unwrap_or(usize::MAX);
        let view = cx.entity();
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("Source"))
                .child(segmented(
                    t,
                    "imp-source",
                    items,
                    selected,
                    move |i, _w, cx| {
                        view.update(cx, |this, cx| {
                            if let Some(d) = this.imp() {
                                d.wizard.set_source(ALL_SOURCES[i]);
                            }
                            cx.notify();
                        });
                    },
                )),
        );

        // 3. target
        let rel = w
            .target
            .strip_prefix(&w.ws_root)
            .map(|p| {
                if p.as_os_str().is_empty() {
                    "./".to_string()
                } else {
                    format!("{}/", p.display())
                }
            })
            .unwrap_or_else(|_| w.target.display().to_string());
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("Import into"))
                .child(
                    row()
                        .gap_2()
                        .child(boxed().flex_1().truncate().child(rel))
                        .child(
                            button(
                                t,
                                "imp-target",
                                if d.picker_open { "Done" } else { "Change" }.to_string(),
                                false,
                                true,
                            )
                            .dev_click(
                                "imp-target",
                                Role::Button,
                                "imp-target",
                                true,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(d) = this.imp() {
                                        d.picker_open = !d.picker_open;
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                ),
        );
        if d.picker_open
            && let Some(ws) = self.scope.workspace.as_ref()
        {
            let mut locs = vec![Location {
                dir: ws.root.clone(),
                label: "Workspace root".into(),
                depth: 0,
            }];
            locs.extend(location_picker::locations(ws));
            let mut list = div()
                .id("imp-locations")
                .max_h(px(120.))
                .overflow_y_scroll()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border);
            for (i, l) in locs.into_iter().enumerate() {
                let on = l.dir == w.target;
                let dir = l.dir.clone();
                list = list.child(
                    div()
                        .id(("imp-loc", i))
                        .pl(px(8. + 12. * l.depth as f32))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(12.))
                        .when(on, |d| d.bg(t.accent_bg()).text_color(t.accent))
                        .hover(|s| s.bg(t.element_hover))
                        .child(l.label)
                        .dev_click(
                            "imp-loc",
                            Role::Button,
                            "imp-loc",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.imp() {
                                    d.wizard.set_target(dir.clone());
                                    d.picker_open = false;
                                }
                                cx.notify();
                            }),
                        ),
                );
            }
            panel = panel.child(list);
        }

        // 4. options
        let check = |id: &'static str, on: bool, text: &'static str| {
            row()
                .id(id)
                .gap_2()
                .cursor_pointer()
                .text_size(px(12.))
                .child(icon(
                    if on { "check" } else { "square" },
                    14.,
                    if on { t.accent } else { t.text_muted },
                ))
                .child(text)
        };
        if w.spec_link_available() {
            panel = panel.child(
                check(
                    "imp-speclink",
                    w.spec_link,
                    "Create a Spec link to this document",
                )
                .dev_click(
                    "imp-speclink",
                    Role::Button,
                    "imp-speclink",
                    true,
                    cx.listener(|this, _, _, cx| {
                        if let Some(d) = this.imp() {
                            let on = !d.wizard.spec_link;
                            d.wizard.set_spec_link(on);
                        }
                        cx.notify();
                    }),
                ),
            );
        }
        panel = panel.child(
            check("imp-writereport", w.write_report, "Save import-report.md").dev_click(
                "imp-writereport",
                Role::Button,
                "imp-writereport",
                true,
                cx.listener(|this, _, _, cx| {
                    if let Some(d) = this.imp() {
                        d.wizard.write_report = !d.wizard.write_report;
                    }
                    cx.notify();
                }),
            ),
        );

        // 5. preview + report
        if let Some(e) = w.error.as_ref().or(d.error.as_ref()) {
            panel = panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(t.deleted)
                    .child(e.clone()),
            );
        }
        if let Some(plan) = &w.plan {
            let conflicts = w.conflicts();
            let mut head = row().justify_between().child(label("Preview"));
            if !conflicts.is_empty() {
                let all = w.overwrite.len() == conflicts.len();
                head = head.child(
                    div()
                        .id("imp-all")
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(t.accent)
                        .child(if all {
                            "Keep all existing"
                        } else {
                            "Overwrite all existing"
                        })
                        .dev_click(
                            "imp-all",
                            Role::Button,
                            "imp-all",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.imp() {
                                    d.wizard.set_all_overwrite(!all);
                                }
                                cx.notify();
                            }),
                        ),
                );
            }
            let mut tree = div()
                .id("imp-tree")
                .max_h(px(180.))
                .overflow_y_scroll()
                .py_1()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .font_family(BUFFER_FONT)
                .text_size(px(12.));
            for (i, r) in w.tree().into_iter().enumerate() {
                let mut line = row()
                    .id(("imp-row", i))
                    .gap_2()
                    .pl(px(8. + 14. * r.depth as f32))
                    .h(px(22.))
                    .child(
                        div()
                            .text_color(if r.path.is_none() {
                                t.text_muted
                            } else {
                                t.text
                            })
                            .child(r.label.clone()),
                    );
                if matches!(r.kind, Some(PlannedKind::LocalEnv)) {
                    line = line.child(
                        div()
                            .text_color(t.text_placeholder)
                            .child("secrets, git-ignored"),
                    );
                }
                if r.exists
                    && let Some(path) = r.path.clone()
                {
                    line = line
                        .cursor_pointer()
                        .child(icon(
                            if r.overwrite { "check" } else { "square" },
                            13.,
                            if r.overwrite { t.accent } else { t.text_muted },
                        ))
                        .child(
                            div()
                                .text_color(if r.overwrite { t.deleted } else { t.text_muted })
                                .child(if r.overwrite {
                                    "exists, will overwrite"
                                } else {
                                    "exists, will be kept"
                                }),
                        )
                        .dev_click(
                            "imp-row",
                            Role::Button,
                            "imp-row",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.imp() {
                                    d.wizard.toggle_overwrite(&path);
                                }
                                cx.notify();
                            }),
                        );
                }
                tree = tree.child(line);
            }
            panel = panel.child(div().flex().flex_col().gap_1().child(head).child(tree));

            let n = plan.report.lossy.len();
            let mut report = div().flex().flex_col().gap_1().child(label(if n == 0 {
                "Report: nothing was lost"
            } else {
                "Report: lossy conversions"
            }));
            if n > 0 {
                let mut list = div()
                    .id("imp-report")
                    .max_h(px(110.))
                    .overflow_y_scroll()
                    .p_2()
                    .gap_1()
                    .flex()
                    .flex_col()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(12.));
                for l in &plan.report.lossy {
                    let at = if l.file.is_empty() {
                        String::new()
                    } else {
                        format!("{}: ", l.file)
                    };
                    list = list.child(
                        div()
                            .text_color(t.text_muted)
                            .child(format!("{at}{}. {}", l.feature, l.reason)),
                    );
                }
                report = report.child(list);
            }
            panel = panel.child(report);
        }

        // 6. buttons
        let can = w.can_import();
        panel.child(
            row()
                .gap_2()
                .justify_end()
                .child(
                    button(t, "imp-cancel", "Cancel".to_string(), false, true).dev_click(
                        "imp-cancel",
                        Role::Button,
                        "imp-cancel",
                        true,
                        cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
                    ),
                )
                .child(
                    button(t, "imp-go", "Import".to_string(), true, can).dev_click(
                        "imp-go",
                        Role::Button,
                        "imp-go",
                        true,
                        cx.listener(move |this, _, w, cx| {
                            if can {
                                this.submit_import(w, cx)
                            }
                        }),
                    ),
                ),
        )
    }
}
