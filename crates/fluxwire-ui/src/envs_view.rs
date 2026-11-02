//! Views for environments, secrets, cookies and the Auth tab (SPEC §8, §9.3, §9.4, §17.3).

use crate::dev_id::{DevClick, Role};
use fluxwire_model::FmValue;
use fluxwire_protocol::{Cookie, now_unix};
use fluxwire_runner::{SendOptions, net_config_from_dir, runner, settings_from_defaults};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    center::button,
    envs::EditTarget,
    theme::Theme,
    ui::{icon, row},
    workspace::{MenuKind, WorkspaceView},
};

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

impl WorkspaceView {
    fn edit_text_for(&self, target: &EditTarget, cx: &gpui::App) -> Option<String> {
        self.scope
            .envs
            .edit
            .as_ref()
            .filter(|e| &e.target == target)
            .map(|e| e.text.read(cx).text().to_string())
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
        let editing = self.edit_text_for(&target, cx);
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
            Some(_) => base
                .on_key_down(cx.listener(|this, ev, w, cx| this.on_env_key(ev, w, cx)))
                .children(self.scope.envs.edit.as_ref().map(|e| e.text.clone()))
                .into_any_element(),
            None => {
                let start = shown.clone();
                base.dev_click(
                    "env-field",
                    Role::Input,
                    "Edit field",
                    true,
                    cx.listener(move |this, _, w, cx| {
                        this.start_field_edit(target.clone(), start.clone(), w, cx)
                    }),
                )
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
        let c = self.scope.envs.completion.as_ref()?;
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
                        .dev_click("var-completion", Role::Button, "var-completion", true, {
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

    pub(crate) fn render_cookies_section(
        &self,
        t: &Theme,
        env: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let jar: Vec<Cookie> = self
            .env_root()
            .and_then(|r| self.scope.envs.states.get(&(r, env.to_string())))
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
                            .dev_click("cookies-clear", Role::Button, "cookies-clear", true, cx.listener(move |this, _, _, cx| this.clear_cookies(&en, cx))),
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
                            .dev_click("envs_view.render_cookies_section", Role::Button, "envs_view.render_cookies_section", true, cx.listener(move |this, _, _, cx| this.remove_cookie(&dom, &path, &name, cx)))
                            .child(icon("x", 12., t.text_placeholder)),
                    )
            }))
            .into_any_element()
    }

    pub fn clear_cookies(&mut self, env: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root()
            && let Some(s) = self.scope.envs.states.get(&(root.clone(), env.to_string()))
        {
            s.cookies.clear();
            self.persist_cookies(&root);
        }
        cx.notify();
    }

    pub fn remove_cookie(&mut self, domain: &str, path: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(root) = self.env_root()
            && let Some(env) = self.active_env_name()
            && let Some(s) = self.scope.envs.states.get(&(root.clone(), env))
        {
            s.cookies.remove(domain, path, name);
            self.persist_cookies(&root);
        }
        cx.notify();
    }

    // ---- Environments panel ------------------------------------------------------------------

