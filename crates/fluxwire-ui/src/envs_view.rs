//! Views for environments, secrets, cookies and the Auth tab (SPEC §8, §9.3, §9.4, §17.3).

use fluxwire_model::FmValue;
use fluxwire_protocol::{Cookie, now_unix};
use fluxwire_runner::{SendOptions, net_config_from_dir, runner, settings_from_defaults};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    center::{button, pill},
    envs::{EditTarget, EnvCol},
    tabs::{Tab, TabKind},
    theme::Theme,
    ui::{icon, row},
    workspace::Workspace,
};

const MASK: &str = "••••••••";

/// Auth types offered in the picker: (label, `type` value).
const AUTH_TYPES: [(&str, &str); 5] = [
    ("Basic", "basic"),
    ("Bearer", "bearer"),
    ("API key", "apikey"),
    ("Digest", "digest"),
    ("OAuth2", "oauth2"),
];

const GRANTS: [&str; 4] = [
    "client_credentials",
    "authorization_code",
    "password",
    "refresh_token",
];

/// Fields to edit for an auth type: (front matter key, label).
fn auth_fields(ty: &str, grant: &str) -> Vec<(&'static str, &'static str)> {
    match ty {
        "basic" | "digest" => vec![("username", "Username"), ("password", "Password")],
        "bearer" => vec![("token", "Token")],
        "apikey" => vec![("key", "Key"), ("value", "Value")],
        "oauth2" => {
            let mut v = vec![("token_url", "Token URL")];
            if grant == "authorization_code" {
                v.push(("auth_url", "Authorization URL"));
            }
            v.push(("client_id", "Client ID"));
            v.push(("client_secret", "Client secret"));
            match grant {
                "password" => {
                    v.push(("username", "Username"));
                    v.push(("password", "Password"));
                }
                "refresh_token" => v.push(("refresh_token", "Refresh token")),
                "authorization_code" => v.push(("redirect_port", "Redirect port")),
                _ => {}
            }
            v.push(("scope", "Scope"));
            v.push(("audience", "Audience"));
            v
        }
        _ => vec![],
    }
}

fn fmt_expiry(expires_at: Option<i64>) -> String {
    match expires_at {
        None => "no expiry reported".into(),
        Some(e) => {
            let left = e - now_unix();
            if left <= 0 {
                "expired".into()
            } else if left < 120 {
                format!("expires in {left} s")
            } else if left < 7200 {
                format!("expires in {} min", left / 60)
            } else {
                format!("expires in {} h", left / 3600)
            }
        }
    }
}

impl Workspace {
    fn edit_text_for(&self, target: &EditTarget) -> Option<String> {
        self.envs
            .edit
            .as_ref()
            .filter(|e| &e.target == target)
            .map(|e| e.text.text().to_string())
    }

    /// A single-line text field that is edited in place. `shown` is used while not editing.
    fn field(
        &self,
        t: &Theme,
        id: impl Into<gpui::ElementId>,
        target: EditTarget,
        shown: String,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editing = self.edit_text_for(&target);
        let base = row()
            .id(id)
            .h(px(24.))
            .px_1()
            .rounded(px(3.))
            .overflow_hidden()
            .cursor_text()
            .border_1()
            .border_color(if editing.is_some() {
                t.accent
            } else {
                gpui::transparent_black()
            })
            .font_family(BUFFER_FONT)
            .text_size(px(12.));
        match editing {
            Some(text) => base
                .track_focus(&self.env_focus)
                .on_key_down(cx.listener(|this, ev, w, cx| this.on_env_key(ev, w, cx)))
                .child(text)
                .child(div().w(px(1.)).h(px(14.)).bg(t.accent))
                .into_any_element(),
            None => {
                let start = shown.clone();
                base.on_click(cx.listener(move |this, _, w, cx| {
                    this.start_field_edit(target.clone(), start.clone(), w, cx)
                }))
                .when(shown.is_empty(), |d| {
                    d.text_color(t.text_placeholder).child(placeholder)
                })
                .when(!shown.is_empty(), |d| {
                    d.children(crate::ui::with_variables(&shown, t, None))
                })
                .into_any_element()
            }
        }
    }

