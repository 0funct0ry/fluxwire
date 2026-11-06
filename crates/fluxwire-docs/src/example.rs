//! The `http example` block saved from a response (SPEC §16).

/// Headers worth documenting. Everything else (credentials, CORS, tracing and per-send
/// values such as `Date` or `X-Request-Id`) is left out of an example.
const KEPT: &[&str] = &[
    "content-type",
    "location",
    "link",
    "etag",
    "cache-control",
    "content-language",
    "retry-after",
    "www-authenticate",
    "allow",
];

/// Header name as written in docs: `content-type` becomes `Content-Type`.
fn canonical(name: &str) -> String {
    name.split('-')
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Block content for an `http example` block: the status line, headers and body of a
/// response, with `redact` applied to every header value and the body.
pub fn example_content(
    status: u16,
    headers: &[(String, String)],
    body: &str,
    redact: &dyn Fn(&str) -> String,
) -> String {
    let mut out = format!("HTTP {status}\n");
    for (k, v) in headers {
        if !KEPT.iter().any(|s| k.eq_ignore_ascii_case(s)) {
            continue;
        }
        out.push_str(&format!("{}: {}\n", canonical(k), redact(v)));
    }
    let body = redact(body);
    let body = body.trim_end_matches(['\n', '\r']);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_content_headers_and_redacts_secrets() {
        let h = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("Set-Cookie".to_string(), "sid=abc".to_string()),
            ("access-control-allow-origin".to_string(), "*".to_string()),
            ("x-request-id".to_string(), "abc".to_string()),
            ("Location".to_string(), "/p/s3cret".to_string()),
        ];
        let r = |s: &str| s.replace("s3cret", "‹secret:token›");
        let t = example_content(201, &h, "{\"k\": \"s3cret\"}\n", &r);
        assert_eq!(
            t,
            "HTTP 201\nContent-Type: application/json\nLocation: /p/‹secret:token›\n\n{\"k\": \"‹secret:token›\"}"
        );
    }

    #[test]
    fn empty_body_has_no_blank_line() {
        assert_eq!(
            example_content(204, &[], "", &|s| s.to_string()),
            "HTTP 204\n"
        );
    }

    #[test]
    fn inserted_example_roundtrips_through_the_parser() {
        use fluxwire_model::BlockKind;
        let doc = fluxwire_format::parse("---\nname: A\n---\n\n```http\nGET /x\n```\n");
        let body = example_content(200, &[], "{}", &|s| s.to_string());
        let edits = fluxwire_format::insert_block(&doc, BlockKind::Example, "http example", &body);
        let src = fluxwire_model::apply_edits(&doc.source, &edits).unwrap();
        let d2 = fluxwire_format::parse(&src);
        assert!(d2.blocks.iter().any(|b| b.kind == BlockKind::Example));
        assert!(
            d2.diagnostics
                .iter()
                .all(|d| !format!("{d:?}").contains("Error"))
        );
    }
}
