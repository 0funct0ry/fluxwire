//! The GraphQL sub-tab of a request (M16): query and variables editors with schema completion,
//! the problems list, introspection, and the query builder. State and pure logic are in
//! `graphql_state.rs`; the builder model and schema live in `fluxwire-graphql`.

use fluxwire_graphql::builder::{Builder, BuilderError, input_for, literal_for, starting_literal};
use fluxwire_graphql::parse::OpKind;
use fluxwire_graphql::schema::{Schema, SchemaCache, TypeDef};
use fluxwire_model::BlockKind;
use fluxwire_protocol::{CancellationToken, ExecCtx};
use fluxwire_runner::{net_config_from_dir, runner, settings_from_defaults};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::{
    assets::BUFFER_FONT,
    center::button,
    dev_id::{DevClick, Role},
    editor::Lang,
    graphql_panel::{field_open, field_visible, path_key},
    graphql_state::{problem_lines, schema_key},
    graphql_vars::{self, VarsMode},
    text_input::{TextInput, bound_input, lazy_input},
    theme::Theme,
    ui::row,
    workspace::WorkspaceView,
    workspace_model::{OpenDoc, Target},
};

/// Deepest field path the builder draws, so recursive types cannot expand forever.
const MAX_DEPTH: usize = 6;

/// Label and index of the GraphQL sub-tab (after the eight request tabs).
pub const SUBTAB: usize = 8;

pub fn is_graphql(doc: &OpenDoc) -> bool {
    doc.parsed
        .blocks_of(BlockKind::GraphqlQuery)
        .next()
        .is_some()
}

/// Directory of the introspection cache of a collection.
pub fn cache_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".fluxwire").join("graphql")
}

impl WorkspaceView {
    /// Endpoint (as written) and active environment of `doc`: the schema's identity.
    fn graphql_identity(&self, doc: &OpenDoc) -> (String, String) {
        let url = doc
            .parsed
            .request()
            .map(|r| r.target.clone())
            .unwrap_or_default();
        (url, self.active_env_name().unwrap_or_default())
    }

    /// The schema of `doc`, from memory or, the first time, the collection's disk cache.
    fn graphql_schema(&self, doc: &OpenDoc) -> Option<std::sync::Arc<Schema>> {
        let (url, env) = self.graphql_identity(doc);
        let key = schema_key(&url, &env);
        if self.graphql.probe(&key)
            && let Some(root) = self.collection_root_of(&doc.key)
            && let Some(s) = SchemaCache::new(cache_dir(&root)).load(&url, &env)
        {
            self.graphql.put_cached(&key, s);
        }
        self.graphql.schema(&key)
    }

    /// Completion provider for a `graphql query` block, once a schema is known.
    pub(crate) fn graphql_completion(
        &self,
        key: &str,
    ) -> Option<std::rc::Rc<dyn crate::completion::CompletionProvider>> {
        let doc = self.scope.workspace.as_ref()?.doc(key)?;
        Some(crate::graphql_state::provider(self.graphql_schema(doc)?))
    }