    /// The workspace's environment rows for the panel.
    fn env_section_rows(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(root) = self.env_root() else {
            return Vec::new();
        };
        let names = fluxwire_format::list_environments(&root);
        let active = self.active_env_name();
        if names.is_empty() {
            return vec![
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(t.text_placeholder)
                    .child("No environments in this workspace.")
                    .child(
                        div()
                            .id("env-empty-new")
                            .cursor_pointer()
                            .text_color(t.accent)
                            .child("New environment…")
                            .dev_click(
                                "env-empty-new",
                                Role::Button,
                                "env-empty-new",
                                true,
                                cx.listener(|this, _, w, cx| this.start_new_env(w, cx)),
                            ),
                    )
                    .into_any_element(),
            ];
        }
        names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let is_active = active.as_deref() == Some(name.as_str());
                let key = (root.clone(), name.clone());
                let dirty = self.env_doc_dirty(&key);
                let count = self.env_var_count(&root, &name);
                let tracked = self
                    .scope
                    .envs
                    .tracked
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| fluxwire_format::local_env_is_tracked(&root, &name));
                let (r1, n1, n2) = (root.clone(), name.clone(), name.clone());
                let id: gpui::ElementId = ("env", i).into();
                let group = format!("env-row-{i}");
                let dots = self.row_dots(t, &group, &name, MenuKind::Environment, i, cx);
                let menu_name = name.clone();
                self.tree_row(t, id, 0, false)
                    .group(group)
                    .gap_2()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.open_tree_menu(&menu_name, MenuKind::Environment, ev.position, cx)
                        }),
                    )
                    .dev_click(
                        "envs_view.env_section_rows",
                        Role::Button,
                        "envs_view.env_section_rows",
                        true,
                        cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                            // Opening never activates; double-click pins the tab.
                            this.open_env_doc(&r1, &n1, ev.click_count() >= 2);
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .id(("env-dot", i))
                            .size(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .dev_click(
                                "envs_view.env_section_rows",
                                Role::Button,
                                "envs_view.env_section_rows",
                                true,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    let v = (this.active_env_name().as_deref()
                                        != Some(n2.as_str()))
                                    .then(|| n2.clone());
                                    this.set_active_env(v, cx);
                                }),
                            )
                            .child(
                                div()
                                    .size(px(8.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if is_active { t.created } else { t.border })
                                    .when(is_active, |d| d.bg(t.created)),
                            ),
                    )
                    .child(div().flex_1().truncate().child(name.clone()))
                    .when(dirty, |d| {
                        d.child(div().size(px(8.)).rounded_full().bg(t.modified))
                    })
                    .when(tracked, |d| d.child(icon("triangle-alert", 12., t.deleted)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(count.to_string()),
                    )
                    .child(dots)
                    .into_any_element()
            })
            .collect()
    }

    /// Number of variables of an environment (open document or files).
    fn env_var_count(&self, root: &std::path::Path, name: &str) -> usize {
        match self
            .scope
            .envs
            .docs
            .get(&(root.to_path_buf(), name.to_string()))
        {
            Some(d) => d.env.rows.len(),
            None => fluxwire_format::load_environment(root, name, &[])
                .rows
                .len(),
        }
    }

    pub(crate) fn render_envs(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = self.env_section_rows(t, cx);
        div()
            .flex()
            .flex_col()
            .child(
                row().px_2().h(px(24.)).justify_end().child(
                    div()
                        .id("env-new")
                        .cursor_pointer()
                        .tooltip(|_, cx| {
                            cx.new(|_| crate::ui::VarTip {
                                text: "New environment…".into(),
                            })
                            .into()
                        })
                        .dev_click(
                            "env-new",
                            Role::Button,
                            "env-new",
                            true,
                            cx.listener(|this, _, w, cx| this.start_new_env(w, cx)),
                        )
                        .child(icon("plus", 14., t.text_muted)),
                ),
            )
            .children(sections)
    }

    // ---- Auth tab ------------------------------------------------------------------------------

    fn auth_map(&self, key: &str) -> (Option<String>, Vec<(String, String)>) {
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return (None, vec![]);
        };
        match doc.parsed.front.get("auth").map(|e| &e.value) {
            Some(FmValue::Scalar(s)) => (Some(s.clone()), vec![]),
            Some(FmValue::Map(m)) => (None, m.clone()),
            _ => (None, vec![]),
        }
    }

    pub fn set_auth_type(&mut self, key: &str, ty: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
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
            self.scope.envs.invalidate();
            self.after_edit_pub(key, cx);
        }
    }

    pub fn set_auth_field(&mut self, key: &str, field: &str, value: &str, cx: &mut Context<Self>) {
        let (_, mut map) = self.auth_map(key);
        match map.iter_mut().find(|(k, _)| k == field) {
            Some((_, v)) => *v = value.to_string(),
            None => map.push((field.to_string(), value.to_string())),
        }
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let edits = fluxwire_format::set_front_matter(&doc.parsed, "auth", &FmValue::Map(map));
        if doc.apply(&edits) {
            self.scope.envs.invalidate();
            self.after_edit_pub(key, cx);
        }
    }

    /// Fetch (or replace) the OAuth2 token for the active request (Auth tab button).
    pub fn fetch_token(&mut self, cx: &mut Context<Self>) {
        let (Some(tab_id), Some(key)) = (self.scope.tabs.active_id(), self.active_key()) else {
            return;
        };
        if self.scope.envs.fetching.contains(&tab_id) {
            return;
        }
        let Some(parsed) = self
            .scope
            .workspace
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
            .scope
            .workspace
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
        self.scope.envs.fetching.insert(tab_id);
        self.notice = Some("Waiting for the token… finish any login in your browser.".into());
        cx.spawn(async move |this, cx| {
            let r = rx.await;
            this.update(cx, |this, cx| {
                this.scope.envs.fetching.remove(&tab_id);
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
                    chip(("auth-type", i), label, current == val).dev_click(
                        "auth-type",
                        Role::Button,
                        label,
                        true,
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
                        chip(("auth-grant", i), g, grant == g).dev_click(
                            "envs_view.render_auth_tab",
                            Role::Button,
                            "envs_view.render_auth_tab",
                            true,
                            cx.listener(move |this, _, _, cx| {
                                this.set_auth_field(&k, "grant", g, cx)
                            }),
                        )
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
                            chip(("auth-pkce", 0), if pkce { "on" } else { "off" }, pkce)
                                .dev_click(
                                    "envs_view.render_auth_tab",
                                    Role::Button,
                                    "envs_view.render_auth_tab",
                                    true,
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
                        chip(("auth-in", 0), if q { "query" } else { "header" }, true).dev_click(
                            "envs_view.render_auth_tab",
                            Role::Button,
                            "envs_view.render_auth_tab",
                            true,
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
            .scope
            .tabs
            .active_id()
            .is_some_and(|id| self.scope.envs.fetching.contains(&id));
        let info = (|| {
            let doc = self.scope.workspace.as_ref()?.doc(key)?;
            let root = self.collection_root_of(key)?;
            let env = self
                .scope
                .envs
                .states
                .get(&(root, self.active_env_name()?))?;
            let scope = self.scope_for(key)?;
            runner().token_info(&doc.parsed, &scope, env)
        })();
        let status = match (&info, self.active_env_name().as_ref()) {
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
                    .dev_click(
                        "envs_view.render_token_status",
                        Role::Button,
                        "envs_view.render_token_status",
                        true,
                        cx.listener(|this, _, _, cx| this.fetch_token(cx)),
                    ),
                ),
            )
            .into_any_element()
    }
}
