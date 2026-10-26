//! WebSocket requests (SPEC §10): connect and disconnect, saved messages and the composer,
//! and the session log shown under the request form and in History entries.

use std::ops::Range;

use fluxwire_model::{BlockKind, MessageBlock};
use fluxwire_runner::Frame;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::*, px, uniform_list,
};

use crate::{
    assets::BUFFER_FONT,
    center::{button, pill},
    editor::{EditorState, KeyOutcome, Lang, code_view, editor_content},
    highlight::highlight,
    session_log::{DirFilter, SessionLog, detail, fmt_offset, preview},
    theme::Theme,
    ui::row,
    workspace::Workspace,
};

/// Which log a handler acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogRef {
    /// The live (or last) session of a request tab.
    Live(u64),
    /// A stored History entry.
    History(i64),
}

/// Unsaved message being written in the Messages tab.
pub struct Composer {
    pub text: String,
    pub state: EditorState,
    /// `json`, `text`, `xml` or `binary`.
    pub lang: &'static str,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            text: String::new(),
            state: EditorState::default(),
            lang: "json",
        }
    }
}

const ROW_H: f32 = 24.0;
const LANGS: [&str; 3] = ["json", "text", "xml"];

fn lang_of(l: &str) -> Lang {
    match l {
        "json" => Lang::Json,
        "xml" => Lang::Xml,
        _ => Lang::Plain,
    }
}

impl Workspace {
    // ---- connection state ---------------------------------------------------------------------

    pub fn is_ws_tab(&self) -> bool {
        self.tabs
            .active()
            .is_some_and(|t| matches!(t.kind, crate::tabs::TabKind::WebSocket))
    }

    /// The upgrade finished and the session is live.
    pub fn ws_connected(&self, tab_id: u64) -> bool {
        self.responses
            .get(&tab_id)
            .and_then(|r| r.sending.as_ref())
            .is_some_and(|s| s.phases.iter().any(|p| p.name == "upgrade"))
    }

    pub fn ws_connecting(&self, tab_id: u64) -> bool {
        self.responses
            .get(&tab_id)
            .is_some_and(|r| r.sending.is_some())
            && !self.ws_connected(tab_id)
    }

    /// Connect, or close a live session gracefully, or cancel a connection in progress.
    pub fn ws_toggle(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        if self.ws_connected(id) {
            if let Some(s) = self
                .responses
                .get(&id)
                .and_then(|r| r.sending.as_ref())
                .map(|s| s.session.clone())
            {
                s.close();
            }
            cx.notify();
        } else if self.ws_connecting(id) {
            self.cancel_request(cx);
        } else {
            self.send_request(cx);
        }
    }