    /// `{{` suggestions under the URL bar while one is being typed.
    pub(crate) fn completion_strip(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.envs.completion.as_ref()?;
        Some(
            row()
                .id("var-completion")
                .flex_none()
                .gap_1()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(t.border_variant)
                .text_size(px(12.))
                .font_family(BUFFER_FONT)
                .child(div().text_color(t.text_placeholder).child("variables"))
                .children(c.names.iter().enumerate().map(|(i, n)| {
                    let on = i == c.selected;
                    row()
                        .id(("var-choice", i))
                        .px_2()
                        .h(px(20.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(if on { t.accent } else { t.border_variant })
                        .text_color(if on { t.accent } else { t.text_muted })
                        .on_click({
                            let n = n.clone();
                            let target = c.target;
                            cx.listener(move |this, _, _, cx| {
                                this.accept_completion_click(target, &n, cx)
                            })
                        })
                        .child(n.clone())
                }))
                .into_any_element(),
        )
    }

    // ---- Environment tab ---------------------------------------------------------------------

    pub(crate) fn render_environment(
        &self,
        t: &Theme,
        tab: &Tab,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = tab.key.strip_prefix("env/").unwrap_or(&tab.key).to_string();
        let rows = if self.envs.loaded.contains_key(&name) {
            self.env_rows(&name)
        } else {
            // Not loaded yet (first frame): read straight from disk.
            self.env_root()
                .map(|r| fluxwire_format::load_environment(&r, &name, &[]).rows)
                .unwrap_or_default()
        };
        let tracked = self.envs.tracked.get(&name).copied().unwrap_or(false);
        let active = self.envs.active.as_deref() == Some(name.as_str());
        let n = rows.len();
        // A row that is being added is drawn (empty) until it has a name and reaches the file.
        let mut rows = rows;
        if self.envs.edit.as_ref().is_some_and(
            |e| matches!(&e.target, EditTarget::Env { env, row, .. } if *env == name && *row >= n),
        ) {
            rows.push(fluxwire_format::EnvRow {
                name: String::new(),
                value: String::new(),
                secret: false,
                local: false,
                line: 0,
            });
        }
        let nm = name.clone();
        let mut page = div()
            .id("env-page")
            .flex_1()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(name.clone()),
                    )
                    .when(active, |d| d.child(pill(t, "active"))),
            )
            .child(div().text_color(t.text_muted).child(format!(
                "environments/{name}.env holds shared values; secret values live in environments/{name}.local.env, which is git-ignored."
            )));
        if tracked {
            page = page.child(
                div()
                    .p_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.deleted)
                    .text_color(t.deleted)
                    .text_size(px(12.))
                    .child(format!(
                        "{name}.local.env is tracked by git, so its secrets may be published. Run: git rm --cached environments/{name}.local.env"
                    )),
            );
        }
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
        let table = div()
            .flex()
            .flex_col()
            .child(
                row()
                    .h(px(24.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(head("Name", Some(200.)))
                    .child(head("Value", None))
                    .child(head("Secret", Some(48.)))
                    .child(head("", Some(16.))),
            )
            .children(rows.into_iter().enumerate().map(|(i, r)| {
                let env = nm.clone();
                let masked = r.secret && !self.envs.is_revealed(&r.name);
                let editing_value = self.edit_text_for(&EditTarget::Env {
                    env: env.clone(),
                    row: i,
                    col: EnvCol::Value,
                });
                let value_cell: AnyElement = if masked && editing_value.is_none() {
                    let var = r.name.clone();
                    row()
                        .id(("env-val-mask", i))
                        .h(px(24.))
                        .px_1()
                        .flex_1()
                        .cursor_pointer()
                        .font_family(BUFFER_FONT)
                        .text_size(px(12.))
                        .text_color(t.text_muted)
                        .on_click(cx.listener(move |this, _, _, cx| this.reveal_secret(&var, cx)))
                        .child(MASK)
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(t.text_placeholder)
                                .child("click to reveal"),
                        )
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.field(
                            t,
                            ("env-val", i),
                            EditTarget::Env {
                                env: env.clone(),
                                row: i,
                                col: EnvCol::Value,
                            },
                            r.value.clone(),
                            "value",
                            cx,
                        ))
                        .into_any_element()
                };
                let (e1, e2) = (env.clone(), env.clone());
                row()
                    .h(px(28.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .child(div().w(px(200.)).flex_none().child(self.field(
                        t,
                        ("env-key", i),
                        EditTarget::Env {
                            env,
                            row: i,
                            col: EnvCol::Name,
                        },
                        r.name.clone(),
                        "name",
                        cx,
                    )))
                    .child(value_cell)
                    .child(
                        div()
                            .id(("env-secret", i))
                            .w(px(48.))
                            .flex_none()
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.toggle_env_secret(&e1, i, cx)
                                }),
                            )
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(t.border)
                                    .when(r.secret, |d| d.bg(t.accent).border_color(t.accent)),
                            ),
                    )
                    .child(
                        div()
                            .id(("env-del", i))
                            .w(px(16.))
                            .flex_none()
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.delete_env_row(&e2, i, cx)),
                            )
                            .child(icon("x", 12., t.text_placeholder)),
                    )
            }))
            .child({
                let env = nm.clone();
                row()
                    .id("env-add")
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
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.start_env_edit(&env, n, EnvCol::Name, w, cx)
                    }))
                    .child(icon("plus", 12., t.text_muted))
                    .child("Add variable")
            });
        page = page
            .child(table)
            .child(self.render_cookies_section(t, &name, cx));
        page
    }

    fn render_cookies_section(&self, t: &Theme, env: &str, cx: &mut Context<Self>) -> AnyElement {
        let jar: Vec<Cookie> = self
            .env_root()
            .and_then(|r| self.envs.states.get(&(r, env.to_string())))
            .map(|s| s.cookies.list())
            .unwrap_or_default();
        let en = env.to_string();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("Cookies · {}", jar.len())),
                    )
                    .child(div().flex_1())
                    .child(
                        button(t, "cookies-clear", "Clear cookies", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.clear_cookies(&en, cx))),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(t.text_muted).child(
                "Each environment has its own cookie jar. Enable persist_cookies in settings.toml to keep it between runs.",
            ))
            .when(jar.is_empty(), |d| {
                d.child(div().py_2().text_color(t.text_placeholder).child("No cookies yet."))
            })
            .children(jar.into_iter().enumerate().map(|(i, c)| {
                let (dom, path, name) = (c.domain.clone(), c.path.clone(), c.name.clone());
                row()
                    .h(px(28.))
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.border_variant)
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .child(div().w(px(200.)).flex_none().truncate().child(format!("{}{}", c.domain, c.path)))
                    .child(div().w(px(120.)).flex_none().truncate().text_color(t.text).child(c.name.clone()))
                    .child(div().flex_1().min_w_0().child(self.field(
                        t,
                        ("cookie-val", i),
                        EditTarget::Cookie { idx: i },
                        c.value.clone(),
                        "value",
                        cx,
                    )))
                    .child(
                        div()
                            .id(("cookie-del", i))
                            .w(px(16.))
                            .flex_none()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_cookie(&dom, &path, &name, cx)))
                            .child(icon("x", 12., t.text_placeholder)),
                    )
            }))
            .into_any_element()
    }

    pub fn clear_cookies(&mut self, env: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root()
            && let Some(s) = self.envs.states.get(&(root.clone(), env.to_string()))
        {
            s.cookies.clear();
            self.persist_cookies(&root);
        }
        cx.notify();
    }

    pub fn remove_cookie(&mut self, domain: &str, path: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root()
            && let Some(env) = self.envs.active.clone()
            && let Some(s) = self.envs.states.get(&(root.clone(), env))
        {
            s.cookies.remove(domain, path, name);
            self.persist_cookies(&root);
        }
        cx.notify();
    }

    // ---- Environments panel ------------------------------------------------------------------

    pub(crate) fn render_envs(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let names = self.env_names();
        let active = self.envs.active.clone();
        let rows: Vec<crate::envs::EnvVarView> = active
            .as_deref()
            .map(|n| self.env_var_views(n))
            .unwrap_or_default();
        let action = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_1()
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(t.text_muted)
                .hover({
                    let c = t.text;
                    move |s| s.text_color(c)
                })
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .child(
                row()
                    .px_2()
                    .h(px(24.))
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child("Environments"),
                    )
                    .child(
                        div()
                            .id("env-new")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, w, cx| this.start_new_env(w, cx)))
                            .child(icon("plus", 14., t.text_muted)),
                    ),
            )
            .when(names.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(t.text_placeholder)
                        .child("No environments yet. Click + to create one."),
                )
            })
            .children(names.iter().enumerate().map(|(i, name)| {
                let is_active = active.as_deref() == Some(name.as_str());
                let n1 = name.clone();
                self.tree_row(t, ("env", i).into(), 0, is_active)
                    .gap_2()
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                        this.set_active_env(Some(n1.clone()), cx);
                        if ev.click_count() >= 2 {
                            this.open_env_tab(&n1);
                        }
                        cx.notify();
                    }))
                    .child(div().size(px(7.)).rounded_full().bg(if is_active {
                        t.created
                    } else {
                        t.border
                    }))
                    .child(name.clone())
            }))
            .when_some(active.clone(), |d, name| {
                let (a, b, c, e) = (name.clone(), name.clone(), name.clone(), name.clone());
                d.child(
                    row()
                        .px_2()
                        .pt_2()
                        .gap_1()
                        .child(action("env-open", "Edit").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.open_env_tab(&e);
                                cx.notify();
                            },
                        )))
                        .child(action("env-rename", "Rename").on_click(
                            cx.listener(move |this, _, w, cx| this.start_rename_env(&a, w, cx)),
                        ))
                        .child(action("env-dup", "Duplicate").on_click(
                            cx.listener(move |this, _, _, cx| this.duplicate_env(&b, cx)),
                        ))
                        .child(
                            action("env-delete", "Delete").on_click(
                                cx.listener(move |this, _, _, cx| this.delete_env(&c, cx)),
                            ),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .pt_3()
                        .pb_1()
                        .text_size(px(11.))
                        .text_color(t.text_placeholder)
                        .child(format!("Variables · {name}")),
                )
            })
            .children(rows.into_iter().enumerate().map(|(i, v)| {
                let var = v.name.clone();
                let masked = v.secret && !self.envs.is_revealed(&v.name);
                row()
                    .id(("env-var", i))
                    .px_2()
                    .py(px(2.))
                    .gap_2()
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .when(masked, |d| {
                        d.cursor_pointer().on_click(
                            cx.listener(move |this, _, _, cx| this.reveal_secret(&var, cx)),
                        )
                    })
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .truncate()
                            .text_color(t.text)
                            .child(v.name.clone()),
                    )
                    .child(div().truncate().child(if masked {
                        MASK.to_string()
                    } else {
                        v.value.clone()
                    }))
            }))
            .child(
                row()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(format!("Runtime variables · {}", self.envs.runtime.len())),
                    )
                    .child(
                        action("runtime-clear", "Clear")
                            .on_click(cx.listener(|this, _, _, cx| this.clear_runtime_vars(cx))),
                    ),
            )
            .children(self.envs.runtime.iter().map(|(k, (v, secret))| {
                let masked = *secret && !self.envs.is_revealed(k);
                row()
                    .px_2()
                    .py(px(2.))
                    .gap_2()
                    .font_family(BUFFER_FONT)
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .truncate()
                            .text_color(t.text)
                            .child(k.clone()),
                    )
                    .child(div().truncate().child(if masked {
                        MASK.to_string()
                    } else {
                        v.clone()
                    }))
            }))
            .when(self.envs.runtime.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .text_size(px(12.))
                        .text_color(t.text_placeholder)
                        .child("None. Scripts and captures set these."),
                )
            })
    }

    pub fn open_env_tab(&mut self, name: &str) {
        self.load_env(name);
        self.open_tab(&format!("env/{name}"), name, TabKind::Environment, true);
    }

    // ---- Auth tab ------------------------------------------------------------------------------

    fn auth_map(&self, key: &str) -> (Option<String>, Vec<(String, String)>) {
        let Some(doc) = self.project.as_ref().and_then(|p| p.doc(key)) else {
            return (None, vec![]);
        };
        match doc.parsed.front.get("auth").map(|e| &e.value) {
            Some(FmValue::Scalar(s)) => (Some(s.clone()), vec![]),
            Some(FmValue::Map(m)) => (None, m.clone()),
            _ => (None, vec![]),
        }
    }

    pub fn set_auth_type(&mut self, key: &str, ty: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let edits = match ty {
            "inherit" => fluxwire_format::remove_front_matter(&doc.parsed, "auth"),
            "none" => fluxwire_format::set_front_matter(
                &doc.parsed,
                "auth",
                &FmValue::Scalar("none".into()),
            ),
            other => {
                let mut m = vec![("type".to_string(), other.to_string())];
                if other == "oauth2" {
                    m.push(("grant".into(), "client_credentials".into()));
                }
                fluxwire_format::set_front_matter(&doc.parsed, "auth", &FmValue::Map(m))
            }
        };
        if doc.apply(&edits) {
            self.envs.invalidate();
            self.after_edit_pub(key, cx);
        }
    }

    pub fn set_auth_field(&mut self, key: &str, field: &str, value: &str, cx: &mut Context<Self>) {
        let (_, mut map) = self.auth_map(key);
        match map.iter_mut().find(|(k, _)| k == field) {
            Some((_, v)) => *v = value.to_string(),
            None => map.push((field.to_string(), value.to_string())),
        }
        let Some(doc) = self.project.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let edits = fluxwire_format::set_front_matter(&doc.parsed, "auth", &FmValue::Map(map));
        if doc.apply(&edits) {
            self.envs.invalidate();
            self.after_edit_pub(key, cx);
        }
    }

    /// Fetch (or replace) the OAuth2 token for the active request (Auth tab button).
    pub fn fetch_token(&mut self, cx: &mut Context<Self>) {
        let (Some(tab_id), Some(key)) = (self.tabs.active_id(), self.active_key()) else {
            return;
        };
        if self.envs.fetching.contains(&tab_id) {
            return;
        }
        let Some(parsed) = self
            .project
            .as_ref()
            .and_then(|p| p.doc(&key))
            .map(|d| d.parsed.clone())
        else {
            return;
        };
        let Some(root) = self.collection_root_of(&key) else {
            return;
        };
        let (Some(scope), Some(env)) = (self.scope_for(&key), self.env_state(&root)) else {
            self.notice =
                Some("Select an environment first; tokens are cached per environment.".into());
            cx.notify();
            return;
        };
        let defaults = self
            .project
            .as_ref()
            .and_then(|p| p.collections.iter().find(|c| c.root == root))
            .map(|c| settings_from_defaults(&c.collection.config.defaults))
            .unwrap_or_default();
        let opts = SendOptions {
            scope: (*scope).clone(),
            settings: defaults,
            net: net_config_from_dir(&root),
            env: Some(env),
            ..Default::default()
        };
        let Some((rx, _cancel)) = runner().fetch_token(&parsed, opts, true) else {
            self.notice = Some("This request does not use OAuth2.".into());
            cx.notify();
            return;
        };
        self.envs.fetching.insert(tab_id);
        self.notice = Some("Waiting for the token… finish any login in your browser.".into());
        cx.spawn(async move |this, cx| {
            let r = rx.await;
            this.update(cx, |this, cx| {
                this.envs.fetching.remove(&tab_id);
                this.notice = Some(match r {
                    Ok(Ok(_)) => "Fetched a new token.".into(),
                    Ok(Err(e)) => format!(
                        "Token request failed: {}{}",
                        e.message,
                        e.fix.map(|f| format!(" {f}")).unwrap_or_default()
                    ),
                    Err(_) => "The token request was dropped.".into(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_auth_tab(
        &self,
        t: &Theme,
        key: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (kw, map) = self.auth_map(key);
        let ty = map
            .iter()
            .find(|(k, _)| k == "type")
            .map(|(_, v)| v.clone());
        let current = match (&ty, kw.as_deref()) {
            (Some(t), _) => t.clone(),
            (None, Some("none")) => "none".into(),
            _ => "inherit".into(),
        };
        let get = |f: &str| {
            map.iter()
                .find(|(k, _)| k == f)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let grant = if current == "oauth2" {
            match get("grant").as_str() {
                "" => "client_credentials".to_string(),
                g => g.to_string(),
            }
        } else {
            String::new()
        };
        let chip = |id: (&'static str, usize), label: &str, on: bool| {
            row()
                .id(id)
                .px_2()
                .h(px(24.))
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .border_1()
                .border_color(if on { t.accent } else { t.border })
                .text_color(if on { t.accent } else { t.text_muted })
                .child(label.to_string())
        };
        let mut types: Vec<(&str, &str)> = vec![("Inherit", "inherit"), ("None", "none")];
        types.extend(AUTH_TYPES);
        let mut col = div()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(row().gap_1().children(types.into_iter().enumerate().map(
                |(i, (label, val))| {
                    let k = key.to_string();
                    chip(("auth-type", i), label, current == val).on_click(
                        cx.listener(move |this, _, _, cx| this.set_auth_type(&k, val, cx)),
                    )
                },
            )));
        if current == "inherit" {
            let inherited = self
                .scope_for(key)
                .and_then(|s| s.inherited.auth.clone())
                .map(|(kw, f)| {
                    f.iter()
                        .find(|(k, _)| k == "type")
                        .map(|(_, v)| v.clone())
                        .or(kw)
                        .unwrap_or_else(|| "none".into())
                })
                .unwrap_or_else(|| "none".into());
            col = col.child(div().text_color(t.text_muted).child(format!(
                "Uses the auth of the nearest folder or collection that sets one (currently: {inherited})."
            )));
        }
        if current == "oauth2" {
            let k = key.to_string();
            col = col.child(
                row()
                    .gap_1()
                    .child(
                        div()
                            .w(px(110.))
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child("Grant"),
                    )
                    .children(GRANTS.iter().enumerate().map(|(i, g)| {
                        let k = k.clone();
                        let g: &'static str = g;
                        chip(("auth-grant", i), g, grant == g).on_click(cx.listener(
                            move |this, _, _, cx| this.set_auth_field(&k, "grant", g, cx),
                        ))
                    })),
            );
            let k = key.to_string();
            let pkce = get("pkce") == "true";
            if grant == "authorization_code" {
                col = col.child(
                    row()
                        .gap_1()
                        .child(
                            div()
                                .w(px(110.))
                                .text_size(px(12.))
                                .text_color(t.text_muted)
                                .child("PKCE"),
                        )
                        .child(
                            chip(("auth-pkce", 0), if pkce { "on" } else { "off" }, pkce).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.set_auth_field(
                                        &k,
                                        "pkce",
                                        if pkce { "false" } else { "true" },
                                        cx,
                                    )
                                }),
                            ),
                        ),
                );
            }
        }
        if current == "apikey" {
            let k = key.to_string();
            let q = get("in") == "query";
            col = col.child(
                row()
                    .gap_1()
                    .child(
                        div()
                            .w(px(110.))
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child("Send in"),
                    )
                    .child(
                        chip(("auth-in", 0), if q { "query" } else { "header" }, true).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.set_auth_field(
                                    &k,
                                    "in",
                                    if q { "header" } else { "query" },
                                    cx,
                                )
                            }),
                        ),
                    ),
            );
        }
        for (i, (field, label)) in auth_fields(&current, &grant).into_iter().enumerate() {
            col = col.child(
                row()
                    .gap_2()
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(t.text_muted)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .border_b_1()
                            .border_color(t.border_variant)
                            .child(self.field(
                                t,
                                ("auth-field", i),
                                EditTarget::Auth {
                                    key: key.to_string(),
                                    field: field.to_string(),
                                },
                                get(field),
                                "value or {{variable}}",
                                cx,
                            )),
                    ),
            );
        }
        if current == "oauth2" {
            col = col.child(self.render_token_status(t, key, cx));
        }
        col.into_any_element()
    }

    fn render_token_status(&self, t: &Theme, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let busy = self
            .tabs
            .active_id()
            .is_some_and(|id| self.envs.fetching.contains(&id));
        let info = (|| {
            let doc = self.project.as_ref()?.doc(key)?;
            let root = self.collection_root_of(key)?;
            let env = self.envs.states.get(&(root, self.envs.active.clone()?))?;
            let scope = self.scope_for(key)?;
            runner().token_info(&doc.parsed, &scope, env)
        })();
        let status = match (&info, self.envs.active.as_ref()) {
            (_, None) => "No environment selected; tokens are cached per environment.".to_string(),
            (None, Some(e)) => format!("No token yet for {e}. Send a request or fetch one now."),
            (Some(i), Some(e)) => format!(
                "{} token for {e}, {}{}",
                i.token_type,
                fmt_expiry(i.expires_at),
                if i.has_refresh {
                    ", refreshes automatically"
                } else {
                    ""
                }
            ),
        };
        div()
            .mt_2()
            .p_2()
            .rounded(px(4.))
            .border_1()
            .border_color(t.border_variant)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child(status),
            )
            .child(
                row().gap_2().child(
                    button(
                        t,
                        "fetch-token",
                        if busy {
                            "Waiting…"
                        } else {
                            "Fetch new token"
                        },
                        !busy,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.fetch_token(cx))),
                ),
            )
            .into_any_element()
    }
}
