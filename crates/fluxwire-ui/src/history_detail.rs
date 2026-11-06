//! The History tab's detail pane (SPEC §13.1): the request snapshot and its response for the
//! selected row, a note field, and the entry actions. A failed send shows its error and phase.

use crate::dev_id::{DevClick, Role};
use fluxwire_store::Entry;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};
use serde_json::Value;

use crate::{
    assets::BUFFER_FONT,
    editor::{Lang, code_view},
    history::{body_text, headers_of, lang_for, pretty, time_label},
    history_filters::ms_label,
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
};

/// The message and phase of a send that failed before a response.
pub fn error_of(payload: &Value) -> Option<(String, Option<String>)> {
    let e = payload.get("error").filter(|e| !e.is_null())?;
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .unwrap_or("The send failed")
        .to_string();
    let phase = e.get("phase").and_then(Value::as_str).map(String::from);
    Some((msg, phase))
}

fn header_lines(v: Option<&Value>) -> String {
    headers_of(v)
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl WorkspaceView {
    /// Put the selected entry's note into the note field.
    pub(crate) fn history_load_note(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.history.selected != Some(id) {
            return;
        }
        let note = self
            .history
            .entries
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.note.clone())
            .or_else(|| {
                self.history
                    .cache
                    .get(&id)
                    .and_then(|e| e.summary.note.clone())
            })
            .unwrap_or_default();
        let input = self.history_note_input(cx);
        input.update(cx, |i, cx| i.set_text(&note, cx));
        self.history.note_for = Some(id);
    }

    pub(crate) fn render_history_detail(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let shell = || {
            div()
                .id("history-detail")
                .w(px(self.history.divider))
                .h_full()
                .flex_none()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .p_3()
                .gap_2()
        };
        let Some(id) = self.history.selected else {
            return shell()
                .items_center()
                .justify_center()
                .text_color(t.text_placeholder)
                .child("Select an entry to see its request and response")
                .into_any_element();
        };
        let Some(e) = self.history.cache.get(&id) else {
            return shell()
                .text_color(t.text_placeholder)
                .child("Loading…")
                .into_any_element();
        };
        self.history_detail_body(shell(), t, id, e, cx)
    }

    fn history_detail_body(
        &self,
        shell: gpui::Stateful<gpui::Div>,
        t: &Theme,
        id: i64,
        e: &Entry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = &e.summary;
        let key = s.request_path.clone();
        let ws = s.protocol == "ws";
        let req = e.payload.get("request").cloned().unwrap_or(Value::Null);
        let res = e.payload.get("response").cloned().unwrap_or(Value::Null);
        let section = |title: &str| {
            div()
                .mt_1()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_muted)
                .child(title.to_string())
        };
        let block = |text: String, lang: Lang| {
            div()
                .p_2()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border_variant)
                .bg(t.editor)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .child(code_view(&text, lang, t))
        };
        let mut meta = vec![time_label(s.ts_ms), ms_label(s.total_ms)];
        if let Some(env) = &s.env {
            meta.push(format!("env {env}"));
        }
        if s.dirty {
            meta.push("sent unsaved".into());
        }
        if e.truncated {
            meta.push("body truncated at 1 MiB".into());
        }
        let status = s.status.map_or("failed".to_string(), |c| c.to_string());
        let pinned = s.pinned;
        let err = error_of(&e.payload);
        let req_body = body_text(req.get("body"));
        let res_body = pretty(&body_text(res.get("body")));
        let extra = |name: &str| {
            e.payload
                .get(name)
                .filter(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
                .and_then(|v| serde_json::to_string_pretty(v).ok())
        };
        let ticked = self.history.ticked.len();
        let open = self.history.actions_menu;
        let item = |id: &'static str, label: String, enabled: bool| {
            let hover = t.element_hover;
            row()
                .id(id)
                .h(px(26.))
                .px_2()
                .text_size(px(12.))
                .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .when(!enabled, |d| d.text_color(t.text_placeholder))
                .child(label)
        };
        let mut menu = div()
            .id("hd-menu")
            .absolute()
            .top(px(28.))
            .right_0()
            .w(px(210.))
            .py_1()
            .rounded(px(6.))
            .border_1()
            .border_color(t.border)
            .bg(t.elevated)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.history.actions_menu = false;
                cx.notify();
            }));
        // Every entry closes the menu first, then acts.
        menu = menu
            .child(item("hd-restore", "Restore".into(), true).dev_click(
                "hd-restore",
                Role::Button,
                "hd-restore",
                true,
                cx.listener({
                    let key = key.clone();
                    move |this, _, w, cx| {
                        this.history.actions_menu = false;
                        this.history_restore(id, key.clone(), w, cx)
                    }
                }),
            ))
            .child(item("hd-curl", "Copy as cURL".into(), true).dev_click(
                "hd-curl",
                Role::Button,
                "hd-curl",
                true,
                cx.listener(move |this, _, _, cx| {
                    this.history.actions_menu = false;
                    this.history_copy_curl(id, cx)
                }),
            ))
            .child(
                item("hd-prev", "Compare to previous".into(), true).dev_click(
                    "hd-prev",
                    Role::Button,
                    "hd-prev",
                    true,
                    cx.listener(move |this, _, _, cx| {
                        this.history.actions_menu = false;
                        this.history_compare_previous(id, cx)
                    }),
                ),
            )
            .child(
                item(
                    "hd-diff",
                    format!("Diff ticked rows ({ticked}/2)"),
                    ticked == 2,
                )
                .dev_click(
                    "hd-diff",
                    Role::Button,
                    "hd-diff",
                    true,
                    cx.listener(|this, _, _, cx| {
                        this.history.actions_menu = false;
                        this.history_diff_ticked(cx)
                    }),
                ),
            )
            .child(
                item("hd-pin", if pinned { "Unpin" } else { "Pin" }.into(), true).dev_click(
                    "hd-pin",
                    Role::Button,
                    "hd-pin",
                    true,
                    cx.listener(move |this, _, _, cx| {
                        this.history.actions_menu = false;
                        this.history_set_pinned(vec![id], !pinned, cx)
                    }),
                ),
            )
            .child(item("hd-goto", "Go to request".into(), true).dev_click(
                "hd-goto",
                Role::Button,
                "hd-goto",
                true,
                cx.listener({
                    let key = key.clone();
                    move |this, _, w, cx| {
                        this.history.actions_menu = false;
                        this.open_request(&key, true, w, cx)
                    }
                }),
            ))
            .child(item("hd-popout", "Open in tab".into(), true).dev_click(
                "hd-popout",
                Role::Button,
                "hd-popout",
                true,
                cx.listener(move |this, _, _, cx| {
                    this.history.actions_menu = false;
                    this.open_history_entry(id, true, cx)
                }),
            ));
        let more = div()
            .relative()
            .flex_none()
            .child(
                div()
                    .id("hd-more")
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(open, |d| d.bg(t.element_selected))
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .child(crate::ui::icon("ellipsis", 16., t.text_muted))
                    .dev_click(
                        "hd-more",
                        Role::Button,
                        "hd-more",
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.history.actions_menu = !this.history.actions_menu;
                            cx.notify();
                        }),
                    ),
            )
            .when(open, |d| d.child(gpui::deferred(menu).with_priority(1)));
        shell
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_color(t.method_color(&s.method))
                            .child(s.method.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_family(BUFFER_FONT)
                            .child(s.url.clone()),
                    )
                    .child(
                        div()
                            .text_color(match s.status {
                                Some(c) => t.status_color(c as u16),
                                None => t.deleted,
                            })
                            .child(status),
                    ),
            )
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(meta.join(" · ")),
                    )
                    .child(more),
            )
            .child(
                row()
                    .id("hd-note")
                    .h(px(26.))
                    .px_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(12.))
                    .overflow_hidden()
                    .children(
                        (self.history.note_for == Some(id))
                            .then(|| self.history.note_input.clone())
                            .flatten(),
                    ),
            )
            .children(err.map(|(msg, phase)| {
                div()
                    .p_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.deleted)
                    .text_color(t.deleted)
                    .child(match phase {
                        Some(p) => format!("Failed during {p}: {msg}"),
                        None => format!("Failed: {msg}"),
                    })
            }))
            .child(section(if ws {
                "Handshake request headers"
            } else {
                "Request headers"
            }))
            .child(block(header_lines(req.get("headers")), Lang::Plain))
            .when(!ws, |d| {
                d.child(section("Request body"))
                    .child(block(req_body.clone(), lang_for(&req_body)))
            })
            .child(section(if ws {
                "Handshake response headers"
            } else {
                "Response headers"
            }))
            .child(block(header_lines(res.get("headers")), Lang::Plain))
            .when(!ws, |d| {
                d.child(section("Response body"))
                    .child(block(res_body.clone(), lang_for(&res_body)))
            })
            .children(
                extra("phases")
                    .map(|x| div().child(section("Timeline")).child(block(x, Lang::Json))),
            )
            .children(
                extra("tests").map(|x| div().child(section("Tests")).child(block(x, Lang::Json))),
            )
            .children(
                extra("console")
                    .map(|x| div().child(section("Console")).child(block(x, Lang::Json))),
            )
            .when(ws, |d| {
                d.child(section("Transcript")).child(
                    div()
                        .text_color(t.text_muted)
                        .child("Open the entry in a tab to read the session transcript."),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failed_sends_report_message_and_phase() {
        assert_eq!(error_of(&json!({})), None);
        assert_eq!(error_of(&json!({"error": null})), None);
        assert_eq!(
            error_of(&json!({"error": {"message": "refused", "phase": "connect"}})),
            Some(("refused".into(), Some("connect".into())))
        );
        assert_eq!(
            error_of(&json!({"error": "boom"})),
            Some(("boom".into(), None))
        );
    }
}
