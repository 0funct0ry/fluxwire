//! Split a GraphQL response body into `data` and `errors` for the response viewer (M16.4).
//! Pure: used by the viewer whenever a JSON body has that shape.

/// A `message` of a GraphQL response's `errors` array with where it points in the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseError {
    pub message: String,
    pub path: String,
    pub line_col: Option<(usize, usize)>,
}

/// The `data` and `errors` of a GraphQL response body, split for the viewer.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub data: Option<String>,
    pub errors: Vec<ResponseError>,
}

pub fn split_response(body: &str) -> Option<Split> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let o = v.as_object()?;
    if !o.contains_key("data") && !o.contains_key("errors") {
        return None;
    }
    let data = o
        .get("data")
        .filter(|d| !d.is_null())
        .and_then(|d| serde_json::to_string_pretty(d).ok());
    let errors = o
        .get("errors")
        .and_then(|e| e.as_array())
        .map(|a| {
            a.iter()
                .map(|e| ResponseError {
                    message: e
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("(no message)")
                        .to_string(),
                    path: e
                        .get("path")
                        .and_then(|p| p.as_array())
                        .map(|p| {
                            p.iter()
                                .map(|s| match s {
                                    serde_json::Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default(),
                    line_col: e.get("locations").and_then(|l| l.get(0)).and_then(|l| {
                        Some((
                            l.get("line")?.as_u64()? as usize,
                            l.get("column")?.as_u64()? as usize,
                        ))
                    }),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Split { data, errors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_splits_into_data_and_errors_with_locations() {
        let s = split_response(
            r#"{"data": {"a": 1}, "errors": [{"message": "bad", "path": ["a", 0], "locations": [{"line": 2, "column": 5}]}]}"#,
        )
        .unwrap();
        assert!(s.data.unwrap().contains("\"a\": 1"));
        assert_eq!(s.errors[0].path, "a.0");
        assert_eq!(s.errors[0].line_col, Some((2, 5)));
        assert!(split_response(r#"{"data": null}"#).unwrap().data.is_none());
        assert!(split_response("[1]").is_none());
        assert!(split_response(r#"{"x": 1}"#).is_none());
    }
}
