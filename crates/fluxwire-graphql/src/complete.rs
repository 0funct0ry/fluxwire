//! Schema-aware completion for the `graphql query` editor. It reads the text before the cursor
//! with a forgiving token scan (queries are usually half-written), so it never needs a full parse.

use crate::schema::{Kind, Schema, TypeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    Field,
    Argument,
    Value,
    Variable,
    Type,
    Keyword,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    /// Shown beside the label: a field's type, an argument's type.
    pub detail: String,
    pub kind: SuggestionKind,
    /// Text to insert in place of the typed prefix.
    pub insert: String,
    pub deprecated: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum T {
    Name(String),
    P(char),
    Spread,
    Other,
}

fn tokens(src: &str) -> Vec<T> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                out.push(T::Other);
            }
            b'.' if src[i..].starts_with("...") => {
                i += 3;
                out.push(T::Spread);
            }
            c if c == b'_' || c.is_ascii_alphabetic() => {
                let s = i;
                while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                out.push(T::Name(src[s..i].to_string()));
            }
            c if c.is_ascii_digit() || c == b'-' => {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                    i += 1;
                }
                out.push(T::Other);
            }
            c if b"{}()[]:!$=@|&".contains(&c) => {
                out.push(T::P(c as char));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

#[derive(Debug)]
enum Ctx {
    /// A selection set on this (possibly unknown) type.
    Sel(Option<String>),
    /// The argument list of this field of `parent`.
    Args {
        parent: Option<String>,
        field: String,
        value_of: Option<String>,
    },
    /// `query Name(` variable definitions.
    Vars,
    /// A list or object literal inside arguments.
    Lit,
}

/// The word being typed at the end of `before` and its start offset.
fn current_word(before: &str) -> (&str, usize) {
    let start = before
        .rfind(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .map_or(0, |i| i + 1);
    (&before[start..], start)
}

/// Variable names defined in the whole document (`$name:`), for `$` completion.
fn defined_variables(src: &str) -> Vec<String> {
    let t = tokens(src);
    let mut out = Vec::new();
    let mut depth_in_vars = false;
    for w in t.windows(3) {
        if let [T::P('$'), T::Name(n), T::P(':')] = w {
            depth_in_vars = true;
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
    }
    let _ = depth_in_vars;
    out
}

pub fn complete(src: &str, offset: usize, schema: &Schema) -> Vec<Suggestion> {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let (word, wstart) = current_word(before);
    let toks = tokens(&before[..wstart]);

    let mut stack: Vec<Ctx> = Vec::new();
    let mut op_kind = crate::parse::OpKind::Query;
    let mut last_field: Option<(Option<String>, String)> = None; // (parent type, field)
    let mut inline_type: Option<String> = None;
    let mut prev: Option<&T> = None;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match t {
            T::Name(n) => match stack.last_mut() {
                None => match n.as_str() {
                    "query" => op_kind = crate::parse::OpKind::Query,
                    "mutation" => op_kind = crate::parse::OpKind::Mutation,
                    "subscription" => op_kind = crate::parse::OpKind::Subscription,
                    _ => {}
                },
                Some(Ctx::Sel(ty)) => {
                    if prev == Some(&T::Spread) && n == "on" {
                        if let Some(T::Name(c)) = toks.get(i + 1) {
                            inline_type = Some(c.clone());
                            i += 1;
                        }
                    } else if prev == Some(&T::P(':')) || prev == Some(&T::P('@')) {
                        // alias target keeps the alias' slot; directive names are ignored
                        if prev == Some(&T::P(':')) {
                            last_field = Some((ty.clone(), n.clone()));
                        }
                    } else if toks.get(i + 1) != Some(&T::P(':')) {
                        last_field = Some((ty.clone(), n.clone()));
                    }
                }
                Some(Ctx::Args { value_of, .. }) => {
                    if toks.get(i + 1) == Some(&T::P(':')) {
                        *value_of = Some(n.clone());
                    }
                }
                _ => {}
            },
            T::P('{') => match stack.last() {
                None => {
                    let root = schema.root(op_kind).map(|t| t.name.clone());
                    stack.push(Ctx::Sel(root));
                }
                Some(Ctx::Sel(parent)) => {
                    let ty = inline_type.take().or_else(|| {
                        let (_, f) = last_field.as_ref()?;
                        let p = schema.ty(parent.as_deref()?)?;
                        Some(p.field(f)?.ty.named().to_string())
                    });
                    stack.push(Ctx::Sel(ty));
                    last_field = None;
                }
                Some(_) => stack.push(Ctx::Lit),
            },
            T::P('}') => {
                stack.pop();
                last_field = None;
            }
            T::P('(') => match stack.last() {
                None => stack.push(Ctx::Vars),
                Some(Ctx::Sel(_)) => {
                    let (parent, field) = last_field.clone().unwrap_or((None, String::new()));
                    stack.push(Ctx::Args {
                        parent,
                        field,
                        value_of: None,
                    });
                }
                Some(_) => stack.push(Ctx::Lit),
            },
            T::P(')') => {
                stack.pop();
            }
            T::P('[') => stack.push(Ctx::Lit),
            T::P(']') => {
                stack.pop();
            }
            _ => {}
        }
        prev = Some(&toks[i]);
        i += 1;
    }

    // `$` right before the word: variable reference.
    let dollar = before[..wstart].ends_with('$');
    let after_colon = matches!(prev, Some(T::P(':')));
    let mut out = Vec::new();
    let mut push = |label: &str, detail: String, kind, deprecated| {
        if label
            .to_ascii_lowercase()
            .starts_with(&word.to_ascii_lowercase())
        {
            out.push(Suggestion {
                label: label.to_string(),
                detail,
                kind,
                insert: label.to_string(),
                deprecated,
            });
        }
    };

    if dollar {
        for v in defined_variables(src) {
            push(&v, String::new(), SuggestionKind::Variable, false);
        }
        return out;
    }

    match stack.last() {
        None => {
            for k in ["query", "mutation", "subscription", "fragment"] {
                push(k, String::new(), SuggestionKind::Keyword, false);
            }
        }
        Some(Ctx::Sel(ty)) => {
            if matches!(prev, Some(T::Spread)) {
                push("on", String::new(), SuggestionKind::Keyword, false);
            } else if matches!(prev, Some(T::Name(n)) if n == "on")
                && toks.len() >= 2
                && toks[toks.len() - 2] == T::Spread
            {
                for t in schema.visible_types().filter(|t| t.is_composite()) {
                    push(
                        &t.name,
                        format!("{:?}", t.kind).to_lowercase(),
                        SuggestionKind::Type,
                        false,
                    );
                }
            } else if let Some(def) = ty.as_deref().and_then(|n| schema.ty(n)) {
                for f in &def.fields {
                    push(
                        &f.name,
                        f.ty.to_string(),
                        SuggestionKind::Field,
                        f.deprecated.is_some(),
                    );
                }
                if def.is_composite() {
                    push("__typename", "String!".into(), SuggestionKind::Field, false);
                }
                if matches!(def.kind, Kind::Interface | Kind::Union) {
                    push(
                        "...",
                        "inline fragment".into(),
                        SuggestionKind::Keyword,
                        false,
                    );
                }
            }
        }
        Some(Ctx::Args {
            parent,
            field,
            value_of,
        }) => {
            let def = parent
                .as_deref()
                .and_then(|p| schema.ty(p))
                .and_then(|p| p.field(field));
            if let (Some(def), Some(name)) = (def, value_of.as_deref().filter(|_| after_colon)) {
                if let Some(arg) = def.args.iter().find(|a| a.name == name) {
                    value_suggestions(schema, &arg.ty, &mut push);
                }
            } else if let Some(def) = def {
                for a in &def.args {
                    push(&a.name, a.ty.to_string(), SuggestionKind::Argument, false);
                }
            }
        }
        Some(Ctx::Vars) => {
            if after_colon {
                for t in schema
                    .visible_types()
                    .filter(|t| matches!(t.kind, Kind::Scalar | Kind::Enum | Kind::InputObject))
                {
                    push(
                        &t.name,
                        format!("{:?}", t.kind).to_lowercase(),
                        SuggestionKind::Type,
                        false,
                    );
                }
            }
        }
        Some(Ctx::Lit) => {}
    }
    out
}

fn value_suggestions(
    schema: &Schema,
    ty: &TypeRef,
    push: &mut impl FnMut(&str, String, SuggestionKind, bool),
) {
    let name = ty.named();
    match schema.ty(name) {
        Some(t) if t.kind == Kind::Enum => {
            for v in &t.enum_values {
                push(v, name.to_string(), SuggestionKind::Value, false);
            }
        }
        _ if name == "Boolean" => {
            push("true", "Boolean".into(), SuggestionKind::Value, false);
            push("false", "Boolean".into(), SuggestionKind::Value, false);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::sample_schema;

    /// `|` marks the cursor.
    fn at(text: &str) -> Vec<String> {
        let off = text.find('|').unwrap();
        let src = text.replacen('|', "", 1);
        complete(&src, off, &sample_schema())
            .into_iter()
            .map(|s| s.label)
            .collect()
    }

    #[test]
    fn root_fields_and_prefix_filter() {
        assert_eq!(at("{ | }"), ["payment", "payments", "__typename"]);
        assert_eq!(at("{ pay| }"), ["payment", "payments"]);
        assert_eq!(at("{ payments| }"), ["payments"]);
    }

    #[test]
    fn nested_selection_follows_field_types() {
        assert_eq!(
            at("{ payment(id: 1) { | } }"),
            ["id", "status", "amount", "customer", "__typename"]
        );
        assert_eq!(at("{ payment(id: 1) { customer { n| } } }"), ["name"]);
        assert_eq!(
            at("{ payments { customer { | } } }"),
            ["name", "email", "__typename"]
        );
    }

    #[test]
    fn mutation_root_and_aliases() {
        assert_eq!(at("mutation { | }"), ["refund", "__typename"]);
        assert_eq!(at("{ a: payment(id: 1) { | } }")[0], "id");
    }

    #[test]
    fn argument_names_and_enum_values() {
        assert_eq!(at("{ payments(|) { id } }"), ["first", "status"]);
        assert_eq!(at("{ payments(first: 1, st|) { id } }"), ["status"]);
        assert_eq!(at("{ payments(status: |) { id } }"), ["PAID", "PENDING"]);
        assert_eq!(at("{ payments(status: P|) { id } }"), ["PAID", "PENDING"]);
    }

    #[test]
    fn variables_and_types() {
        assert_eq!(
            at("query($id: ID!, $n: Int) { payment(id: $|) { id } }"),
            ["id", "n"]
        );
        assert!(at("query($id: |) { a }").contains(&"ID".to_string()));
    }

    #[test]
    fn top_level_keywords() {
        assert_eq!(at("|"), ["query", "mutation", "subscription", "fragment"]);
        assert_eq!(at("que|"), ["query"]);
    }

    #[test]
    fn closed_blocks_return_to_the_parent_type() {
        assert_eq!(
            at("{ payment(id: 1) { id } | }"),
            ["payment", "payments", "__typename"]
        );
    }
}
