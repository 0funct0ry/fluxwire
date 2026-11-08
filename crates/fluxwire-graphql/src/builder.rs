//! The query builder model (M16.3): a tree of selected fields with argument values that renders
//! to a query, and that is rebuilt from query text when the text parses. The UI draws the schema
//! with checkboxes from `Schema` and edits this tree; nothing here touches GPUI.

use crate::parse::{self, OpKind, Selection, SelectionSet};
use crate::schema::{FieldDef, Kind, Schema, TypeDef, TypeRef};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderError {
    /// The text does not parse yet.
    Parse(String),
    /// The text parses, but uses something the tree cannot show (the text stays as written).
    Unsupported(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node {
    pub field: String,
    pub alias: Option<String>,
    /// `(name, value as written)`, e.g. `("first", "10")` or `("id", "$id")`.
    pub args: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Builder {
    pub kind: OpKind,
    pub name: Option<String>,
    pub variables: Vec<Variable>,
    pub roots: Vec<Node>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new(OpKind::Query)
    }
}

fn node_of(set: &SelectionSet) -> Result<Vec<Node>, BuilderError> {
    set.items
        .iter()
        .map(|s| match s {
            Selection::Field(f) => Ok(Node {
                field: f.name.clone(),
                alias: f.alias.clone(),
                args: f
                    .args
                    .iter()
                    .map(|a| (a.name.clone(), a.text.clone()))
                    .collect(),
                children: match &f.selection {
                    Some(s) => node_of(s)?,
                    None => Vec::new(),
                },
            }),
            _ => Err(BuilderError::Unsupported(
                "fragments are not shown in the builder",
            )),
        })
        .collect()
}

/// Names of `$variables` used in an argument value.
fn var_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
        } else if b[i] == b'$' {
            let s = i + 1;
            let mut e = s;
            while e < b.len() && (b[e] == b'_' || b[e].is_ascii_alphanumeric()) {
                e += 1;
            }
            if e > s {
                out.push(text[s..e].to_string());
            }
            i = e;
            continue;
        }
        i += 1;
    }
    out
}

impl Builder {
    pub fn new(kind: OpKind) -> Self {
        Self {
            kind,
            name: None,
            variables: Vec::new(),
            roots: Vec::new(),
        }
    }

    /// The tree for the first operation of `src`.
    pub fn from_query(src: &str) -> Result<Builder, BuilderError> {
        let doc = match parse::parse(src) {
            Ok(d) => d,
            // `query Name { }` is what the builder leaves behind when every field is unchecked:
            // it is not valid GraphQL, but it is a valid starting point for the tree.
            Err(e) => match skeleton_of(src) {
                Some(mut b) => {
                    b.roots.clear();
                    return Ok(b);
                }
                None => return Err(BuilderError::Parse(e.message)),
            },
        };
        if doc.operations().count() != 1 || doc.definitions.len() != 1 {
            return Err(BuilderError::Unsupported(
                "the builder edits documents with a single operation and no fragments",
            ));
        }
        let op = doc.operations().next().expect("one operation");
        Ok(Builder {
            kind: op.kind,
            name: op.name.clone(),
            variables: op
                .variables
                .iter()
                .map(|v| Variable {
                    name: v.name.clone(),
                    ty: v.ty.clone(),
                    default: v.default.clone(),
                })
                .collect(),
            roots: node_of(&op.selection)?,
        })
    }

    pub fn find(&self, path: &[&str]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut n = self.roots.iter().find(|n| n.field == *first)?;
        for p in rest {
            n = n.children.iter().find(|c| c.field == *p)?;
        }
        Some(n)
    }

    fn find_mut(&mut self, path: &[&str]) -> Option<&mut Node> {
        let (first, rest) = path.split_first()?;
        let mut n = self.roots.iter_mut().find(|n| n.field == *first)?;
        for p in rest {
            n = n.children.iter_mut().find(|c| c.field == *p)?;
        }
        Some(n)
    }

    pub fn is_checked(&self, path: &[&str]) -> bool {
        self.find(path).is_some()
    }

