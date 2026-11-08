//! Validate a query against a schema: the checks that catch typos before the server does.

use std::collections::HashSet;
use std::ops::Range;

use crate::parse::{self, Arg, Definition, Field, OperationDef, Selection, SelectionSet, Value};
use crate::schema::{InputValue, Kind, Schema, TypeDef, TypeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub code: &'static str,
    pub message: String,
    pub span: Range<usize>,
}

struct V<'a> {
    schema: &'a Schema,
    out: Vec<Problem>,
    defined: HashSet<String>,
    used: HashSet<String>,
}

impl V<'_> {
    fn err(&mut self, code: &'static str, span: Range<usize>, message: String) {
        self.out.push(Problem {
            level: Level::Error,
            code,
            message,
            span,
        });
    }

    fn set(&mut self, parent: &TypeDef, set: &SelectionSet, frags: &parse::Document) {
        for s in &set.items {
            match s {
                Selection::Field(f) => self.field(parent, f, frags),
                Selection::Spread { name, span } => {
                    if frags.fragment(name).is_none() {
                        self.err("G203", span.clone(), format!("Unknown fragment '{name}'."));
                    }
                }
                Selection::Inline {
                    type_condition,
                    selection,
                    span,
                } => {
                    let ty = match type_condition {
                        Some(t) => match self.schema.ty(t) {
                            Some(t) if t.is_composite() => t,
                            _ => {
                                self.err("G202", span.clone(), format!("Unknown type '{t}'."));
                                continue;
                            }
                        },
                        None => parent,
                    };
                    self.set(ty, selection, frags);
                }
            }
        }
    }

    fn field(&mut self, parent: &TypeDef, f: &Field, frags: &parse::Document) {
        if f.name == "__typename" {
            return;
        }
        let Some(def) = parent.field(&f.name) else {
            let hint = if parent.kind == Kind::Union {
                " A union only has `__typename` and inline fragments."
            } else {
                ""
            };
            self.err(
                "G201",
                f.name_span.clone(),
                format!("Type '{}' has no field '{}'.{hint}", parent.name, f.name),
            );
            return;
        };
        if let Some(reason) = &def.deprecated {
            self.out.push(Problem {
                level: Level::Warning,
                code: "G210",
                message: if reason.is_empty() {
                    format!("'{}' is deprecated.", f.name)
                } else {
                    format!("'{}' is deprecated: {reason}", f.name)
                },
                span: f.name_span.clone(),
            });
        }
        self.args(&def.args, &f.args, f);
        let target = self.schema.ty(def.ty.named());
        match (&f.selection, target) {
            (Some(sel), Some(t)) if t.is_composite() => self.set(t, sel, frags),
            (Some(sel), Some(_)) => self.err(
                "G204",
                sel.span.clone(),
                format!(
                    "'{}' is a {} and cannot have a selection set.",
                    f.name,
                    def.ty.named()
                ),
            ),
            (None, Some(t)) if t.is_composite() => self.err(
                "G205",
                f.name_span.clone(),
                format!(
                    "'{}' returns '{}' and needs a selection of its fields.",
                    f.name,
                    def.ty.named()
                ),
            ),
            _ => {}
        }
    }

    fn args(&mut self, defs: &[InputValue], given: &[Arg], f: &Field) {
        for a in given {
            match defs.iter().find(|d| d.name == a.name) {
                None => self.err(
                    "G206",
                    a.name_span.clone(),
                    format!("Field '{}' has no argument '{}'.", f.name, a.name),
                ),
                Some(d) => self.value(&a.value, &d.ty, a.span.clone()),
            }
        }
        for d in defs.iter().filter(|d| d.required()) {
            if !given.iter().any(|a| a.name == d.name) {
                self.err(
                    "G207",
                    f.name_span.clone(),
                    format!(
                        "Argument '{}: {}' of '{}' is required.",
                        d.name, d.ty, f.name
                    ),
                );
            }
        }
    }

    fn value(&mut self, v: &Value, ty: &TypeRef, span: Range<usize>) {
        let inner = match ty {
            TypeRef::NonNull(t) => t,
            t => t,
        };
        match v {
            Value::Variable(n) => {
                self.used.insert(n.clone());
                if !self.defined.contains(n) {
                    self.err("G208", span, format!("Variable '${n}' is not defined."));
                }
            }
            Value::Null => {
                if ty.is_non_null() {
                    self.err("G209", span, format!("Expected {ty}, found null."));
                }
            }
            Value::List(items) => {
                if let TypeRef::List(t) = inner {
                    for i in items {
                        self.value(i, t, span.clone());
                    }
                }
            }
            Value::Enum(e) => {
                if let Some(t) = self.schema.ty(inner.named())
                    && t.kind == Kind::Enum
                    && !t.enum_values.contains(e)
                {
                    self.err(
                        "G209",
                        span,
                        format!("'{e}' is not a value of enum '{}'.", t.name),
                    );
                } else if let Some(t) = self.schema.ty(inner.named())
                    && t.kind == Kind::Scalar
                {
                    self.err("G209", span, format!("Expected {}, found {e}.", t.name));
                }
            }
            Value::Object(fields) => {
                if let Some(t) = self.schema.ty(inner.named())
                    && t.kind == Kind::InputObject
                {
                    for (n, val) in fields {
                        match t.input_fields.iter().find(|f| &f.name == n) {
                            Some(f) => self.value(val, &f.ty, span.clone()),
                            None => self.err(
                                "G206",
                                span.clone(),
                                format!("Input '{}' has no field '{n}'.", t.name),
                            ),
                        }
                    }
                    for f in t.input_fields.iter().filter(|f| f.required()) {
                        if !fields.iter().any(|(n, _)| n == &f.name) {
                            self.err(
                                "G207",
                                span.clone(),
                                format!("Input field '{}: {}' is required.", f.name, f.ty),
                            );
                        }
                    }
                }
            }
            Value::Int | Value::Float | Value::String | Value::Bool => {
                let name = inner.named();
                let ok = match (v, name) {
                    (Value::Int, "Int" | "Float" | "ID") => true,
                    (Value::Float, "Float") => true,
                    (Value::String, "String" | "ID") => true,
                    (Value::Bool, "Boolean") => true,
                    // custom scalars accept any literal
                    (_, n) => {
                        self.schema.ty(n).is_some_and(|t| t.kind == Kind::Scalar)
                            && !matches!(n, "Int" | "Float" | "ID" | "String" | "Boolean")
                    }
                };
                if !ok {
                    self.err(
                        "G209",
                        span,
                        format!("Expected {ty}, found a literal of another type."),
                    );
                }
            }
        }
    }

    fn operation(&mut self, op: &OperationDef, doc: &parse::Document) {
        self.defined = op.variables.iter().map(|v| v.name.clone()).collect();
        self.used.clear();
        for v in &op.variables {
            if let Some(t) = TypeRef::parse(&v.ty)
                && self.schema.ty(t.named()).is_none()
            {
                self.err(
                    "G202",
                    v.span.clone(),
                    format!("Unknown type '{}'.", t.named()),
                );
            }
        }
        let Some(root) = self.schema.root(op.kind) else {
            self.err(
                "G200",
                op.span.clone(),
                format!("This schema has no {} type.", op.kind.keyword()),
            );
            return;
        };
        self.set(root, &op.selection, doc);
        for v in &op.variables {
            if !self.used.contains(&v.name) {
                self.out.push(Problem {
                    level: Level::Warning,
                    code: "G211",
                    message: format!("Variable '${}' is never used.", v.name),
                    span: v.span.clone(),
                });
            }
        }
    }
}

