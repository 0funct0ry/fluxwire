//! `http expect` checking (SPEC §7.7): status, headers, and JSON/text body.

use fluxwire_model::{Body as DocBody, ExpectBlock, StatusMatch};
use fluxwire_protocol::{CheckCtx, Exchange, TestResult};
use serde_json::Value;

const WILDCARD: &str = "{{*}}";
/// Largest body read for checking.
const BODY_LIMIT: usize = 16 * 1024 * 1024;

fn interp(cx: &CheckCtx<'_>, text: &str) -> String {
    fluxwire_vars::interpolate(text, cx.scope).text
}

fn t(name: String, passed: bool, expected: Option<String>, actual: Option<String>) -> TestResult {
    TestResult {
        name,
        passed,
        expected,
        actual,
        message: None,
    }
}

fn fail(name: String, message: impl Into<String>) -> TestResult {
    TestResult {
        name,
        passed: false,
        expected: None,
        actual: None,
        message: Some(message.into()),
    }
}

pub fn check(x: &Exchange, expect: &ExpectBlock, cx: &CheckCtx<'_>) -> Vec<TestResult> {
    check_opts(x, expect, cx, false)
}

/// Like `check`. With `data_root`, an expected JSON object that names neither `data` nor
/// `errors` is matched against the response's `data` member (GraphQL partial matching).
pub fn check_opts(
    x: &Exchange,
    expect: &ExpectBlock,
    cx: &CheckCtx<'_>,
    data_root: bool,
) -> Vec<TestResult> {
    let mut out = Vec::new();

    let (ok, want) = match expect.status {
        StatusMatch::Exact(c) => (x.status == c, c.to_string()),
        StatusMatch::Class(c) => (x.status / 100 == c as u16, format!("{c}xx")),
        StatusMatch::Any => (true, "any".to_string()),
    };
    out.push(t(
        "expect status".into(),
        ok,
        Some(want),
        Some(x.status.to_string()),
    ));

    if let Some(v) = &expect.version {
        let want = v.trim_start_matches("HTTP/").trim_end_matches(".0");
        let have = x.version.trim_start_matches("HTTP/").trim_end_matches(".0");
        out.push(t(
            "expect version".into(),
            want == have,
            Some(v.clone()),
            Some(x.version.clone()),
        ));
    }

    for h in &expect.headers {
        let want = interp(cx, &h.value);
        let want = want.trim();
        let actual = x.header(&h.name);
        let passed = match actual {
            None => false,
            Some(_) if want == WILDCARD => true,
            Some(a) => match want.strip_prefix("~/").and_then(|r| r.strip_suffix('/')) {
                Some(re) => regex::Regex::new(re).is_ok_and(|re| re.is_match(a)),
                None => a.trim() == want,
            },
        };
        out.push(t(
            format!("expect header {}", h.name),
            passed,
            Some(want.to_string()),
            actual.map(str::to_string),
        ));
    }

    out.extend(check_body(x, expect, cx, data_root));
    out
}

fn check_body(
    x: &Exchange,
    expect: &ExpectBlock,
    cx: &CheckCtx<'_>,
    data_root: bool,
) -> Vec<TestResult> {
    let DocBody::Raw(span) = &expect.body else {
        return Vec::new();
    };
    let Some(doc) = cx.doc else { return Vec::new() };
    let raw = span.slice(&doc.source);
    let want_text = interp(cx, raw);
    let want_text = want_text.trim();
    if want_text.is_empty() {
        return Vec::new();
    }
    let bytes = match x.body.read_prefix(BODY_LIMIT) {
        Ok(b) => b,
        Err(e) => {
            return vec![fail(
                "expect body".into(),
                format!("cannot read the body: {e}"),
            )];
        }
    };
    let actual_text = String::from_utf8_lossy(&bytes);

    match serde_json::from_str::<Value>(want_text) {
        Ok(want) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(actual) => {
                let mut out = Vec::new();
                let rooted = data_root
                    && want
                        .as_object()
                        .is_some_and(|o| !o.contains_key("data") && !o.contains_key("errors"));
                if rooted {
                    walk("$.data", &want, actual.get("data"), &mut out);
                } else {
                    walk("$", &want, Some(&actual), &mut out);
                }
                out
            }
            Err(e) => vec![fail(
                "expect body".into(),
                format!("the body is not JSON ({e}), but the expected body is."),
            )],
        },
        Err(_) => vec![t(
            "expect body".into(),
            actual_text.trim() == want_text,
            Some(want_text.to_string()),
            Some(actual_text.trim().to_string()),
        )],
    }
}

fn child(path: &str, key: &str) -> String {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        format!("{path}.{key}")
    } else {
        format!("{path}[{}]", Value::String(key.to_string()))
    }
}

