//! Query parameters of a request target (SPEC §7.6). A parameter can be written in the target
//! (`/a?x=1&y=2`) or as an indented continuation line; the Params table shows both, URL ones
//! first. Everything here is byte-preserving: values are never decoded, and a `{{var}}`
//! reference is opaque, so a `?`, `&`, `=` or `#` inside one does not split anything.

/// One parameter: name and optional value, exactly as written.
pub type Pair = (String, Option<String>);

/// Byte offset of the first `ch` (ASCII) outside any `{{ … }}` reference.
fn find_top(s: &str, ch: char) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0usize, 0usize);
    while i < b.len() {
        if b[i..].starts_with(b"{{") {
            depth += 1;
            i += 2;
        } else if depth > 0 && b[i..].starts_with(b"}}") {
            depth -= 1;
            i += 2;
        } else if depth == 0 && b[i] == ch as u8 {
            return Some(i);
        } else {
            i += 1;
        }
    }
    None
}

fn split_top(s: &str, ch: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = find_top(rest, ch) {
        out.push(&rest[..i]);
        rest = &rest[i + 1..];
    }
    out.push(rest);
    out
}

/// `(base, query string, fragment)`; the fragment keeps its `#`. No `?` means an empty query.
fn split_target(target: &str) -> (&str, &str, &str) {
    let (head, frag) = match find_top(target, '#') {
        Some(i) => target.split_at(i),
        None => (target, ""),
    };
    match find_top(head, '?') {
        Some(i) => (&head[..i], &head[i + 1..], frag),
        None => (head, "", frag),
    }
}

fn parse_pairs(qs: &str) -> Vec<Pair> {
    split_top(qs, '&')
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|seg| match find_top(seg, '=') {
            Some(i) => (seg[..i].to_string(), Some(seg[i + 1..].to_string())),
            None => (seg.to_string(), None),
        })
        .collect()
}

/// The parameters written in the target, in order.
pub fn url_query(target: &str) -> Vec<Pair> {
    parse_pairs(split_target(target).1)
}

fn join(base: &str, pairs: &[Pair], frag: &str) -> String {
    let mut out = base.to_string();
    for (i, (n, v)) in pairs.iter().enumerate() {
        out.push(if i == 0 { '?' } else { '&' });
        out.push_str(n);
        if let Some(v) = v {
            out.push('=');
            out.push_str(v);
        }
    }
    out.push_str(frag);
    out
}

/// `target` with URL parameter `idx` replaced by `with`, or removed when `with` is `None`.
/// Other parameters keep their exact text. `None` result: no such parameter.
pub fn rewrite_url_query(target: &str, idx: usize, with: Option<Pair>) -> Option<String> {
    let (base, qs, frag) = split_target(target);
    let mut pairs = parse_pairs(qs);
    if idx >= pairs.len() {
        return None;
    }
    match with {
        Some(p) => pairs[idx] = p,
        None => {
            pairs.remove(idx);
        }
    }
    Some(join(base, &pairs, frag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: &str, v: Option<&str>) -> Pair {
        (n.to_string(), v.map(str::to_string))
    }

    #[test]
    fn reads_pairs() {
        assert_eq!(
            url_query("{{BASE_URL}}/healthz?delay=100&fail=500"),
            [p("delay", Some("100")), p("fail", Some("500"))]
        );
        assert_eq!(
            url_query("/a?flag&x=&y=1=2"),
            [p("flag", None), p("x", Some("")), p("y", Some("1=2"))]
        );
        assert!(url_query("/a").is_empty());
        assert!(url_query("/a?").is_empty());
        assert!(url_query("/a#f?x=1").is_empty());
        assert_eq!(url_query("/a?x=1#top"), [p("x", Some("1"))]);
        assert_eq!(url_query("/a?x=1&&y=2").len(), 2);
    }

    #[test]
    fn variables_are_opaque() {
        assert_eq!(
            url_query("{{base}}/a?q={{a&b}}&t={{x?y}}"),
            [p("q", Some("{{a&b}}")), p("t", Some("{{x?y}}"))]
        );
        assert_eq!(url_query("{{u?x=1}}/a"), Vec::<Pair>::new());
    }

    #[test]
    fn rewrites_one_pair_and_keeps_the_rest() {
        let t = "/a?x=1&y=%20b&z#top";
        assert_eq!(
            rewrite_url_query(t, 0, Some(p("x", Some("9")))).unwrap(),
            "/a?x=9&y=%20b&z#top"
        );
        assert_eq!(rewrite_url_query(t, 1, None).unwrap(), "/a?x=1&z#top");
        assert_eq!(
            rewrite_url_query("/a?x=1", 0, None).unwrap(),
            "/a",
            "the `?` goes with the last parameter"
        );
        assert_eq!(rewrite_url_query(t, 3, None), None);
        assert_eq!(rewrite_url_query("/a", 0, None), None);
    }
}