/// Problems in `src`, in source order. A parse failure is reported as the only problem.
pub fn validate(src: &str, schema: &Schema) -> Vec<Problem> {
    let doc = match parse::parse(src) {
        Ok(d) => d,
        Err(e) => {
            return vec![Problem {
                level: Level::Error,
                code: "G100",
                message: e.message,
                span: e.offset..(e.offset + 1).min(src.len()).max(e.offset),
            }];
        }
    };
    let mut v = V {
        schema,
        out: Vec::new(),
        defined: HashSet::new(),
        used: HashSet::new(),
    };
    for d in &doc.definitions {
        match d {
            Definition::Operation(op) => v.operation(op, &doc),
            Definition::Fragment(f) => match schema.ty(&f.type_condition) {
                Some(t) if t.is_composite() => v.set(t, &f.selection, &doc),
                _ => v.err(
                    "G202",
                    f.span.clone(),
                    format!("Unknown type '{}'.", f.type_condition),
                ),
            },
        }
    }
    v.out.sort_by_key(|p| p.span.start);
    v.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::sample_schema;

    fn codes(q: &str) -> Vec<&'static str> {
        validate(q, &sample_schema())
            .iter()
            .map(|p| p.code)
            .collect()
    }

    #[test]
    fn a_correct_query_is_clean() {
        assert!(
            codes("query($id: ID!) { payment(id: $id) { id status customer { name } } }")
                .is_empty()
        );
    }

    #[test]
    fn unknown_field_and_argument() {
        assert_eq!(codes("{ payment(id: \"1\") { nope } }"), ["G201"]);
        assert_eq!(codes("{ payment(id: \"1\", zzz: 1) { id } }"), ["G206"]);
    }

    #[test]
    fn required_argument_and_selection_rules() {
        assert_eq!(codes("{ payment { id } }"), ["G207"]);
        assert_eq!(codes("{ payment(id: 1) }"), ["G205"]);
        assert_eq!(codes("{ payment(id: 1) { id { x } } }"), ["G204"]);
    }

    #[test]
    fn variables_defined_and_used() {
        assert_eq!(codes("{ payment(id: $x) { id } }"), ["G208"]);
        assert_eq!(
            codes("query($x: ID) { payment(id: \"1\") { id } }"),
            ["G211"]
        );
    }

    #[test]
    fn enum_and_literal_types() {
        assert_eq!(codes("{ payments(status: BOGUS) { id } }"), ["G209"]);
        assert!(codes("{ payments(status: PAID, first: 2) { id } }").is_empty());
        assert_eq!(codes("{ payments(first: \"two\") { id } }"), ["G209"]);
    }

    #[test]
    fn deprecated_fields_warn_and_parse_errors_are_single() {
        let p = validate("{ payment(id: 1) { amount } }", &sample_schema());
        assert_eq!((p[0].code, p[0].level), ("G210", Level::Warning));
        let p = validate("{ payment(", &sample_schema());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].code, "G100");
    }

    #[test]
    fn input_object_fields_are_checked() {
        assert!(codes("mutation { refund(input: {paymentId: \"1\"}) { id } }").is_empty());
        assert_eq!(
            codes("mutation { refund(input: {reason: \"x\"}) { id } }"),
            ["G207"]
        );
    }
}