    /// `graphql: introspect`: fetch the schema of the active request and cache it.
    pub fn graphql_introspect(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&key)) else {
            return;
        };
        let parsed = doc.parsed.clone();
        let (url, env) = self.graphql_identity(doc);
        let skey = schema_key(&url, &env);
        let (Some(scope), Some(root)) = (self.scope_for(&key), self.collection_root_of(&key))
        else {
            return;
        };
        let plan = match fluxwire_graphql::introspection_plan(&parsed, &scope) {
            Ok(p) => p,
            Err(d) => {
                self.graphql.fail(
                    &skey,
                    d.first().map(|d| d.message.clone()).unwrap_or_default(),
                );
                cx.notify();
                return;
            }
        };
        if !self.graphql.start(&skey) {
            return;
        }
        let settings = self
            .scope
            .workspace
            .as_ref()
            .and_then(|p| p.collections.iter().find(|c| c.root == root))
            .map(|c| settings_from_defaults(&c.collection.config.defaults))
            .unwrap_or_default();
        let ctx = ExecCtx {
            cancel: CancellationToken::new(),
            settings,
            net: net_config_from_dir(&root),
            temp_dir: None,
            env: self.env_state(&root),
            session: None,
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        runner().handle().spawn(async move {
            let _ = tx.send(fluxwire_graphql::fetch_schema(plan, ctx).await);
        });
        cx.spawn(async move |this, cx| {
            let r = rx.await;
            this.update(cx, |this, cx| {
                match r {
                    Ok(Ok((schema, raw))) => {
                        let _ = SchemaCache::new(cache_dir(&root)).store(&url, &env, &raw);
                        this.graphql.set_schema(skey, schema);
                    }
                    Ok(Err(e)) => this.graphql.fail(&skey, e),
                    Err(_) => this.graphql.fail(&skey, "the request was dropped".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn graphql_toggle_builder(&mut self, cx: &mut Context<Self>) {
        self.graphql.builder_open = !self.graphql.builder_open;
        cx.notify();
    }

    /// Show the GraphQL sub-tab of the active request.
    pub fn graphql_open_tab(&mut self, cx: &mut Context<Self>) {
        self.show_request_tab(SUBTAB);
        cx.notify();
    }

    /// Apply `f` to the builder tree of the active query (or a new one) and write it back.
    fn graphql_edit_builder(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Builder, &Schema),
    ) {
        self.graphql_edit_builder_with(cx, true, f)
    }

    /// Like [`Self::graphql_edit_builder`]; `sync_vars` also adds new variables to the
    /// `json variables` block. Typing an argument value passes `false`, because every
    /// half-typed `$name` would otherwise leave a variable behind; the block is synced when the
    /// input loses focus instead.
    fn graphql_edit_builder_with(
        &mut self,
        cx: &mut Context<Self>,
        sync_vars: bool,
        f: impl FnOnce(&mut Builder, &Schema),
    ) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&key)) else {
            return;
        };
        let Some(schema) = self.graphql_schema(doc) else {
            self.graphql.notice = Some("Introspect the schema first.".into());
            cx.notify();
            return;
        };
        let Some(qi) = doc.find_block(BlockKind::GraphqlQuery, None) else {
            return;
        };
        let text = doc.target_text(Target::Block(qi));
        let mut b = if text.trim().is_empty() {
            Builder::default()
        } else {
            match Builder::from_query(&text) {
                Ok(b) => b,
                Err(BuilderError::Parse(m)) => {
                    self.graphql.notice = Some(format!("Fix the query first: {m}"));
                    cx.notify();
                    return;
                }
                Err(BuilderError::Unsupported(m)) => {
                    self.graphql.notice = Some(format!("The builder cannot edit this query: {m}."));
                    cx.notify();
                    return;
                }
            }
        };
        f(&mut b, &schema);
        let new = b.render(&schema);
        let defs = b.variable_defs(&schema);
        if !b.roots.is_empty() && fluxwire_graphql::parse::parse(&new).is_err() {
            // a half-typed argument value: keep the query as it is until the value parses
            return;
        }
        if let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key))
            && new.trim_end() != text.trim_end()
        {
            doc.set_target_text(Target::Block(qi), new.trim_end());
        }
        if sync_vars {
            self.graphql_sync_variables(&key, &defs);
        }
        self.after_edit(&key, cx);
    }

    /// Add the variables the query declares to the `json variables` block.
    pub(crate) fn graphql_sync_variables_now(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(&key)) else {
            return;
        };
        let Some(schema) = self.graphql_schema(doc) else {
            return;
        };
        let Some(qi) = doc.find_block(BlockKind::GraphqlQuery, None) else {
            return;
        };
        let Ok(b) = Builder::from_query(&doc.target_text(Target::Block(qi))) else {
            return;
        };
        let defs = b.variable_defs(&schema);
        self.graphql_sync_variables(&key, &defs);
        self.after_edit(&key, cx);
    }

    /// Make sure the `json variables` block exists and has a key for every variable the query
    /// declares, with a placeholder value for new ones. Existing values are never touched.
    fn graphql_sync_variables(&mut self, key: &str, defs: &[fluxwire_graphql::builder::Variable]) {
        if defs.is_empty() {
            return;
        }
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let vi = match doc.find_block(BlockKind::GraphqlVariables, None) {
            Some(i) => i,
            None => {
                let edits = fluxwire_format::insert_block(
                    &doc.parsed,
                    BlockKind::GraphqlVariables,
                    "json variables",
                    "",
                );
                if !doc.apply(&edits) {
                    return;
                }
                match doc.find_block(BlockKind::GraphqlVariables, None) {
                    Some(i) => i,
                    None => return,
                }
            }
        };
        let text = doc.target_text(Target::Block(vi));
        let mut obj = if text.trim().is_empty() {
            serde_json::Map::new()
        } else {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(serde_json::Value::Object(m)) => m,
                // not an object (or not JSON yet): leave the user's text alone
                _ => return,
            }
        };
        let mut changed = false;
        for v in defs {
            if !obj.contains_key(&v.name) {
                obj.insert(
                    v.name.clone(),
                    fluxwire_graphql::builder::placeholder_json(&v.ty),
                );
                changed = true;
            }
        }
        if changed && let Ok(out) = serde_json::to_string_pretty(&serde_json::Value::Object(obj)) {
            doc.set_target_text(Target::Block(vi), &out);
        }
    }

    pub(crate) fn graphql_toggle(&mut self, path: Vec<String>, cx: &mut Context<Self>) {
        self.graphql_edit_builder(cx, move |b, s| {
            let p: Vec<&str> = path.iter().map(String::as_str).collect();
            let on = b.is_checked(&p);
            b.set_checked(s, &p, !on);
        });
    }

    /// The `$ variable` button: use `$name` for the argument, or, when it already is `$name`,
    /// go back to a plain value (a starting value for a required argument, none otherwise).
    pub(crate) fn graphql_toggle_arg(
        &mut self,
        path: Vec<String>,
        arg: String,
        cx: &mut Context<Self>,
    ) {
        self.graphql_edit_builder(cx, move |b, s| {
            let p: Vec<&str> = path.iter().map(String::as_str).collect();
            let var = format!("${arg}");
            let value = if b.arg(&p, &arg) == Some(var.as_str()) {
                b.arg_type(s, &p, &arg)
                    .filter(|t| t.is_non_null())
                    .and_then(|t| starting_literal(s, &t))
            } else {
                Some(var)
            };
            b.set_arg(&p, &arg, value);
        });
    }

    fn graphql_set_kind(&mut self, kind: OpKind, cx: &mut Context<Self>) {
        self.graphql_edit_builder(cx, move |b, _| {
            if b.kind != kind {
                *b = Builder::new(kind);
            }
        });
    }

    /// Set the value of an argument from what the user typed. Nothing is written while the
    /// resulting query would not parse, so half-typed values never break the builder.
    pub(crate) fn graphql_set_arg(
        &mut self,
        path: Vec<String>,
        arg: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        self.graphql_edit_builder_with(cx, false, move |b, s| {
            let p: Vec<&str> = path.iter().map(String::as_str).collect();
            let value = match b.arg_type(s, &p, &arg) {
                Some(ty) => literal_for(&ty, &text),
                None => text.trim().to_string(),
            };
            b.set_arg(&p, &arg, Some(value));
        });
    }

    fn graphql_toggle_expand(&mut self, path: &[String], cx: &mut Context<Self>) {
        let k = path_key(path);
        if !self.graphql.expanded.remove(&k) {
            self.graphql.expanded.insert(k);
        }
        cx.notify();
    }

    /// Give a blank `graphql query` block a `query { }` stub, once per document.
    fn graphql_stub(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.graphql.stubbed.insert(key.to_string()) {
            return;
        }
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(key)) else {
            return;
        };
        let Some(qi) = doc.find_block(BlockKind::GraphqlQuery, None) else {
            return;
        };
        if doc.target_text(Target::Block(qi)).trim().is_empty() {
            doc.set_target_text(Target::Block(qi), "query {\n  \n}");
            self.after_edit(key, cx);
        }
    }

    /// The text input of one argument. It shows `shown`, and is reset only when the model value
    /// changed by some other route (a toggle or the text editor), never while the
    /// user's own typing is what changed it.
    #[allow(clippy::too_many_arguments)]
    fn arg_input(
        &self,
        id: String,
        path: Vec<String>,
        arg: String,
        model: &str,
        shown: &str,
        ty: &fluxwire_graphql::schema::TypeRef,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<TextInput> {
        let existing = self.graphql.arg_inputs.borrow().get(&id).cloned();
        let (input, last) = match existing {
            Some(e) => e,
            None => {
                let (p, a) = (path, arg);
                let i = bound_input("value", cx, move |this: &mut Self, text, cx| {
                    this.graphql_set_arg(p.clone(), a.clone(), text, cx)
                });
                i.update(cx, |i, cx| i.set_text(shown, cx));
                cx.subscribe(&i, |this, _, ev: &crate::text_input::TextInputEvent, cx| {
                    if *ev == crate::text_input::TextInputEvent::Blur {
                        this.graphql_sync_variables_now(cx);
                    }
                })
                .detach();
                (i, model.to_string())
            }
        };
        if last != model {
            let typed = literal_for(ty, input.read(cx).text());
            if typed != model {
                input.update(cx, |i, cx| i.set_text(shown, cx));
            }
        }
        self.graphql
            .arg_inputs
            .borrow_mut()
            .insert(id, (input.clone(), model.to_string()));
        input
    }

    #[allow(clippy::too_many_arguments)]
    fn render_builder_fields(
        &self,
        t: &Theme,
        doc_key: &str,
        schema: &Schema,
        ty: &TypeDef,
        b: &Builder,
        path: &mut Vec<String>,
        out: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let filter = self.graphql.filter.clone();
        for f in &ty.fields {
            if !field_visible(schema, f, &filter) {
                continue;
            }
            path.push(f.name.clone());
            let refs: Vec<&str> = path.iter().map(String::as_str).collect();
            let checked = b.is_checked(&refs);
            let depth = path.len() - 1;
            let pkey = path_key(path);
            let composite = schema
                .ty(f.ty.named())
                .filter(|n| n.is_composite())
                .is_some();
            let expanded = self.graphql.expanded.contains(&pkey);
            let open = composite
                && path.len() < MAX_DEPTH
                && field_open(schema, f, &filter, expanded, checked);
            let toggle_path = path.clone();
            let expand_path = path.clone();
            let indent = 8. + 16. * depth as f32;
            let mut line = row()
                .h(px(28.))
                .pl(px(indent))
                .gap_1()
                .text_size(px(13.))
                .child(if composite {
                    div()
                        .id(SharedString::from(format!("gql-x-{pkey}")))
                        .w(px(16.))
                        .cursor_pointer()
                        .text_color(t.text_muted)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.graphql_toggle_expand(&expand_path, cx)
                        }))
                        .child(if open { "▾" } else { "▸" })
                        .into_any_element()
                } else {
                    div().w(px(16.)).into_any_element()
                })
                .child(
                    row()
                        .id(SharedString::from(format!("gql-f-{pkey}")))
                        .gap_2()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.graphql_toggle(toggle_path.clone(), cx)
                        }))
                        .child(
                            div()
                                .text_size(px(16.))
                                .text_color(if checked { t.accent } else { t.text_muted })
                                .child(if checked { "☑" } else { "☐" }),
                        )
                        .child(div().child(f.name.clone()))
                        .child(
                            div()
                                .font_family(BUFFER_FONT)
                                .text_size(px(11.))
                                .text_color(t.text_muted)
                                .child(f.ty.to_string()),
                        ),
                );
            if f.deprecated.is_some() {
                line = line.child(div().text_color(t.text_muted).child("deprecated"));
            }
            out.push(line.into_any_element());
            let has_args = checked && !f.args.is_empty();
            if has_args {
                out.push(Self::section_label(t, "ARGUMENTS · sent", indent + 16.));
                for a in &f.args {
                    let model = b.arg(&refs, &a.name).map(str::to_string);
                    let on_var = model.as_deref() == Some(format!("${}", a.name).as_str());
                    let shown = model
                        .as_deref()
                        .map(|m| input_for(&a.ty, m))
                        .unwrap_or_default();
                    let input = self.arg_input(
                        format!("{doc_key}|{pkey}.{}", a.name),
                        path.clone(),
                        a.name.clone(),
                        model.as_deref().unwrap_or(""),
                        &shown,
                        &a.ty,
                        cx,
                    );
                    let required = a.ty.is_non_null() && a.default.is_none();
                    let ap = path.clone();
                    let name = a.name.clone();
                    out.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .pl(px(indent + 16.))
                            .pr_2()
                            .py(px(3.))
                            .child(
                                row()
                                    .gap_2()
                                    .text_size(px(12.))
                                    .child(div().child(a.name.clone()))
                                    .child(
                                        div()
                                            .font_family(BUFFER_FONT)
                                            .text_size(px(11.))
                                            .text_color(t.text_muted)
                                            .child(a.ty.to_string()),
                                    )
                                    .children(required.then(|| {
                                        div()
                                            .text_size(px(11.))
                                            .text_color(t.deleted)
                                            .child("required")
                                    })),
                            )
                            .child(
                                row()
                                    .gap_2()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "gql-av-{pkey}.{}",
                                                a.name
                                            )))
                                            .flex_1()
                                            .min_w(px(60.))
                                            .h(px(26.))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .rounded(px(4.))
                                            .border_1()
                                            .border_color(t.border)
                                            .child(input),
                                    )
                                    .child(
                                        row()
                                            .id(SharedString::from(format!(
                                                "gql-a-{pkey}.{}",
                                                a.name
                                            )))
                                            .h(px(26.))
                                            .px_2()
                                            .rounded(px(4.))
                                            .border_1()
                                            .border_color(if on_var { t.accent } else { t.border })
                                            .cursor_pointer()
                                            .font_family(BUFFER_FONT)
                                            .text_size(px(11.))
                                            .text_color(if on_var {
                                                t.accent
                                            } else {
                                                t.text_muted
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.graphql_toggle_arg(
                                                    ap.clone(),
                                                    name.clone(),
                                                    cx,
                                                )
                                            }))
                                            .child("$ variable"),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
            }
            if open && let Some(next) = schema.ty(f.ty.named()) {
                if has_args {
                    out.push(Self::section_label(t, "FIELDS · returned", indent + 16.));
                }
                self.render_builder_fields(t, doc_key, schema, next, b, path, out, cx);
            }
            path.pop();
        }
    }

    fn section_label(t: &Theme, text: &'static str, indent: f32) -> AnyElement {
        div()
            .pl(px(indent))
            .pt(px(6.))
            .pb(px(2.))
            .text_size(px(10.))
            .text_color(t.text_muted)
            .child(text)
            .into_any_element()
    }

    fn side_panel_shell(t: &Theme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .w(px(320.))
            .flex_none()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .border_l_1()
            .border_color(t.border)
    }

    fn render_builder(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        schema: Option<&Schema>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut col = Self::side_panel_shell(t).child(
            div()
                .px_3()
                .py_1()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child("Query builder"),
        );
        let note = |t: &Theme, text: String| {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child(text)
        };
        let Some(schema) = schema else {
            return col
                .child(note(t, "Introspect the schema to build a query.".into()))
                .into_any_element();
        };
        let text = doc
            .find_block(BlockKind::GraphqlQuery, None)
            .map(|i| doc.target_text(Target::Block(i)))
            .unwrap_or_default();
        let b = if text.trim().is_empty() {
            Ok(Builder::default())
        } else {
            Builder::from_query(&text)
        };
        let b = match b {
            Ok(b) => b,
            Err(e) => {
                let why = match e {
                    BuilderError::Parse(m) => format!("The query does not parse yet: {m}"),
                    BuilderError::Unsupported(m) => format!("Not shown in the builder: {m}."),
                };
                return col.child(note(t, why)).into_any_element();
            }
        };
        let kind_btn =
            |id: &'static str, label: &'static str, k: OpKind, on: bool, cx: &mut Context<Self>| {
                button(t, id, label, on).dev_click(
                    id,
                    Role::Button,
                    id,
                    true,
                    cx.listener(move |this, _, _, cx| this.graphql_set_kind(k, cx)),
                )
            };
        let filter = lazy_input(
            &self.graphql.filter_input,
            "Filter fields…",
            cx,
            |this: &mut Self, text, cx| {
                this.graphql.filter = text;
                cx.notify();
            },
        );
        col = col
            .child(
                row()
                    .px_3()
                    .pb_1()
                    .gap_2()
                    .child(kind_btn(
                        "gql-kind-query",
                        "Query",
                        OpKind::Query,
                        b.kind == OpKind::Query,
                        cx,
                    ))
                    .child(kind_btn(
                        "gql-kind-mutation",
                        "Mutation",
                        OpKind::Mutation,
                        b.kind == OpKind::Mutation,
                        cx,
                    )),
            )
            .child(
                row()
                    .id("gql-filter")
                    .px_3()
                    .pb_1()
                    .flex_none()
                    .child(div().flex_1().child(filter)),
            );
        let mut fields = Vec::new();
        if let Some(root) = schema.root(b.kind) {
            self.render_builder_fields(
                t,
                &doc.key,
                schema,
                root,
                &b,
                &mut Vec::new(),
                &mut fields,
                cx,
            );
            if fields.is_empty() {
                fields.push(note(t, "No field matches the filter.".into()).into_any_element());
            }
        } else {
            fields
                .push(note(t, "This schema has no such operation type.".into()).into_any_element());
        }
        col.child(
            div()
                .id("gql-builder-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(fields),
        )
        .into_any_element()
    }

    /// Edit the text of the `json variables` block with `f` (see `graphql_vars`).
    pub(crate) fn graphql_vars_edit(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&str) -> Option<String>,
    ) {
        let Some(key) = self.active_key() else { return };
        let Some(doc) = self.scope.workspace.as_mut().and_then(|p| p.doc_mut(&key)) else {
            return;
        };
        let Some(vi) = doc.find_block(BlockKind::GraphqlVariables, None) else {
            return;
        };
        if let Some(new) = f(&doc.target_text(Target::Block(vi))) {
            doc.set_target_text(Target::Block(vi), &new);
            self.after_edit(&key, cx);
        }
    }

    /// A text input of the variables table. It shows `model` and is reset only when the model
    /// changed by some other route, never while the user's own typing is what changed it.
    fn var_input(
        &self,
        id: String,
        model: &str,
        on_change: impl Fn(&mut Self, String, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<TextInput> {
        let existing = self.graphql.arg_inputs.borrow().get(&id).cloned();
        let (input, last) = match existing {
            Some(e) => e,
            None => {
                let i = bound_input("", cx, on_change);
                i.update(cx, |i, cx| i.set_text(model, cx));
                (i, model.to_string())
            }
        };
        if last != model && input.read(cx).text() != model {
            input.update(cx, |i, cx| i.set_text(model, cx));
        }
        self.graphql
            .arg_inputs
            .borrow_mut()
            .insert(id, (input.clone(), model.to_string()));
        input
    }

    fn render_vars_table(
        &self,
        t: &Theme,
        doc_key: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let note = |t: &Theme, msg: String| {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child(msg)
                .into_any_element()
        };
        let rows = match graphql_vars::rows(text) {
            Ok(r) => r,
            Err(why) => return note(t, format!("{why}. Switch to JSON to fix it.")),
        };
        let boxed = |t: &Theme, input: gpui::Entity<TextInput>| {
            div()
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(t.border)
                .child(input)
        };
        let mut list: Vec<AnyElement> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            let ki = self.var_input(
                format!("{doc_key}|vars|{i}|k"),
                &r.key,
                move |this: &mut Self, text, cx| {
                    this.graphql_vars_edit(cx, |t| graphql_vars::rename(t, i, &text))
                },
                cx,
            );
            let vi = self.var_input(
                format!("{doc_key}|vars|{i}|v"),
                &r.value,
                move |this: &mut Self, text, cx| {
                    this.graphql_vars_edit(cx, |t| graphql_vars::set_value(t, i, &text))
                },
                cx,
            );
            list.push(
                row()
                    .px_3()
                    .py(px(2.))
                    .gap_2()
                    .child(
                        boxed(t, ki)
                            .id(SharedString::from(format!("gql-vk-{i}")))
                            .w(px(140.))
                            .flex_none(),
                    )
                    .child(
                        boxed(t, vi)
                            .id(SharedString::from(format!("gql-vv-{i}")))
                            .flex_1()
                            .min_w(px(60.)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("gql-vx-{i}")))
                            .w(px(22.))
                            .flex_none()
                            .cursor_pointer()
                            .text_color(t.text_muted)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.graphql_vars_edit(cx, |t| graphql_vars::remove(t, i))
                            }))
                            .child("✕"),
                    )
                    .into_any_element(),
            );
        }
        if list.is_empty() {
            list.push(note(t, "No variables yet.".into()));
        }
        list.push(
            row()
                .px_3()
                .py_1()
                .child(
                    button(t, "gql-vars-add", "+ Add variable", false).dev_click(
                        "gql-vars-add",
                        Role::Button,
                        "gql-vars-add",
                        true,
                        cx.listener(|this, _, _, cx| this.graphql_vars_edit(cx, graphql_vars::add)),
                    ),
                )
                .into_any_element(),
        );
        div()
            .id("gql-vars-table")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(list)
            .into_any_element()
    }

    /// The variables pane: a drag handle, a Table / JSON switch and the chosen view.
    fn render_vars(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        vi: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = doc.target_text(Target::Block(vi));
        let mode = self.graphql.vars_mode;
        let pick = |id: &'static str, label: &'static str, m: VarsMode, cx: &mut Context<Self>| {
            button(t, id, label, mode == m).dev_click(
                id,
                Role::Button,
                id,
                true,
                cx.listener(move |this, _, _, cx| {
                    this.graphql.vars_mode = m;
                    cx.notify();
                }),
            )
        };
        let body = match mode {
            VarsMode::Json => self.editor_surface(
                "gql-vars",
                &doc.key,
                &text,
                Lang::Json,
                Target::Block(vi),
                cx,
            ),
            VarsMode::Table => self.render_vars_table(t, &doc.key, &text, cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .id("gql-vars-handle")
                    .h(px(5.))
                    .w_full()
                    .flex_none()
                    .cursor_row_resize()
                    .border_t_1()
                    .border_color(t.border)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, ev: &gpui::MouseDownEvent, _, cx| {
                            this.graphql.vars_drag =
                                Some((f32::from(ev.position.y), this.graphql.vars_height()));
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .h(px(self.graphql.vars_height()))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        row()
                            .px_3()
                            .pb_1()
                            .gap_2()
                            .flex_none()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.text_muted)
                                    .child("Variables"),
                            )
                            .child(
                                row()
                                    .ml_auto()
                                    .gap_1()
                                    .child(pick(
                                        "gql-vars-table-mode",
                                        "Table",
                                        VarsMode::Table,
                                        cx,
                                    ))
                                    .child(pick("gql-vars-json-mode", "JSON", VarsMode::Json, cx)),
                            ),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    pub(crate) fn render_graphql_tab(
        &self,
        t: &Theme,
        doc: &OpenDoc,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = doc.key.clone();
        let schema = self.graphql_schema(doc);
        let (url, env) = self.graphql_identity(doc);
        let skey = schema_key(&url, &env);
        let loading = self.graphql.is_loading(&skey);
        let qi = doc.find_block(BlockKind::GraphqlQuery, None);
        let builder_open = self.graphql.builder_open;

        let toolbar = row()
            .px_3()
            .py_1()
            .gap_2()
            .child(
                button(
                    t,
                    "gql-introspect",
                    if loading { "Fetching…" } else { "Introspect" },
                    true,
                )
                .dev_click(
                    "gql-introspect",
                    Role::Button,
                    "gql-introspect",
                    !loading,
                    cx.listener(|this, _, _, cx| this.graphql_introspect(cx)),
                ),
            )
            .child(
                button(
                    t,
                    "gql-builder",
                    if builder_open {
                        "Hide builder"
                    } else {
                        "Builder"
                    },
                    false,
                )
                .dev_click(
                    "gql-builder",
                    Role::Button,
                    "gql-builder",
                    true,
                    cx.listener(|this, _, _, cx| this.graphql_toggle_builder(cx)),
                ),
            )
            .child(div().text_size(px(12.)).text_color(t.text_muted).child(
                match (&self.graphql.notice, &schema) {
                    (Some(n), _) => n.clone(),
                    (None, Some(s)) => format!("Schema: {} types.", s.visible_types().count()),
                    (None, None) => "No schema yet.".to_string(),
                },
            ));

        let mut left = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(div().flex_none().child(toolbar));
        if let Some(qi) = qi {
            let qtext = doc.target_text(Target::Block(qi));
            if qtext.trim().is_empty() && !self.graphql.stubbed.contains(&key) {
                let k = key.clone();
                cx.spawn(async move |this, cx| {
                    this.update(cx, |this, cx| this.graphql_stub(&k, cx)).ok();
                })
                .detach();
            }
            left = left.child(
                div()
                    .flex_1()
                    .min_h(px(80.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(self.editor_surface(
                        "gql-query",
                        &key,
                        &qtext,
                        Lang::Graphql,
                        Target::Block(qi),
                        cx,
                    )),
            );
            let vars = doc.find_block(BlockKind::GraphqlVariables, None);
            left = left.child(match vars {
                Some(vi) => self.render_vars(t, doc, vi, cx),
                None => row()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(t.text_muted)
                    .child("No `json variables` block.")
                    .child(button(t, "gql-add-vars", "Add variables", false).dev_click(
                        "gql-add-vars",
                        Role::Button,
                        "gql-add-vars",
                        true,
                        cx.listener(|this, _, _, cx| {
                            this.add_block(BlockKind::GraphqlVariables, "json variables", cx)
                        }),
                    ))
                    .into_any_element(),
            });
            if let Some(s) = &schema {
                let lines = problem_lines(&qtext, s);
                if !lines.is_empty() {
                    let mut list = div()
                        .id("gql-problems")
                        .flex_none()
                        .max_h(px(80.))
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(t.border);
                    for (n, l) in lines.into_iter().enumerate() {
                        let k = key.clone();
                        let offset = l.offset;
                        list = list.child(
                            div()
                                .id(SharedString::from(format!("gql-problem-{n}")))
                                .px_3()
                                .py(px(2.))
                                .text_size(px(12.))
                                .font_family(BUFFER_FONT)
                                .cursor_pointer()
                                .text_color(if l.error { t.deleted } else { t.text_muted })
                                .on_click(cx.listener(move |this, _, w: &mut Window, cx| {
                                    this.focus_code_editor(
                                        &k,
                                        Target::Block(qi),
                                        Some(offset),
                                        w,
                                        cx,
                                    )
                                }))
                                .child(l.text),
                        );
                    }
                    left = left.child(list);
                }
            }
        }
        // Errors of the last response, each linked to the place in the query it points at.
        let errors = self
            .scope
            .tabs
            .active_id()
            .and_then(|id| self.scope.responses.get(&id))
            .and_then(|r| r.view.clone())
            .and_then(|v| v.gql.clone())
            .map(|g| g.list)
            .unwrap_or_default();
        if let (Some(qi), false) = (qi, errors.is_empty()) {
            let qtext = doc.target_text(Target::Block(qi));
            let mut list = div()
                .id("gql-response-errors")
                .flex_none()
                .max_h(px(80.))
                .overflow_y_scroll()
                .border_t_1()
                .border_color(t.border)
                .child(
                    div()
                        .px_3()
                        .py(px(2.))
                        .text_size(px(11.))
                        .text_color(t.text_muted)
                        .child("Errors in the last response"),
                );
            for (n, e) in errors.into_iter().enumerate() {
                let k = key.clone();
                let offset = e
                    .line_col
                    .map(|(l, c)| fluxwire_graphql::parse::offset_of(&qtext, l, c));
                let at = e
                    .line_col
                    .map(|(l, c)| format!("{l}:{c}  "))
                    .unwrap_or_default();
                let mut r = div()
                    .id(SharedString::from(format!("gql-error-{n}")))
                    .px_3()
                    .py(px(2.))
                    .text_size(px(12.))
                    .font_family(BUFFER_FONT)
                    .text_color(t.deleted)
                    .child(format!("{at}{}", e.message));
                if let Some(o) = offset {
                    r = r.cursor_pointer().on_click(cx.listener(
                        move |this, _, w: &mut Window, cx| {
                            this.focus_code_editor(&k, Target::Block(qi), Some(o), w, cx)
                        },
                    ));
                }
                list = list.child(r);
            }
            left = left.child(list);
        }
        let mut root = div()
            .flex()
            .flex_row()
            .w_full()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .child(left);
        if builder_open {
            root = root.child(self.render_builder(t, doc, schema.as_deref(), cx));
        }
        root.into_any_element()
    }
}