    /// The type a field at `path` returns, walking from the root type.
    fn type_at<'s>(&self, schema: &'s Schema, path: &[&str]) -> Option<&'s TypeDef> {
        let mut t = schema.root(self.kind)?;
        for p in path {
            t = schema.ty(t.field(p)?.ty.named())?;
        }
        Some(t)
    }

    fn field_def<'s>(&self, schema: &'s Schema, path: &[&str]) -> Option<&'s FieldDef> {
        let (last, parent) = path.split_last()?;
        self.type_at(schema, parent)?.field(last)
    }

    /// Give every required argument of the node at `path` that has no value a starting value,
    /// so a freshly checked field is a complete, valid selection.
    fn fill_required(&mut self, schema: &Schema, path: &[&str]) {
        let Some(def) = self.field_def(schema, path) else {
            return;
        };
        let missing: Vec<(String, String)> = def
            .args
            .iter()
            .filter(|a| a.required())
            .filter(|a| self.arg(path, &a.name).is_none())
            .map(|a| {
                (
                    a.name.clone(),
                    starting_literal(schema, &a.ty).unwrap_or_else(|| format!("${}", a.name)),
                )
            })
            .collect();
        for (name, value) in missing {
            self.set_arg(path, &name, Some(value));
        }
    }

    /// Check or uncheck the field at `path`. Checking adds the ancestors and, for an object
    /// field with nothing under it yet, one default sub-field so the query stays valid.
    pub fn set_checked(&mut self, schema: &Schema, path: &[&str], checked: bool) {
        if path.is_empty() {
            return;
        }
        if !checked {
            let (last, parent) = path.split_last().expect("non-empty");
            let list = if parent.is_empty() {
                Some(&mut self.roots)
            } else {
                self.find_mut(parent).map(|n| &mut n.children)
            };
            if let Some(l) = list {
                l.retain(|n| n.field != *last);
            }
            // an object left with no fields is no longer a valid selection: drop it too
            if !parent.is_empty() && self.find(parent).is_some_and(|n| n.children.is_empty()) {
                self.set_checked(schema, parent, false);
            }
            return;
        }
        let mut created: Vec<usize> = Vec::new();
        for depth in 1..=path.len() {
            let p = &path[..depth];
            if self.find(p).is_none() {
                created.push(depth);
                let node = Node {
                    field: p[depth - 1].to_string(),
                    ..Node::default()
                };
                let list = if depth == 1 {
                    &mut self.roots
                } else {
                    &mut self
                        .find_mut(&p[..depth - 1])
                        .expect("ancestor exists")
                        .children
                };
                list.push(node);
            }
        }
        for depth in created {
            self.fill_required(schema, &path[..depth]);
        }
        if let Some(t) = self.type_at(schema, path)
            && t.is_composite()
            && self.find(path).is_some_and(|n| n.children.is_empty())
            && let Some(d) = default_child(t)
        {
            self.find_mut(path)
                .expect("just added")
                .children
                .push(Node {
                    field: d,
                    ..Node::default()
                });
        }
    }

    /// Set (or, with `None`, remove) an argument of a checked field.
    pub fn set_arg(&mut self, path: &[&str], name: &str, value: Option<String>) {
        let Some(n) = self.find_mut(path) else { return };
        n.args.retain(|(k, _)| k != name);
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            n.args.push((name.to_string(), v));
        }
    }

    /// The declared type of argument `name` of the field at `path`.
    pub fn arg_type(&self, schema: &Schema, path: &[&str], name: &str) -> Option<TypeRef> {
        let (last, parent) = path.split_last()?;
        let owner = self.type_at(schema, parent)?;
        owner
            .field(last)?
            .args
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.ty.clone())
    }

    pub fn arg(&self, path: &[&str], name: &str) -> Option<&str> {
        self.find(path)?
            .args
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Variable definitions for every `$ref` in the arguments: existing ones that are still used
    /// keep their type and default; new ones take the type of the argument they feed.
    pub fn variable_defs(&self, schema: &Schema) -> Vec<Variable> {
        self.synced_variables(schema)
    }

    fn synced_variables(&self, schema: &Schema) -> Vec<Variable> {
        fn walk(
            schema: &Schema,
            parent: &TypeDef,
            nodes: &[Node],
            out: &mut Vec<(String, Option<TypeRef>)>,
        ) {
            for n in nodes {
                let def = parent.field(&n.field);
                for (an, av) in &n.args {
                    let ty = def
                        .and_then(|d| d.args.iter().find(|a| &a.name == an))
                        .map(|a| a.ty.clone());
                    let whole = av.trim().strip_prefix('$').map(str::to_string);
                    for r in var_refs(av) {
                        // only a bare `$x` takes the argument's type; nested refs keep theirs
                        let t = (whole.as_deref() == Some(r.as_str()))
                            .then(|| ty.clone())
                            .flatten();
                        out.push((r, t));
                    }
                }
                if let Some(t) = def.and_then(|d| schema.ty(d.ty.named())) {
                    walk(schema, t, &n.children, out);
                }
            }
        }
        let mut refs = Vec::new();
        if let Some(root) = schema.root(self.kind) {
            walk(schema, root, &self.roots, &mut refs);
        }
        let mut out: Vec<Variable> = Vec::new();
        for (name, ty) in refs {
            if out.iter().any(|v| v.name == name) {
                continue;
            }
            if let Some(v) = self.variables.iter().find(|v| v.name == name) {
                out.push(v.clone());
            } else if let Some(t) = ty {
                out.push(Variable {
                    name,
                    ty: t.to_string(),
                    default: None,
                });
            } else {
                out.push(Variable {
                    name,
                    ty: "String".into(),
                    default: None,
                });
            }
        }
        out
    }

    /// The query text. Variable definitions are kept in step with the `$refs` in arguments.
    pub fn render(&self, schema: &Schema) -> String {
        let vars = self.synced_variables(schema);
        let mut head = String::new();
        let anon = self.kind == OpKind::Query
            && self.name.is_none()
            && vars.is_empty()
            && !self.roots.is_empty();
        if !anon {
            head.push_str(self.kind.keyword());
            if let Some(n) = &self.name {
                head.push(' ');
                head.push_str(n);
            }
            if !vars.is_empty() {
                let defs: Vec<String> = vars
                    .iter()
                    .map(|v| match &v.default {
                        Some(d) => format!("${}: {} = {d}", v.name, v.ty),
                        None => format!("${}: {}", v.name, v.ty),
                    })
                    .collect();
                head.push_str(&format!("({})", defs.join(", ")));
            }
            head.push(' ');
        }
        let mut out = head;
        render_set(&self.roots, 0, &mut out);
        out.push('\n');
        out
    }
}