fn show(v: &Value) -> String {
    v.to_string()
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

/// Partial deep match: every expected key must exist and match; extra actual keys are ignored.
/// Arrays match element by element with equal length. One test per expected leaf.
fn walk(path: &str, want: &Value, actual: Option<&Value>, out: &mut Vec<TestResult>) {
    let name = format!("expect body {path}");
    match want {
        Value::String(s) if s.trim() == WILDCARD => {
            out.push(t(
                name,
                actual.is_some(),
                Some("any value".into()),
                actual.map(show),
            ));
        }
        Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                walk(&child(path, k), v, actual.and_then(|a| a.get(k)), out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let len = actual.and_then(Value::as_array).map(Vec::len);
            out.push(t(
                format!("{name} length"),
                len == Some(items.len()),
                Some(items.len().to_string()),
                len.map(|l| l.to_string()),
            ));
            for (i, v) in items.iter().enumerate() {
                walk(
                    &format!("{path}[{i}]"),
                    v,
                    actual.and_then(|a| a.get(i)),
                    out,
                );
            }
        }
        leaf => {
            let passed = actual.is_some_and(|a| values_eq(leaf, a));
            out.push(t(name, passed, Some(show(leaf)), actual.map(show)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use fluxwire_model::{Header, Span};
    use fluxwire_protocol::Body;
    use fluxwire_vars::VarScope;

    fn exchange(status: u16, headers: &[(&str, &str)], body: &str) -> Exchange {
        Exchange {
            status,
            status_text: String::new(),
            version: "HTTP/1.1".into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Body::Memory(Bytes::from(body.to_string())),
            content_type: None,
            wire_size: 0,
            decoded_size: 0,
        }
    }

    /// An expect block whose body is `body` (stored as the doc source).
    fn run(
        x: &Exchange,
        status: StatusMatch,
        headers: Vec<Header>,
        body: &str,
        scope: &VarScope,
    ) -> Vec<TestResult> {
        let doc = fluxwire_format::parse(body);
        assert_eq!(doc.source, body);
        let expect = ExpectBlock {
            status,
            version: None,
            headers,
            body: if body.is_empty() {
                DocBody::None
            } else {
                DocBody::Raw(Span::new(0, body.len()))
            },
        };
        check(
            x,
            &expect,
            &CheckCtx {
                doc: Some(&doc),
                scope,
            },
        )
    }

    fn hdr(name: &str, value: &str) -> Header {
        Header {
            enabled: true,
            name: name.into(),
            value: value.into(),
            span: Span::new(0, 0),
        }
    }

    fn by_name(r: &[TestResult]) -> std::collections::BTreeMap<&str, bool> {
        r.iter().map(|t| (t.name.as_str(), t.passed)).collect()
    }

    #[test]
    fn status_classes_and_any() {
        let s = VarScope::default();
        let x = exchange(201, &[], "");
        assert!(run(&x, StatusMatch::Exact(201), vec![], "", &s)[0].passed);
        assert!(run(&x, StatusMatch::Class(2), vec![], "", &s)[0].passed);
        assert!(run(&x, StatusMatch::Any, vec![], "", &s)[0].passed);
        assert!(!run(&x, StatusMatch::Exact(200), vec![], "", &s)[0].passed);
        assert!(!run(&x, StatusMatch::Class(4), vec![], "", &s)[0].passed);
    }

    #[test]
    fn headers_exact_wildcard_regex_and_interpolation() {
        let mut s = VarScope::default();
        s.set_literal("ct", "application/json");
        let x = exchange(
            200,
            &[("content-type", "application/json"), ("X-Id", "req-42")],
            "",
        );
        let r = run(
            &x,
            StatusMatch::Any,
            vec![
                hdr("Content-Type", "{{ct}}"),
                hdr("X-Id", "~/^req-\\d+$/"),
                hdr("X-Id", "{{*}}"),
                hdr("X-Missing", "{{*}}"),
                hdr("X-Id", "nope"),
            ],
            "",
            &s,
        );
        let passed: Vec<_> = r.iter().skip(1).map(|t| t.passed).collect();
        assert_eq!(passed, [true, true, true, false, false]);
        assert_eq!(r[1].name, "expect header Content-Type");
    }

    #[test]
    fn json_partial_deep_match() {
        let s = VarScope::default();
        let x = exchange(
            201,
            &[],
            r#"{"status":"requires_capture","amount":4200,"extra":1,"customer":{"id":"c1","name":"A"},"items":[{"sku":"a"},{"sku":"b"}]}"#,
        );
        let r = run(
            &x,
            StatusMatch::Any,
            vec![],
            r#"{ "status": "requires_capture", "amount": 4200.0, "id": "{{*}}", "customer": {"id":"c1"}, "items": [{"sku":"a"},{"sku":"b"}] }"#,
            &s,
        );
        let m = by_name(&r);
        assert!(m["expect body $.status"]);
        assert!(m["expect body $.amount"]);
        assert!(m["expect body $.customer.id"]);
        assert!(m["expect body $.items length"]);
        assert!(m["expect body $.items[1].sku"]);
        assert!(
            !m["expect body $.id"],
            "wildcard still requires the key to exist"
        );
        assert!(!m.contains_key("expect body $.extra"));

        let r = run(
            &x,
            StatusMatch::Any,
            vec![],
            r#"{"items":[{"sku":"a"}]}"#,
            &s,
        );
        assert!(!by_name(&r)["expect body $.items length"]);
        let r = run(&x, StatusMatch::Any, vec![], r#"{"amount": 1}"#, &s);
        assert!(!by_name(&r)["expect body $.amount"]);
    }

    #[test]
    fn body_variables_and_text_bodies() {
        let mut s = VarScope::default();
        s.set_literal("cid", "c1");
        let x = exchange(200, &[], r#"{"customer":"c1"}"#);
        let r = run(
            &x,
            StatusMatch::Any,
            vec![],
            r#"{"customer":"{{cid}}"}"#,
            &s,
        );
        assert!(by_name(&r)["expect body $.customer"]);

        let x = exchange(200, &[], "  hello world \n");
        assert!(
            run(&x, StatusMatch::Any, vec![], "hello world", &s)
                .last()
                .unwrap()
                .passed
        );
        assert!(
            !run(&x, StatusMatch::Any, vec![], "hello", &s)
                .last()
                .unwrap()
                .passed
        );
        // JSON expected against a non-JSON body fails with a message, not a panic.
        let r = run(&x, StatusMatch::Any, vec![], r#"{"a":1}"#, &s);
        assert!(!r.last().unwrap().passed && r.last().unwrap().message.is_some());
    }
}
