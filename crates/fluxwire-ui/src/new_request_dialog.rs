//! The `New request…` dialog (M9.07, ADR-017): type, name, location, URL and method or a
//! pasted cURL command, options. Form logic is in `new_request.rs`; this file holds the
//! entities, the view and the create step.

use crate::dev_id::{DevClick, Role};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use fluxwire_format::{InitialBody, RequestSpec};
use fluxwire_runner::{ProtocolDesc, runner};
use gpui::{
    Context, Entity, Focusable, InteractiveElement, ParentElement, StatefulInteractiveElement,
    Styled, Subscription, Window, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    code_editor::{CodeEditor, CodeEditorEvent},
    dialogs::{Dialog, button},
    highlight::Lang,
    location_picker::{self, Location},
    new_request::{Field, HTTP_METHODS, NewKind, NewRequestForm, available_kinds, relative_to},
    segmented::segmented,
    text_input::{TextInput, TextInputEvent},
    theme::Theme,
    ui::{VarTip, icon, row},
    workspace::WorkspaceView,
};

/// The Options disclosure stays as the user left it for the rest of the session.
static OPTIONS_OPEN: AtomicBool = AtomicBool::new(false);

pub struct NewRequestDialog {
    pub form: NewRequestForm,
    pub kinds: Vec<NewKind>,
    pub descs: Vec<ProtocolDesc>,
    pub name: Entity<TextInput>,
    pub url: Entity<TextInput>,
    pub search: Entity<TextInput>,
    pub curl: Entity<CodeEditor>,
    pub desc: Entity<CodeEditor>,
    pub method_open: bool,
    pub picker_open: bool,
    /// A cURL command found on the clipboard when the dialog opened, offered once.
    pub clipboard_hint: Option<String>,
    /// Set after Esc on a non-empty cURL field; a second Esc discards.
    pub esc_armed: bool,
    /// Error from the last Create attempt (containment, write); the dialog stays open.
    pub error: Option<String>,
    pub attempted: bool,
    _subs: Vec<Subscription>,
}

impl NewRequestDialog {
    fn locations(&self, ws: &crate::workspace_model::Workspace) -> Vec<Location> {
        location_picker::locations(ws)
    }
}

/// A `curl …` command on the clipboard, if any.
pub fn curl_on_clipboard(text: Option<String>) -> Option<String> {
    let t = text?;
    let t = t.trim();
    (t.starts_with("curl ") || t.starts_with("curl\n")).then(|| t.to_string())
}

impl WorkspaceView {
    /// Open the New request dialog. With no Collection, New collection opens first.
    pub fn start_new_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        if ws.collections.is_empty() {
            self.start_new_collection(
                Some("Requests live in a collection. Create one first."),
                window,
                cx,
            );
            return;
        }
        let location = self.new_request_location();
        let descs = runner().protocols();
        let kinds = available_kinds(&descs, runner().can_import_paste());
        let Some(&first) = kinds.first() else {
            self.notice = Some("No request types are available in this build.".into());
            cx.notify();
            return;
        };
        let kind = if kinds.contains(&NewKind::Http) {
            NewKind::Http
        } else {
            first
        };
        let mut form = NewRequestForm::new(location, kind);
        form.options_open = OPTIONS_OPEN.load(Ordering::Relaxed);