/// The builder of `src` when its only problem is an empty final selection set: the text up to
/// that `{ }` is parsed with a placeholder field.
fn skeleton_of(src: &str) -> Option<Builder> {
    let trimmed = src.trim_end();
    let body = trimmed.strip_suffix('}')?;
    let open = body.rfind('{')?;
    if !body[open + 1..].trim().is_empty() {
        return None;
    }
    let probe = format!("{}{{ __typename }}", &body[..open]);
    let mut b = Builder::from_query(&probe).ok()?;
    b.roots.clear();
    Some(b)
}

/// A value to start a required argument with: an empty string, zero, `false`, the first enum
/// value, an empty list or object. `None` when no neutral value exists.
pub fn starting_literal(schema: &Schema, ty: &TypeRef) -> Option<String> {
    if ty.is_list() {
        return Some("[]".into());
    }
    match ty.named() {
        "String" | "ID" => Some("\"\"".into()),
        "Int" | "Float" => Some("0".into()),
        "Boolean" => Some("false".into()),
        n => match schema.ty(n) {
            Some(t) if t.kind == Kind::Enum => t.enum_values.first().cloned(),
            Some(t) if t.kind == Kind::InputObject => Some("{}".into()),
            _ => None,
        },
    }
}

/// The JSON value a new variable of GraphQL type text `ty` (such as `ID!` or `[Int!]`) starts
/// with in the `json variables` block.
pub fn placeholder_json(ty: &str) -> serde_json::Value {
    use serde_json::Value;
    let t = ty.trim().trim_end_matches('!');
    if t.starts_with('[') {
        return Value::Array(Vec::new());
    }
    match t {
        "String" | "ID" => Value::String(String::new()),
        "Int" => Value::from(0),
        "Float" => Value::from(0.0),
        "Boolean" => Value::Bool(false),
        _ => Value::Null,
    }
}