    /// Cmd/Ctrl+Enter on a WebSocket tab: connect, or send the composer text when it has focus
    /// (else the selected saved message).
    pub fn ws_send_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        if !self.ws_connected(id) {
            if self.ws_connecting(id) {
                self.cancel_request(cx);
            } else {
                self.send_request(cx);
            }
            return;
        }
        if self.composer_focus.is_focused(window)
            && self.composers.get(&id).is_some_and(|c| !c.text.is_empty())
        {
            self.ws_send_composer(cx);
        } else {
            let sel = self.tab_ui.get(&id).map_or(0, |u| u.msg_sel);
            self.ws_send_saved(sel, cx);
        }
    }

    fn ws_session(&self, tab_id: u64) -> Option<fluxwire_runner::SessionHandle> {
        self.responses
            .get(&tab_id)?
            .sending
            .as_ref()
            .map(|s| s.session.clone())
    }

    /// Interpolate variables at send time and queue the frame.
    fn ws_send_text(&mut self, kind: &str, text: &str, file: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let Some(session) = self.ws_session(id).filter(|_| self.ws_connected(id)) else {
            self.notice = Some("Connect first, then send a message.".into());
            cx.notify();
            return;
        };
        let Some(key) = self.active_key() else { return };
        let scope = self.scope_for(&key);
        let data: Vec<u8> = match file {
            Some(path) => {
                let base = scope.as_ref().and_then(|s| s.base_dir.clone());
                let p = match &base {
                    Some(b) if std::path::Path::new(path).is_relative() => b.join(path),
                    _ => std::path::PathBuf::from(path),
                };
                match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.notice = Some(format!("Cannot read {}: {e}", p.display()));
                        cx.notify();
                        return;
                    }
                }
            }
            None => match &scope {
                Some(s) => fluxwire_vars::interpolate(text, s).text.into_bytes(),
                None => text.as_bytes().to_vec(),
            },
        };
        session.send(kind, data);
        cx.notify();
    }

    fn saved_messages(&self) -> Vec<MessageBlock> {
        self.active_key()
            .and_then(|k| self.project.as_ref().and_then(|p| p.doc(&k)))
            .map(|d| d.parsed.messages())
            .unwrap_or_default()
    }

    pub fn ws_send_saved(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(m) = self.saved_messages().into_iter().nth(idx) else {
            self.notice = Some("No saved message to send. Write one in the composer.".into());
            cx.notify();
            return;
        };
        self.ws_send_text(m.frame_kind(), &m.text, m.file.as_deref(), cx);
    }

    pub fn ws_send_composer(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let Some((text, lang)) = self.composers.get(&id).map(|c| (c.text.clone(), c.lang)) else {
            return;
        };
        let kind = if lang == "binary" { "binary" } else { "text" };
        self.ws_send_text(kind, &text, None, cx);
    }

    /// "Save as message": insert a `<lang> message <label>` block with a span insert.
    pub fn ws_save_composer(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let Some(key) = self.active_key() else { return };
        let Some((text, lang)) = self.composers.get(&id).map(|c| (c.text.clone(), c.lang)) else {
            return;
        };
        if text.trim().is_empty() {
            self.notice = Some("The composer is empty.".into());
            cx.notify();
            return;
        }
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let taken: Vec<String> = doc.parsed.messages().into_iter().map(|m| m.label).collect();
        let label = (taken.len() + 1..)
            .map(|n| format!("message-{n}"))
            .find(|l| !taken.contains(l))
            .unwrap_or_default();
        let edits = fluxwire_format::insert_block(
            &doc.parsed,
            BlockKind::Message,
            &format!("{lang} message {label}"),
            &text,
        );
        if doc.apply(&edits) {
            self.notice = Some(format!("Saved as message `{label}`."));
            self.after_edit_pub(&key, cx);
        }
    }

    // ---- composer input -----------------------------------------------------------------------

    pub fn on_composer_key(&mut self, ev: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let clip = cx.read_from_clipboard().and_then(|c| c.text());
        let c = self.composers.entry(id).or_default();
        let mut text = std::mem::take(&mut c.text);
        let out = c.state.handle_key(&mut text, ev, clip.as_deref());
        c.text = text;
        if !matches!(out, KeyOutcome::Ignored) {
            cx.notify();
        }
    }

    fn ws_edit_saved(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        if let Some(m) = self.saved_messages().into_iter().nth(idx) {
            let c = self.composers.entry(id).or_default();
            c.text = m.text;
            c.state = EditorState::default();
            c.lang = match m.language.as_str() {
                "text" => "text",
                "xml" => "xml",
                "binary" => "text",
                _ => "json",
            };
            if let Some(ui) = self.tab_ui.get_mut(&id) {
                ui.msg_sel = idx;
            }
            cx.notify();
        }
    }

    // ---- log state ----------------------------------------------------------------------------

    pub fn log_ref(&self, which: LogRef) -> Option<&SessionLog> {
        match which {
            LogRef::Live(id) => self.responses.get(&id).map(|r| &r.log),
            LogRef::History(id) => self.history_logs.get(&id),
        }
    }

    pub fn log_mut(&mut self, which: LogRef) -> Option<&mut SessionLog> {
        match which {
            LogRef::Live(id) => self.responses.get_mut(&id).map(|r| &mut r.log),
            LogRef::History(id) => self.history_logs.get_mut(&id),
        }
    }

    /// Key presses in the log's text filter.
    pub fn on_log_filter_key(
        &mut self,
        which: LogRef,
        ev: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        let ks = &ev.keystroke;
        let clip = if ks.modifiers.secondary() && ks.key == "v" {
            cx.read_from_clipboard().and_then(|c| c.text())
        } else {
            None
        };
        let Some(log) = self.log_mut(which) else {
            return;
        };
        let changed = match ks.key.as_str() {
            "escape" => {
                log.filter.clear();
                true
            }
            "enter" => false,
            _ => match clip {
                Some(t) => {
                    log.filter.insert(&t);
                    true
                }
                None => log.filter.handle_key(ev),
            },
        };
        if changed {
            log.refilter();
            cx.notify();
        }
    }

    // ---- rendering ----------------------------------------------------------------------------

    /// Messages sub-tab: saved messages, then the composer.
    pub(crate) fn render_ws_messages(
        &self,
        t: &Theme,
        tab_id: u64,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let messages = self.saved_messages();
        let connected = self.ws_connected(tab_id);
        let label = |s: &'static str| {
            row()
                .px_3()
                .pt_2()
                .pb_1()
                .text_size(px(11.))
                .text_color(t.text_placeholder)
                .child(s)
        };
        let list: Vec<AnyElement> = if messages.is_empty() {
            vec![
                div()
                    .px_3()
                    .text_color(t.text_muted)
                    .child("No saved messages. Write one below and choose Save as message.")
                    .into_any_element(),
            ]
        } else {
            messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let on = i == selected;
                    row()
                        .id(("msg", i))
                        .h(px(30.))
                        .px_3()
                        .gap_2()
                        .cursor_pointer()
                        .when(on, |d| d.bg(t.element_selected))
                        .hover({
                            let h = t.element_hover;
                            move |s| s.bg(h)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(ui) = this.active_ui() {
                                ui.msg_sel = i;
                            }
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(m.label.clone()),
                        )
                        .child(pill(t, m.language.clone()))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .font_family(BUFFER_FONT)
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child(match &m.file {
                                    Some(f) => format!("@file({f})"),
                                    None => m.text.split_whitespace().collect::<Vec<_>>().join(" "),
                                }),
                        )
                        .child(
                            button(t, "msg-edit", "Edit", false)
                                .id(("msg-edit", i))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ws_edit_saved(i, cx)
                                })),
                        )
                        .child(
                            button(t, "msg-send", "Send", connected)
                                .id(("msg-send", i))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ws_send_saved(i, cx)
                                })),
                        )
                        .into_any_element()
                })
                .collect()
        };

        let composer = self.composers.get(&tab_id);
        let (text, cursor, lang) = composer.map_or((String::new(), 0, "json"), |c| {
            (c.text.clone(), c.state.cursor, c.lang)
        });
        let spans = highlight(&text, lang_of(lang));
        let focused = self.composer_active;
        let editor = editor_content(
            &text,
            &spans,
            focused.then_some(cursor),
            Some(self.composer_bounds.clone()),
            t,
        );
        div()
            .flex()
            .flex_col()
            .child(label(
                "Saved messages · ⌘/Ctrl+Enter sends the selected one",
            ))
            .children(list)
            .child(
                row()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child("Composer"),
                    )
                    .children(LANGS.map(|l| {
                        div()
                            .id(l)
                            .px_2()
                            .h(px(20.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(if l == lang { t.accent } else { t.text_muted })
                            .hover({
                                let h = t.element_hover;
                                move |s| s.bg(h)
                            })
                            .child(l)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(id) = this.tabs.active_id() {
                                    this.composers.entry(id).or_default().lang = l;
                                }
                                cx.notify();
                            }))
                    }))
                    .child(div().flex_1())
                    .child(
                        button(t, "composer-save", "Save as message", false)
                            .on_click(cx.listener(|this, _, _, cx| this.ws_save_composer(cx))),
                    )
                    .child(
                        button(t, "composer-send", "Send", connected)
                            .on_click(cx.listener(|this, _, _, cx| this.ws_send_composer(cx))),
                    ),
            )
            .child(
                div()
                    .id("composer")
                    .mx_3()
                    .mb_3()
                    .min_h(px(96.))
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if focused { t.accent } else { t.border })
                    .bg(t.editor)
                    .track_focus(&self.composer_focus)
                    .key_context("Editor")
                    .on_key_down(cx.listener(|this, ev, _, cx| this.on_composer_key(ev, cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, w, cx| {
                            w.focus(&this.composer_focus);
                            this.composer_active = true;
                            if let Some(id) = this.tabs.active_id() {
                                let bounds = this.composer_bounds.get();
                                let c = this.composers.entry(id).or_default();
                                c.state.cursor =
                                    EditorState::offset_for_click(&c.text, bounds, ev.position);
                            }
                            cx.notify();
                        }),
                    )
                    .child(editor),
            )
            .into_any_element()
    }

    /// The session log with its toolbar. `status` is the line above the list (state, error).
    pub(crate) fn render_session_log(
        &self,
        t: &Theme,
        which: LogRef,
        status: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(log) = self.log_ref(which) else {
            return div().into_any_element();
        };
        let (sent, received) = log.counts();
        let dir_chip = |id: &'static str, text: String, f: DirFilter| {
            div()
                .id(id)
                .px_2()
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if log.dir == f { t.accent } else { t.text_muted })
                .hover({
                    let h = t.element_hover;
                    move |s| s.bg(h)
                })
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(l) = this.log_mut(which) {
                        l.set_dir(f);
                    }
                    cx.notify();
                }))
        };
        let filter_box = row()
            .id((
                "log-filter",
                match which {
                    LogRef::Live(i) => i as usize,
                    LogRef::History(i) => i as usize,
                },
            ))
            .track_focus(&self.log_focus)
            .on_key_down(cx.listener(move |this, ev, _, cx| this.on_log_filter_key(which, ev, cx)))
            .on_click({
                let f = self.log_focus.clone();
                move |_, w, _| w.focus(&f)
            })
            .w(px(170.))
            .h(px(22.))
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
            .child(if log.filter.text().is_empty() {
                div().text_color(t.text_placeholder).child("Filter text")
            } else {
                div().child(log.filter.text().to_string())
            });
        let toolbar = row()
            .flex_none()
            .h(px(30.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(t.border_variant)
            .child(dir_chip(
                "dir-all",
                format!("All {}", log.len()),
                DirFilter::All,
            ))
            .child(dir_chip(
                "dir-sent",
                format!("↑ Sent {sent}"),
                DirFilter::Sent,
            ))
            .child(dir_chip(
                "dir-recv",
                format!("↓ Received {received}"),
                DirFilter::Received,
            ))
            .child(div().flex_1())
            .when(log.dropped > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(format!("{} older frames dropped", log.dropped)),
                )
            })
            .child(filter_box)
            .when(matches!(which, LogRef::Live(_)), |d| {
                d.child(
                    div()
                        .id("log-clear")
                        .px_2()
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .hover({
                            let h = t.element_hover;
                            move |s| s.bg(h)
                        })
                        .child("Clear")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(l) = this.log_mut(which) {
                                l.clear();
                            }
                            cx.notify();
                        })),
                )
            });

        let n = log.visible_len();
        let list = uniform_list(
            "session-log",
            n,
            cx.processor(move |this: &mut Workspace, range: Range<usize>, _w, cx| {
                let t = crate::theme::theme(cx).clone();
                let Some(log) = this.log_ref(which) else {
                    return Vec::new();
                };
                range
                    .filter_map(|ix| log.visible_entry(ix))
                    .map(|e| {
                        let f: &Frame = &e.frame;
                        let id = e.id;
                        let on = log.selected == Some(id);
                        let kind_color = match f.kind.as_str() {
                            "close" => t.deleted,
                            "ping" | "pong" => t.text_placeholder,
                            _ => t.text_muted,
                        };
                        row()
                            .id(("frame", id as usize))
                            .h(px(ROW_H))
                            .px_3()
                            .gap_3()
                            .cursor_pointer()
                            .when(on, |d| d.bg(t.element_selected))
                            .hover({
                                let h = t.element_hover;
                                move |s| s.bg(h)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(l) = this.log_mut(which) {
                                    l.selected = if l.selected == Some(id) {
                                        None
                                    } else {
                                        Some(id)
                                    };
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(14.))
                                    .text_color(if f.outbound { t.accent } else { t.created })
                                    .child(if f.outbound { "↑" } else { "↓" }),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .text_color(t.text_placeholder)
                                    .child(fmt_offset(f.ts_ms)),
                            )
                            .child(
                                div()
                                    .w(px(52.))
                                    .text_color(kind_color)
                                    .child(f.kind.clone()),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .text_color(t.text_placeholder)
                                    .child(crate::response::fmt_size(f.size() as u64)),
                            )
                            .child(div().flex_1().truncate().child(preview(f)))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(log.scroll.clone())
        .flex_1()
        .min_h_0()
        .font_family(BUFFER_FONT)
        .text_size(px(12.));

        let selected = log.selected.and_then(|id| log.get(id));
        let detail_pane = selected.map(|e| {
            let (text, lang) = detail(&e.frame);
            div()
                .id("log-detail")
                .flex_none()
                .max_h(px(180.))
                .overflow_y_scroll()
                .border_t_1()
                .border_color(t.border_variant)
                .bg(t.editor)
                .font_family(BUFFER_FONT)
                .text_size(px(12.))
                .p_2()
                .child(code_view(&text, lang, t))
        });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(status)
            .child(toolbar)
            .child(if n == 0 {
                div()
                    .flex_1()
                    .p_3()
                    .text_color(t.text_muted)
                    .child(if log.is_empty() {
                        "No frames yet. Connect, then send a message."
                    } else {
                        "No frames match the filters."
                    })
                    .into_any_element()
            } else {
                list.into_any_element()
            })
            .children(detail_pane)
            .into_any_element()
    }

    /// Session log of a History entry, built on first view.
    pub(crate) fn ensure_history_log(&mut self, id: i64, outcome_frames: Vec<Frame>) {
        self.history_logs
            .entry(id)
            .or_insert_with(|| SessionLog::from_frames(&outcome_frames));
    }
}

impl Workspace {
    /// Status line above the live log: state, URL, and any error with its fix.
    pub(crate) fn ws_status_line(&self, t: &Theme, tab_id: u64) -> AnyElement {
        let r = self.responses.get(&tab_id);
        let (text, color) = if self.ws_connected(tab_id) {
            ("● Connected".to_string(), t.created)
        } else if self.ws_connecting(tab_id) {
            ("◌ Connecting…".to_string(), t.text_muted)
        } else if let Some(res) = r.and_then(|r| r.result.as_ref()) {
            match (&res.error, &res.outcome) {
                (Some(e), _) => {
                    let (m, fix) = e.describe();
                    (
                        format!(
                            "✗ {} — {m}{}",
                            e.phase(),
                            fix.map(|f| format!(" {f}")).unwrap_or_default()
                        ),
                        t.deleted,
                    )
                }
                (None, Some(o)) => {
                    let s = o.session.as_ref();
                    let code = s
                        .and_then(|s| s.close_code)
                        .map_or(String::new(), |c| format!(" (code {c})"));
                    let err = s
                        .and_then(|s| s.error.clone())
                        .map_or(String::new(), |e| format!(" · {e}"));
                    (format!("○ Disconnected{code}{err}"), t.text_muted)
                }
                _ => ("○ Not connected".into(), t.text_muted),
            }
        } else {
            ("○ Not connected".into(), t.text_muted)
        };
        row()
            .flex_none()
            .px_3()
            .py_1()
            .text_size(px(12.))
            .text_color(color)
            .child(text)
            .into_any_element()
    }
}