        let name = cx.new(|cx| TextInput::new("Request name", cx));
        let url = cx.new(|cx| TextInput::new("{{baseUrl}}/path", cx));
        let search = cx.new(|cx| TextInput::new("Filter folders", cx));
        let curl = cx.new(|cx| {
            CodeEditor::new(Lang::Plain, cx)
                .placeholder("curl https://api.example.com/users -H 'Accept: application/json'")
        });
        let desc = cx.new(|cx| CodeEditor::new(Lang::Plain, cx).placeholder("Description"));
        let mut subs = Vec::new();
        for input in [&name, &url, &search] {
            subs.push(cx.subscribe(input, |this, _, ev: &TextInputEvent, cx| {
                if *ev == TextInputEvent::Changed {
                    this.sync_new_request(cx);
                }
            }));
        }
        for ed in [&curl, &desc] {
            subs.push(cx.subscribe(ed, |this, _, ev: &CodeEditorEvent, cx| {
                if *ev == CodeEditorEvent::Changed {
                    this.sync_new_request(cx);
                }
            }));
        }
        let clipboard_hint = curl_on_clipboard(
            cx.read_from_clipboard()
                .and_then(|c| c.text().map(|s| s.to_string())),
        )
        .filter(|_| kinds.contains(&NewKind::Curl));
        window.focus(&name.read(cx).focus_handle(cx));
        self.dialog = Some(Dialog::NewRequest(Box::new(NewRequestDialog {
            form,
            kinds,
            descs,
            name,
            url,
            search,
            curl,
            desc,
            method_open: false,
            picker_open: false,
            clipboard_hint,
            esc_armed: false,
            error: None,
            attempted: false,
            _subs: subs,
        })));
        self.sync_new_request(cx);
        cx.notify();
    }

    /// Open the dialog with `key` (a Collection or Folder) preselected.
    pub fn start_new_request_in(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.scope.selected = Some(key.to_string());
        self.start_new_request(window, cx);
    }

    /// Where `Save in` starts: the selected item's folder, else the active tab's folder, else
    /// the first Collection.
    fn new_request_location(&self) -> PathBuf {
        let Some(ws) = self.scope.workspace.as_ref() else {
            return PathBuf::new();
        };
        let from = |key: &str| {
            let abs = ws.abs_of(key);
            if abs.is_dir() {
                Some(abs)
            } else {
                abs.parent().filter(|p| p.is_dir()).map(Path::to_path_buf)
            }
        };
        self.scope
            .selected
            .as_deref()
            .and_then(from)
            .or_else(|| self.active_key().as_deref().and_then(from))
            .filter(|d| ws.collections.iter().any(|c| d.starts_with(&c.root)))
            .or_else(|| ws.collections.first().map(|c| c.root.clone()))
            .unwrap_or_default()
    }

    /// Copy the field texts into the form and refresh the cURL preview.
    pub(crate) fn sync_new_request(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::NewRequest(d)) = &mut self.dialog else {
            return;
        };
        d.form.name = d.name.read(cx).text().to_string();
        d.form.url = d.url.read(cx).text().to_string();
        d.form.curl = d.curl.read(cx).text().to_string();
        d.form.description = d.desc.read(cx).text().to_string();
        d.form.curl_preview = if d.form.curl.trim().is_empty() {
            None
        } else {
            runner().preview_paste(&d.form.curl)
        };
        d.esc_armed = false;
        d.error = None;
        cx.notify();
    }

    fn nr(&mut self) -> Option<&mut NewRequestDialog> {
        match &mut self.dialog {
            Some(Dialog::NewRequest(d)) => Some(d),
            _ => None,
        }
    }

    pub fn nr_set_kind(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.nr() else { return };
        let Some(&kind) = d.kinds.get(i) else { return };
        d.form.set_kind(kind);
        d.method_open = false;
        let target = if kind == NewKind::Curl {
            d.curl.focus_handle(cx)
        } else {
            d.name.read(cx).focus_handle(cx)
        };
        window.focus(&target);
        cx.notify();
    }

    fn nr_use_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.nr() else { return };
        let Some(text) = d.clipboard_hint.take() else {
            return;
        };
        d.form.set_kind(NewKind::Curl);
        let curl = d.curl.clone();
        curl.update(cx, |e, cx| e.set_text(&text, cx));
        window.focus(&curl.focus_handle(cx));
        self.sync_new_request(cx);
    }

    /// Esc: close, but ask a second Esc when the cURL field has text.
    pub(crate) fn nr_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.nr() else { return };
        if d.picker_open || d.method_open {
            d.picker_open = false;
            d.method_open = false;
        } else if !d.form.curl.trim().is_empty() && !d.esc_armed {
            d.esc_armed = true;
        } else {
            self.close_dialog(window, cx);
            return;
        }
        cx.notify();
    }

    /// Enter in a single-line field creates; in the multi-line fields only Secondary-Enter does.
    pub(crate) fn nr_enter(
        &mut self,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(d) = self.nr() else { return };
        let in_editor = d.curl.focus_handle(cx).is_focused(window)
            || d.desc.focus_handle(cx).is_focused(window);
        if in_editor && !secondary {
            return;
        }
        self.submit_new_request(window, cx);
    }

    /// Write the file, then select and open it pinned with the URL focused.
    pub fn submit_new_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_new_request(cx);
        let Some(Dialog::NewRequest(d)) = &self.dialog else {
            return;
        };
        let form = d.form.clone();
        let Some(ws) = self.scope.workspace.as_ref() else {
            return;
        };
        let root = crate::containment::normalize(&ws.root);
        let inside = crate::containment::normalize(&form.location).starts_with(&root);
        let fail = |this: &mut Self, msg: String, cx: &mut Context<Self>| {
            if let Some(d) = this.nr() {
                d.attempted = true;
                d.error = Some(msg);
            }
            cx.notify();
        };
        if !inside {
            return fail(self, "That location is outside the workspace.".into(), cx);
        }
        let default_url =
            self.default_url_for(&form.location, matches!(form.kind, NewKind::WebSocket));
        let Some(plan) = form.plan(&default_url) else {
            if let Some(d) = self.nr() {
                d.attempted = true;
            }
            cx.notify();
            return;
        };
        let mut text = plan.render();
        if let Some(curl) = &plan.curl {
            let doc = fluxwire_format::parse(&text);
            let applied = runner()
                .edits_from_paste(&doc, curl)
                .and_then(|edits| fluxwire_model::apply_edits(&text, &edits));
            match applied {
                Some(t) => text = t,
                None => {
                    return fail(self, "That cURL command could not be imported.".into(), cx);
                }
            }
        }
        let spec = RequestSpec {
            name: &plan.name,
            description: &plan.description,
            method: &plan.method,
            url: &plan.url,
            websocket: plan.kind == NewKind::WebSocket,
            body: plan.body,
        };
        let path = match fluxwire_format::create_request_from(&plan.dir, &spec, Some(text)) {
            Ok(p) => p,
            Err(e) => return fail(self, format!("Could not create {}: {e}", plan.name), cx),
        };
        self.dialog = None;
        if let Some(p) = self.scope.workspace.as_mut() {
            p.rescan();
        }
        let key = self.key_for(&path).unwrap_or_default();
        // expand every folder above the new request
        let mut parent = key.as_str();
        while let Some((up, _)) = parent.rsplit_once('/') {
            self.scope.collapsed.remove(up);
            parent = up;
        }
        self.scope.collapsed.remove("");
        self.scope.left_open = true;
        self.scope.panel = crate::workspace::Panel::Workspace;
        self.open_request(&key, true, window, cx);
        if let Some(id) = self.scope.tabs.active().map(|t| t.id) {
            self.scope.tabs.toggle_pin(id);
        }
        self.focus_url(window, cx);
        let shown = self
            .scope
            .workspace
            .as_ref()
            .map(|w| w.key_of(&path))
            .unwrap_or_else(|| path.display().to_string());
        self.notice = Some(format!("Created {shown}"));
        cx.notify();
    }

    pub(crate) fn render_new_request(
        &self,
        d: &NewRequestDialog,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let view = cx.entity();
        let form = &d.form;
        let errors = form.validate();
        let shown = |field: Field, touched: bool| {
            errors
                .iter()
                .find(|e| e.field == field)
                .filter(|_| touched || d.attempted)
                .map(|e| e.message.clone())
        };
        let label = |s: &'static str| {
            div()
                .text_size(px(11.))
                .text_color(t.text_placeholder)
                .child(s)
        };
        let boxed = |id: &'static str, on: bool| {
            div()
                .id(id)
                .px_2()
                .min_h(px(28.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if on { t.accent } else { t.border })
                .bg(t.editor)
        };
        let err_line = |m: String| div().text_size(px(12.)).text_color(t.deleted).child(m);
        let ws = self.scope.workspace.as_ref();

        let mut panel = div().flex().flex_col().gap_3();

        // 1. title
        panel = panel.child(
            row()
                .justify_between()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(15.))
                        .child("New request"),
                )
                .child(
                    div()
                        .id("nr-close")
                        .cursor_pointer()
                        .child(icon("x", 14., t.text_muted))
                        .dev_click(
                            "nr-close",
                            Role::Button,
                            "nr-close",
                            true,
                            cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
                        ),
                ),
        );

        // clipboard hint
        if let Some(text) = &d.clipboard_hint {
            let first = text.lines().next().unwrap_or("").to_string();
            panel = panel.child(
                row()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(t.accent_bg())
                    .text_size(px(12.))
                    .child(
                        div()
                            .id("nr-hint")
                            .cursor_pointer()
                            .text_color(t.accent)
                            .child("Paste cURL command from clipboard")
                            .dev_click(
                                "nr-hint",
                                Role::Button,
                                "nr-hint",
                                true,
                                cx.listener(|this, _, w, cx| this.nr_use_clipboard(w, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_family(BUFFER_FONT)
                            .text_color(t.text_muted)
                            .child(first),
                    )
                    .child(
                        div()
                            .id("nr-hint-x")
                            .cursor_pointer()
                            .child(icon("x", 12., t.text_muted))
                            .dev_click(
                                "nr-hint-x",
                                Role::Button,
                                "nr-hint-x",
                                true,
                                cx.listener(|this, _, _, cx| {
                                    if let Some(d) = this.nr() {
                                        d.clipboard_hint = None;
                                    }
                                    cx.notify();
                                }),
                            ),
                    ),
            );
        }

        // 2. type
        let labels: Vec<String> = d.kinds.iter().map(|k| k.label(&d.descs)).collect();
        let selected = d.kinds.iter().position(|k| *k == form.kind).unwrap_or(0);
        let v = view.clone();
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("Type"))
                .child(segmented(
                    t,
                    "nr-kind",
                    labels,
                    selected,
                    move |i, w, cx| v.update(cx, |this, cx| this.nr_set_kind(i, w, cx)),
                )),
        );

        // 3. name + filename preview
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("Name"))
                .child(boxed("nr-name", true).child(div().flex_1().child(d.name.clone())))
                .when_some(shown(Field::Name, !form.name.is_empty()), |d, m| {
                    d.child(err_line(m))
                })
                .when_some(form.file_preview(), |c, (file, collided)| {
                    let rel = ws
                        .map(|w| relative_to(&w.root, &form.location))
                        .unwrap_or_default();
                    let path = if rel.is_empty() {
                        file
                    } else {
                        format!("{rel}/{file}")
                    };
                    let note = if form.name.trim().is_empty() {
                        form.effective_name()
                            .map(|n| format!("Named “{n}”. "))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let tail = if collided { " (name already used)" } else { "" };
                    c.child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.text_muted)
                            .font_family(BUFFER_FONT)
                            .child(format!("{note}Creates {path}{tail}")),
                    )
                }),
        );

        // 4. save in
        let rel = ws
            .map(|w| relative_to(&w.root, &form.location))
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "/".into());
        let mut save_in = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label("Save in"))
            .child(
                boxed("nr-loc", d.picker_open)
                    .cursor_pointer()
                    .justify_between()
                    .child(
                        div()
                            .font_family(BUFFER_FONT)
                            .text_size(px(12.))
                            .truncate()
                            .child(crate::dock::middle_truncate(&rel, 60)),
                    )
                    .child(icon("chevron-down", 12., t.text_muted))
                    .dev_click(
                        "new_request_dialog.render_new_request",
                        Role::Button,
                        "new_request_dialog.render_new_request",
                        true,
                        cx.listener(|this, _, _, cx| {
                            if let Some(d) = this.nr() {
                                d.picker_open = !d.picker_open;
                            }
                            cx.notify();
                        }),
                    ),
            );
        if d.picker_open
            && let Some(w) = ws
        {
            let all = d.locations(w);
            let q = d.search.read(cx).text().to_string();
            let mut list = div()
                .id("nr-picker")
                .max_h(px(160.))
                .overflow_y_scroll()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .bg(t.surface);
            for (i, l) in location_picker::filter(&all, &q).into_iter().enumerate() {
                let on = l.dir == form.location;
                let dir = l.dir.clone();
                list = list.child(
                    row()
                        .id(("nr-loc-item", i))
                        .gap_2()
                        .h(px(24.))
                        .pl(px(8. + 12. * l.depth as f32))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .hover(|s| s.bg(t.element_hover))
                        .child(icon(
                            if l.depth == 0 {
                                "folder-open"
                            } else {
                                "folder"
                            },
                            12.,
                            t.text_muted,
                        ))
                        .child(div().flex_1().child(l.label.clone()))
                        .when(on, |r| r.child(icon("check", 12., t.accent)))
                        .dev_click(
                            "nr-picker",
                            Role::Button,
                            "nr-picker",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.nr() {
                                    d.form.location = dir.clone();
                                    d.picker_open = false;
                                }
                                cx.notify();
                            }),
                        ),
                );
            }
            save_in = save_in
                .child(boxed("nr-search", false).child(div().flex_1().child(d.search.clone())))
                .child(list);
        }
        panel = panel.child(save_in);

        // 5. url row or cURL
        if form.kind == NewKind::Curl {
            let mut c = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label("cURL command"))
                .child(
                    div()
                        .id("nr-curl")
                        .h(px(110.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.editor)
                        .child(d.curl.clone()),
                );
            if let Some(s) = form.curl_summary() {
                c = c.child(
                    div()
                        .text_size(px(12.))
                        .font_family(BUFFER_FONT)
                        .text_color(t.text_muted)
                        .child(s),
                );
            }
            if let Some(m) = shown(Field::Curl, !form.curl.trim().is_empty()) {
                c = c.child(err_line(m));
            }
            for w in form.curl_warnings() {
                c = c.child(div().text_size(px(12.)).text_color(t.modified).child(w));
            }
            panel = panel.child(c);
        } else {
            let http = form.kind == NewKind::Http;
            let mut r = row().gap_2().items_start();
            if http {
                let m = form.method.clone();
                r = r.child(
                    row()
                        .id("nr-method")
                        .w(px(96.))
                        .h(px(28.))
                        .px_2()
                        .justify_between()
                        .cursor_pointer()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.editor)
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .text_color(t.method_color(&m))
                        .child(m)
                        .child(icon("chevron-down", 12., t.text_muted))
                        .dev_click(
                            "nr-method",
                            Role::Button,
                            "nr-method",
                            true,
                            cx.listener(|this, _, _, cx| {
                                if let Some(d) = this.nr() {
                                    d.method_open = !d.method_open;
                                }
                                cx.notify();
                            }),
                        ),
                );
            }
            r = r.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        boxed("nr-url", false)
                            .h(px(28.))
                            .child(div().flex_1().child(d.url.clone())),
                    )
                    .when_some(shown(Field::Url, !form.url.is_empty()), |c, m| {
                        c.child(err_line(m))
                    }),
            );
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(if http { "Method and URL" } else { "URL" }))
                    .child(r),
            );
            if http && d.method_open {
                let mut chips = row().gap_1().flex_wrap();
                for (i, m) in HTTP_METHODS.into_iter().enumerate() {
                    let on = form.method == m;
                    chips = chips.child(
                        div()
                            .id(("nr-m", i))
                            .px_2()
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .rounded(px(4.))
                            .font_family(BUFFER_FONT)
                            .text_size(px(11.))
                            .text_color(t.method_color(m))
                            .when(on, |c| c.bg(t.element_selected))
                            .hover(|s| s.bg(t.element_hover))
                            .child(m)
                            .dev_click(
                                "new_request_dialog.render_new_request",
                                Role::Button,
                                "new_request_dialog.render_new_request",
                                true,
                                cx.listener(move |this, _, _, cx| {
                                    if let Some(d) = this.nr() {
                                        d.form.method = m.to_string();
                                        d.method_open = false;
                                    }
                                    cx.notify();
                                }),
                            ),
                    );
                }
                panel = panel.child(chips);
            }
        }

        // 6. options
        let open = form.options_open;
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    row()
                        .id("nr-options")
                        .gap_1()
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .child(icon(
                            if open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            12.,
                            t.text_muted,
                        ))
                        .child("Options")
                        .dev_click(
                            "nr-options",
                            Role::Button,
                            "nr-options",
                            true,
                            cx.listener(|this, _, _, cx| {
                                if let Some(d) = this.nr() {
                                    d.form.options_open = !d.form.options_open;
                                    OPTIONS_OPEN.store(d.form.options_open, Ordering::Relaxed);
                                }
                                cx.notify();
                            }),
                        ),
                )
                .when(open, |c| {
                    let mut c = c.child(label("Description")).child(
                        div()
                            .id("nr-desc")
                            .h(px(64.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .bg(t.editor)
                            .child(d.desc.clone()),
                    );
                    if form.kind == NewKind::Http {
                        let bodies = [
                            ("None", InitialBody::None),
                            ("JSON", InitialBody::Json),
                            ("Text", InitialBody::Text),
                            ("Form", InitialBody::Form),
                        ];
                        let cur = bodies
                            .iter()
                            .position(|(_, b)| *b == form.body_kind())
                            .unwrap_or(0);
                        let v = view.clone();
                        c = c.child(label("Initial body type")).child(segmented(
                            t,
                            "nr-body",
                            bodies.iter().map(|(l, _)| l.to_string()).collect(),
                            cur,
                            move |i, _, cx| {
                                v.update(cx, |this, cx| {
                                    if let Some(d) = this.nr() {
                                        d.form.body = Some(bodies[i].1);
                                    }
                                    cx.notify();
                                })
                            },
                        ));
                    }
                    c
                }),
        );

        // errors from the last attempt, esc hint
        if let Some(e) = &d.error {
            panel = panel.child(err_line(e.clone()));
        }
        if d.esc_armed {
            panel = panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(t.modified)
                    .child("The cURL command will be discarded. Press Esc again to close."),
            );
        }

        // 7. footer
        let reason = form.first_error();
        let valid = reason.is_none();
        let mut create = button(t, "nr-create", "Create".to_string(), true, valid);
        if let Some(r) = reason.clone() {
            create = create.tooltip(move |_, cx| cx.new(|_| VarTip { text: r.clone() }).into());
        }
        panel.child(
            row()
                .gap_2()
                .justify_end()
                .when_some(reason, |r, m| {
                    r.child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(m),
                    )
                })
                .child(
                    button(t, "nr-cancel", "Cancel".to_string(), false, true).dev_click(
                        "nr-cancel",
                        Role::Button,
                        "nr-cancel",
                        true,
                        cx.listener(|this, _, w, cx| this.close_dialog(w, cx)),
                    ),
                )
                .child(create.dev_click(
                    "new_request_dialog.render_new_request",
                    Role::Button,
                    "new_request_dialog.render_new_request",
                    true,
                    cx.listener(move |this, _, w, cx| {
                        if valid {
                            this.submit_new_request(w, cx)
                        }
                    }),
                )),
        )
    }
}