/// The argument text to store for what the user typed. Bare words for `String` and `ID`
/// arguments are quoted; `$variables`, quoted strings and everything else are kept as written.
pub fn literal_for(ty: &TypeRef, input: &str) -> String {
    let v = input.trim();
    let stringy = matches!(ty.named(), "String" | "ID");
    let plain = !v.is_empty()
        && !v.starts_with(['$', '"', '[', '{'])
        && !matches!(v, "null" | "true" | "false");
    if stringy && !ty.is_list() && plain {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

/// The inverse of [`literal_for`]: what the argument input shows for a stored value.
pub fn input_for(ty: &TypeRef, value: &str) -> String {
    let stringy = matches!(ty.named(), "String" | "ID");
    if stringy
        && !ty.is_list()
        && let Some(inner) = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .filter(|v| !v.contains(['"', '\\']))
    {
        return inner.to_string();
    }
    value.to_string()
}

fn default_child(t: &TypeDef) -> Option<String> {
    let leaf = |f: &&crate::schema::FieldDef| {
        f.deprecated.is_none() && f.args.iter().all(|a| !a.required())
    };
    if t.field("id").is_some() {
        return Some("id".into());
    }
    t.fields
        .iter()
        .filter(leaf)
        .find(|f| f.ty.named() != t.name)
        .map(|f| f.name.clone())
        .or_else(|| (t.kind != Kind::Scalar).then(|| "__typename".to_string()))
}

fn render_set(nodes: &[Node], depth: usize, out: &mut String) {
    out.push_str("{\n");
    for n in nodes {
        out.push_str(&"  ".repeat(depth + 1));
        if let Some(a) = &n.alias {
            out.push_str(a);
            out.push_str(": ");
        }
        out.push_str(&n.field);
        if !n.args.is_empty() {
            let a: Vec<String> = n.args.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            out.push_str(&format!("({})", a.join(", ")));
        }
        if !n.children.is_empty() {
            out.push(' ');
            render_set(&n.children, depth + 1, out);
        }
        out.push('\n');
    }
    out.push_str(&"  ".repeat(depth));
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::sample_schema;

    #[test]
    fn checking_fields_builds_a_valid_query() {
        let s = sample_schema();
        let mut b = Builder::default();
        b.set_checked(&s, &["payments"], true);
        // an object field gets a default sub-field
        assert!(b.is_checked(&["payments", "id"]));
        b.set_checked(&s, &["payments", "status"], true);
        b.set_checked(&s, &["payments", "customer", "name"], true);
        let q = b.render(&s);
        assert_eq!(
            q,
            "{\n  payments {\n    id\n    status\n    customer {\n      name\n    }\n  }\n}\n"
        );
        assert!(crate::validate::validate(&q, &s).is_empty());
    }

    #[test]
    fn unchecking_prunes_empty_parents() {
        let s = sample_schema();
        let mut b = Builder::default();
        b.set_checked(&s, &["payments", "customer", "name"], true);
        b.set_checked(&s, &["payments", "customer", "name"], false);
        assert!(!b.is_checked(&["payments", "customer"]));
        b.set_checked(&s, &["payments", "id"], false);
        assert!(b.roots.is_empty());
    }

    #[test]
    fn arguments_and_variables_stay_in_step() {
        let s = sample_schema();
        let mut b = Builder::default();
        b.set_checked(&s, &["payment", "status"], true);
        b.set_arg(&["payment"], "id", Some("$id".into()));
        let q = b.render(&s);
        assert!(
            q.starts_with("query($id: ID!) {\n  payment(id: $id) {"),
            "{q}"
        );
        assert!(crate::validate::validate(&q, &s).is_empty());
        b.set_arg(&["payment"], "id", Some("\"7\"".into()));
        let q = b.render(&s);
        assert!(q.starts_with("{\n  payment(id: \"7\")"), "{q}");
        assert_eq!(b.arg(&["payment"], "id"), Some("\"7\""));
        b.set_arg(&["payment"], "id", None);
        assert!(b.arg(&["payment"], "id").is_none());
    }

    #[test]
    fn text_round_trips_through_the_tree() {
        let s = sample_schema();
        let src = "query Pay($id: ID!) {\n  payment(id: $id) {\n    id\n    status\n  }\n}\n";
        let b = Builder::from_query(src).unwrap();
        assert_eq!(b.name.as_deref(), Some("Pay"));
        assert!(b.is_checked(&["payment", "status"]));
        assert_eq!(b.render(&s), src);
    }

    #[test]
    fn text_that_does_not_parse_or_has_fragments_is_reported() {
        assert!(matches!(
            Builder::from_query("{ a {"),
            Err(BuilderError::Parse(_))
        ));
        assert!(matches!(
            Builder::from_query("{ a { ...F } } fragment F on A { b }"),
            Err(BuilderError::Unsupported(_))
        ));
        assert!(matches!(
            Builder::from_query("{ a { ... on A { b } } }"),
            Err(BuilderError::Unsupported(_))
        ));
    }

    #[test]
    fn unchecking_everything_keeps_the_operation_skeleton() {
        let schema = sample_schema();
        let mut b = Builder::new(OpKind::Query);
        b.name = Some("GetCountry".into());
        b.set_checked(&schema, &["hello"], true);
        let text = b.render(&schema);
        let mut b = Builder::from_query(&text).unwrap();
        b.set_checked(&schema, &["hello"], false);
        let empty = b.render(&schema);
        assert!(empty.starts_with("query GetCountry {"), "{empty}");
        let back = Builder::from_query(&empty).expect("skeleton is accepted");
        assert_eq!(back.name.as_deref(), Some("GetCountry"));
        assert!(back.roots.is_empty());
        assert!(Builder::from_query("query { }").unwrap().roots.is_empty());
        assert!(Builder::from_query("query { a ").is_err());
    }

    #[test]
    fn empty_default_builder_renders_a_query_stub() {
        let schema = sample_schema();
        assert_eq!(Builder::default().render(&schema), "query {\n}\n");
    }

    #[test]
    fn literals_quote_bare_strings_only() {
        let s = TypeRef::Named("String".into());
        let i = TypeRef::Named("Int".into());
        assert_eq!(literal_for(&s, "JP"), "\"JP\"");
        assert_eq!(literal_for(&s, "$code"), "$code");
        assert_eq!(literal_for(&s, "\"JP\""), "\"JP\"");
        assert_eq!(literal_for(&i, "10"), "10");
        assert_eq!(literal_for(&s, "  "), "");
        assert_eq!(input_for(&s, "\"JP\""), "JP");
        assert_eq!(input_for(&s, "$code"), "$code");
        assert_eq!(input_for(&i, "10"), "10");
    }

    #[test]
    fn checking_a_field_fills_its_required_arguments() {
        let schema = sample_schema();
        let mut b = Builder::new(OpKind::Query);
        b.set_checked(&schema, &["payment", "status"], true);
        assert_eq!(b.arg(&["payment"], "id"), Some("\"\""));
        let text = b.render(&schema);
        assert!(text.contains("payment(id: \"\")"), "{text}");
        // optional arguments are left alone
        b.set_checked(&schema, &["payments", "id"], true);
        assert_eq!(b.arg(&["payments"], "first"), None);
        // the user's own value is never overwritten by a later check
        b.set_arg(&["payment"], "id", Some("\"p1\"".into()));
        b.set_checked(&schema, &["payment", "amount"], true);
        assert_eq!(b.arg(&["payment"], "id"), Some("\"p1\""));
        assert!(parse::parse(&b.render(&schema)).is_ok());
    }

    #[test]
    fn starting_values_and_variable_placeholders_follow_the_type() {
        let schema = sample_schema();
        let named = |n: &str| TypeRef::Named(n.into());
        assert_eq!(
            starting_literal(&schema, &named("Status")),
            Some("PAID".into())
        );
        assert_eq!(
            starting_literal(&schema, &named("RefundInput")),
            Some("{}".into())
        );
        assert_eq!(
            starting_literal(&schema, &named("Boolean")),
            Some("false".into())
        );
        assert_eq!(placeholder_json("ID!"), serde_json::json!(""));
        assert_eq!(placeholder_json("[Int!]!"), serde_json::json!([]));
        assert_eq!(placeholder_json("Int"), serde_json::json!(0));
        assert_eq!(placeholder_json("RefundInput!"), serde_json::Value::Null);
    }

    #[test]
    fn mutations_use_their_own_root() {
        let s = sample_schema();
        let mut b = Builder::new(OpKind::Mutation);
        b.set_checked(&s, &["refund", "status"], true);
        b.set_arg(&["refund"], "input", Some("{paymentId: \"1\"}".into()));
        let q = b.render(&s);
        assert!(
            q.starts_with("mutation {\n  refund(input: {paymentId: \"1\"})"),
            "{q}"
        );
        assert!(crate::validate::validate(&q, &s).is_empty());
    }
}
