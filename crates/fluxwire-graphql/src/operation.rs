//! Build the GraphQL-over-HTTP request from a document's blocks (SPEC §7.9).

use fluxwire_model::{BlockKind, Diagnostic, FluxDoc, FmValue, Severity, Span};
use fluxwire_vars::VarScope;
use serde_json::{Map, Value};

use crate::parse;

/// The operation a document sends.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub query: String,
    pub variables: Option<Map<String, Value>>,
    pub operation_name: Option<String>,
}

impl Operation {
    /// The `{query, variables, operationName}` JSON object of a POST body.
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("query".into(), Value::String(self.query.clone()));
        if let Some(v) = &self.variables {
            o.insert("variables".into(), Value::Object(v.clone()));
        }
        if let Some(n) = &self.operation_name {
            o.insert("operationName".into(), Value::String(n.clone()));
        }
        Value::Object(o)
    }
}

fn err(code: &'static str, span: Span, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, span, msg)
}

fn block_text(doc: &FluxDoc, kind: BlockKind) -> Option<(Span, String)> {
    let (_, b) = doc.blocks_of(kind).find(|(_, b)| b.top_level && b.closed)?;
    let text = b.content.slice(&doc.source);
    Some((b.content, text.trim_end_matches(['\r', '\n']).to_string()))
}

/// Interpolate `{{var}}` references, collecting unresolved names as warnings.
fn interpolate(
    text: &str,
    scope: &VarScope,
    span: Span,
    warnings: &mut Vec<Diagnostic>,
    errors: &mut Vec<Diagnostic>,
) -> String {
    let r = fluxwire_vars::interpolate(text, scope);
    for m in r.missing {
        warnings.push(Diagnostic::new(
            "P101",
            Severity::Warning,
            span,
            format!("Variable '{m}' is not defined; it is sent as written."),
        ));
    }
    for s in r.schemes {
        errors.push(err(
            "P103",
            span,
            format!("'{{{{{s}}}}}' is a secret reference that this version cannot resolve."),
        ));
    }
    r.text
}

/// The operation of `doc`, or the diagnostics that stop it from being sent.
pub fn from_doc(
    doc: &FluxDoc,
    scope: &VarScope,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Operation, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let Some((qspan, qtext)) = block_text(doc, BlockKind::GraphqlQuery) else {
        return Err(vec![err(
            "G101",
            Span::at(0),
            "This file has no `graphql query` block.",
        )]);
    };
    if qtext.trim().is_empty() {
        errors.push(err("G101", qspan, "The `graphql query` block is empty."));
    }
    let query = interpolate(&qtext, scope, qspan, warnings, &mut errors);

    let mut variables = None;
    if let Some((vspan, vtext)) = block_text(doc, BlockKind::GraphqlVariables) {
        let text = interpolate(&vtext, scope, vspan, warnings, &mut errors);
        if !text.trim().is_empty() {
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(m)) => variables = Some(m),
                Ok(_) => errors.push(err(
                    "G102",
                    vspan,
                    "The `json variables` block must be a JSON object.",
                )),
                Err(e) => errors.push(err(
                    "G102",
                    vspan,
                    format!("The `json variables` block is not valid JSON: {e}."),
                )),
            }
        }
    }

    // `operation` front matter picks one operation of a multi-operation document.
    let named = match doc.front.get("operation").map(|e| &e.value) {
        Some(FmValue::Scalar(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let mut operation_name = named;
    if let Ok(parsed) = parse::parse(&query) {
        let names: Vec<&str> = parsed
            .operations()
            .filter_map(|o| o.name.as_deref())
            .collect();
        let count = parsed.operations().count();
        match &operation_name {
            Some(n) if !names.contains(&n.as_str()) => errors.push(err(
                "G103",
                qspan,
                format!("The query has no operation named '{n}'."),
            )),
            None if count > 1 => errors.push(err(
                "G103",
                qspan,
                "The query has several operations; set `operation: Name` in the front matter.",
            )),
            None if count == 1 => operation_name = names.first().map(|s| s.to_string()),
            _ => {}
        }
    }

    if errors.is_empty() {
        Ok(Operation {
            query,
            variables,
            operation_name,
        })
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_vars::VarScope;

    fn doc(body: &str) -> FluxDoc {
        // The parser lives in fluxwire-format, a dev-dependency only.
        fluxwire_format::parse(&format!(
            "---\nname: t\nrequires: [graphql]\n---\n\n```http\nPOST https://x.test/graphql\n```\n\n{body}"
        ))
    }

    #[test]
    fn builds_query_variables_and_operation_name() {
        let d = doc(
            "```graphql query\nquery P($id: ID!) { p(id: $id) { s } }\n```\n\n```json variables\n{\"id\": \"7\"}\n```\n",
        );
        let mut w = Vec::new();
        let op = from_doc(&d, &VarScope::default(), &mut w).unwrap();
        assert_eq!(op.operation_name.as_deref(), Some("P"));
        assert_eq!(op.variables.unwrap()["id"], "7");
        assert!(op.query.starts_with("query P"));
    }

    #[test]
    fn rejects_bad_variables_and_ambiguous_operations() {
        let d = doc("```graphql query\n{ a }\n```\n\n```json variables\n[1]\n```\n");
        let e = from_doc(&d, &VarScope::default(), &mut Vec::new()).unwrap_err();
        assert_eq!(e[0].code, "G102");
        let d = doc("```graphql query\nquery A { a }\nquery B { b }\n```\n");
        let e = from_doc(&d, &VarScope::default(), &mut Vec::new()).unwrap_err();
        assert_eq!(e[0].code, "G103");
    }

    #[test]
    fn missing_variables_warn() {
        let d = doc("```graphql query\n{ a(x: \"{{nope}}\") }\n```\n");
        let mut w = Vec::new();
        from_doc(&d, &VarScope::default(), &mut w).unwrap();
        assert_eq!(w[0].code, "P101");
    }
}
