//! Copy as cURL and paste a `curl` command into a request (SPEC §9.5).

use fluxwire_model::{Edit, FluxDoc};

use crate::auth::Auth;
use crate::plan::HttpPlan;

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

const MASK: &str = "***";

pub fn to_curl(p: &HttpPlan, mask: bool) -> String {
    let out = to_curl_raw(p, mask);
    if !mask {
        return out;
    }
    // Secret variables can land anywhere (URL, headers, body): redact by value.
    let mut secrets: Vec<&(String, String)> =
        p.secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
    secrets.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = out;
    for (name, value) in secrets {
        let tag = format!("‹secret:{name}›");
        out = out.replace(value.as_str(), &tag);
        let enc: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        out = out.replace(&enc, &tag);
    }
    out
}

fn to_curl_raw(p: &HttpPlan, mask: bool) -> String {
    let mut url = p.url.clone();
    if let Auth::ApiKey {
        key,
        value,
        in_query: true,
    } = &p.auth
    {
        let v = if mask { MASK } else { value.as_str() };
        url.query_pairs_mut().append_pair(key, v);
    }
    let mut parts = vec!["curl".to_string()];
    if p.method != "GET" {
        parts.push(format!("-X {}", p.method));
    }
    parts.push(quote(url.as_str()));
    let secret_header = match &p.auth {
        Auth::ApiKey {
            key,
            in_query: false,
            ..
        } => Some(key.to_ascii_lowercase()),
        _ => None,
    };
    let mut headers = p.headers.clone();
    if let Some((k, v)) = p.auth.header()
        && !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(&k))
    {
        headers.push((k, v));
    }
    for (k, v) in &headers {
        let lk = k.to_ascii_lowercase();
        let secret = lk == "authorization"
            || lk == "proxy-authorization"
            || secret_header.as_deref() == Some(lk.as_str());
        let v = if mask && secret {
            match v.split_once(' ') {
                Some((scheme, _)) if lk.ends_with("authorization") => format!("{scheme} {MASK}"),
                _ => MASK.to_string(),
            }
        } else {
            v.clone()
        };
        parts.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    if !p.body.is_empty() {
        match std::str::from_utf8(&p.body) {
            Ok(s) => parts.push(format!("--data-raw {}", quote(s))),
            Err(_) => parts.push(format!(
                "--data-binary {}",
                quote(&format!("<{} bytes of binary data>", p.body.len()))
            )),
        }
    }
    if let Auth::Digest { username, password } = &p.auth {
        let pw = if mask { MASK } else { password.as_str() };
        parts.push(format!(
            "--digest -u {}",
            quote(&format!("{username}:{pw}"))
        ));
    }
    parts.join(" \\\n  ")
}

/// Split a shell command line into words (quotes, escapes, line continuations).
pub fn tokenize(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut any = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('\n') => {}
                Some('\r') => {
                    if it.peek() == Some(&'\n') {
                        it.next();
                    }
                }
                Some(n) => {
                    cur.push(n);
                    any = true;
                }
                None => {}
            },
            '\'' => {
                any = true;
                loop {
                    match it.next()? {
                        '\'' => break,
                        ch => cur.push(ch),
                    }
                }
            }
            '"' => {
                any = true;
                loop {
                    match it.next()? {
                        '"' => break,
                        '\\' => match it.next()? {
                            n @ ('"' | '\\' | '$' | '`') => cur.push(n),
                            '\n' => {}
                            n => {
                                cur.push('\\');
                                cur.push(n);
                            }
                        },
                        ch => cur.push(ch),
                    }
                }
            }
            c if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    Some(out)
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

pub fn parse_curl(text: &str) -> Option<Parsed> {
    parse_curl_ex(text).map(|(p, _)| p)
}

/// Flags that take a value and have no effect on a request file; they are reported as warnings.
const IGNORED_WITH_VALUE: &[&str] = &[
    "-o",
    "--output",
    "-x",
    "--proxy",
    "-m",
    "--max-time",
    "--connect-timeout",
    "--retry",
    "-w",
    "--write-out",
];

/// Like [`parse_curl`], also returning warnings for ignored flags and extra URLs.
pub fn parse_curl_ex(text: &str) -> Option<(Parsed, Vec<String>)> {
    let mut warnings: Vec<String> = Vec::new();
    let words = tokenize(text.trim())?;
    let mut it = words.into_iter();
    if it.next()? != "curl" {
        return None;
    }
    let mut p = Parsed::default();
    let mut data: Vec<String> = Vec::new();
    let mut get = false;
    let mut head = false;
    let mut json = false;
    let mut args: Vec<String> = it.collect();
    args.reverse();
    while let Some(a) = args.pop() {
        // `-sSL` style bundles of argument-less flags
        if a.len() > 2
            && a.starts_with('-')
            && !a.starts_with("--")
            && a[1..].chars().all(|c| "sSLkvifGIi".contains(c))
        {
            args.extend(a[1..].chars().rev().map(|c| format!("-{c}")));
            continue;
        }
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let value = |args: &mut Vec<String>| inline.clone().or_else(|| args.pop());
        match flag.as_str() {
            "-X" | "--request" => p.method = value(&mut args)?.to_ascii_uppercase(),
            "-H" | "--header" => {
                let h = value(&mut args)?;
                let (k, v) = h.split_once(':')?;
                p.headers.push((k.trim().to_string(), v.trim().to_string()));
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii"
            | "--data-urlencode" => {
                let d = value(&mut args)?;
                if d.starts_with('@') {
                    return None; // `-d @file` is not supported
                }
                data.push(d)
            }
            "--json" => {
                let d = value(&mut args)?;
                if d.starts_with('@') {
                    return None; // `--json @file` is not supported
                }
                json = true;
                data.push(d)
            }
            "-u" | "--user" => {
                use base64::Engine as _;
                let u = value(&mut args)?;
                p.headers.push((
                    "Authorization".into(),
                    format!(
                        "Basic {}",
                        base64::engine::general_purpose::STANDARD.encode(u)
                    ),
                ));
            }
            "-A" | "--user-agent" => p.headers.push(("User-Agent".into(), value(&mut args)?)),
            "-b" | "--cookie" => p.headers.push(("Cookie".into(), value(&mut args)?)),
            "-e" | "--referer" => p.headers.push(("Referer".into(), value(&mut args)?)),
            "--url" => p.url = value(&mut args)?,
            "-G" | "--get" => get = true,
            "-I" | "--head" => head = true,
            "-L" | "--location" | "-k" | "--insecure" | "--compressed" | "-s" | "--silent"
            | "-S" | "--show-error" | "-v" | "--verbose" | "-i" | "--include" | "-f" | "--fail"
            | "--http1.1" | "--http2" => {}
            f if IGNORED_WITH_VALUE.contains(&f) => {
                value(&mut args)?;
                warnings.push(format!("Ignored {f}"));
            }
            f if f.starts_with('-') && f.len() > 1 => return None, // unknown flag: refuse rather than guess
            _ => {
                if p.url.is_empty() {
                    p.url = a;
                } else {
                    warnings.push(format!("Only the first URL is used; ignored {a}"));
                }
            }
        }
    }
    if p.url.is_empty() {
        return None;
    }
    if json {
        // `--json` implies both headers unless the command set them explicitly
        for (name, value) in [
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ] {
            if !p.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
                p.headers.push((name.into(), value.into()));
            }
        }
    }
    if !data.is_empty() {
        let joined = data.join("&");
        if get {
            p.url.push(if p.url.contains('?') { '&' } else { '?' });
            p.url.push_str(&joined);
        } else {
            p.body = Some(joined);
        }
    }
    if p.method.is_empty() {
        p.method = if head {
            "HEAD".into()
        } else if p.body.is_some() {
            "POST".into()
        } else {
            "GET".into()
        };
    }
    Some((p, warnings))
}

/// What a pasted cURL command will become, for the New request dialog.
pub fn preview_curl(text: &str) -> Option<fluxwire_protocol::PasteSummary> {
    let (p, mut warnings) = parse_curl_ex(text)?;
    let has = |name: &str| p.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if has("authorization") || has("cookie") {
        warnings.push("Contains credentials; consider moving to an environment variable".into());
    }
    let body = p.body.as_ref().map(|b| {
        let ct = p
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default();
        let kind = if ct.contains("json") {
            "JSON"
        } else if ct.contains("x-www-form-urlencoded") {
            "form"
        } else {
            "text"
        };
        (kind.to_string(), b.len())
    });
    Some(fluxwire_protocol::PasteSummary {
        method: p.method,
        url: p.url,
        header_count: p.headers.len(),
        body,
        warnings,
    })
}

/// Replace the request block's content with the pasted command.
pub fn edits_from_paste(text: &str, doc: &FluxDoc) -> Option<Vec<Edit>> {
    let parsed = parse_curl(text)?;
    let i = doc.request_index()?;
    let nl = if doc.source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = format!("{} {}{nl}", parsed.method, parsed.url);
    for (k, v) in &parsed.headers {
        out.push_str(&fluxwire_format::header_line(true, k, v));
        out.push_str(nl);
    }
    if let Some(b) = &parsed.body {
        out.push_str(nl);
        out.push_str(b.trim_end_matches(['\n', '\r']));
        out.push_str(nl);
    }
    Some(vec![Edit::replace(doc.blocks[i].content, out)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_quotes_and_continuations() {
        let t = tokenize("curl -X POST 'http://a/b c' \\\n -H \"X: y\\\"z\"").unwrap();
        assert_eq!(t, ["curl", "-X", "POST", "http://a/b c", "-H", "X: y\"z"]);
    }

    #[test]
    fn json_flag_sets_body_and_headers() {
        let p = parse_curl(
            "curl -X POST https://x.test/v1 -H 'Authorization: Bearer t' --json '{\"a\": 1}'",
        )
        .unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.body.as_deref(), Some("{\"a\": 1}"));
        assert_eq!(
            p.headers,
            [
                ("Authorization".into(), "Bearer t".into()),
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json".into()),
            ]
        );
    }

    #[test]
    fn json_flag_keeps_explicit_headers() {
        let p = parse_curl(
            "curl https://x.test -H 'content-type: application/vnd.api+json' -H 'Accept: */*' --json '{}'",
        )
        .unwrap();
        assert_eq!(p.headers.len(), 2);
        assert_eq!(p.method, "POST");
    }

    #[test]
    fn parses_common_flags() {
        let p = parse_curl(
            "curl -sSL https://x.test/v1/p -H 'Content-Type: application/json' -d '{\"a\":1}' --compressed",
        )
        .unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.url, "https://x.test/v1/p");
        assert_eq!(
            p.headers,
            [("Content-Type".into(), "application/json".into())]
        );
        assert_eq!(p.body.as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn unknown_flags_and_non_curl_are_refused() {
        assert!(parse_curl("curl --weird-flag http://x").is_none());
        assert!(parse_curl("echo hi").is_none());
        assert!(parse_curl("curl http://x -d @body.json").is_none());
    }

    #[test]
    fn preview_summarizes_and_warns() {
        let s = preview_curl(
            "curl -X POST https://x.test/v1/p -H 'Content-Type: application/json' -d '{\"a\":1}'",
        )
        .unwrap();
        assert_eq!(
            (s.method.as_str(), s.url.as_str(), s.header_count),
            ("POST", "https://x.test/v1/p", 1)
        );
        assert_eq!(s.body, Some(("JSON".to_string(), 7)));
        assert!(s.warnings.is_empty());
        let w = preview_curl("curl http://a http://b -o out -H 'Authorization: x'").unwrap();
        assert_eq!(w.warnings.len(), 3);
        assert!(w.warnings[0].contains("http://b"));
        assert!(preview_curl("curl --proxy").is_none());
    }

    #[test]
    fn pasting_replaces_only_the_request_block_content() {
        let src =
            "---\nname: A\n---\n\nProse stays.\n\n```http\nGET http://old\n```\n\nMore prose.\n";
        let doc = fluxwire_format::parse(src);
        let edits = edits_from_paste("curl -u a:b http://new.test/x -X PUT", &doc).unwrap();
        let out = fluxwire_model::apply_edits(src, &edits).unwrap();
        assert!(
            out.contains("PUT http://new.test/x\nAuthorization: Basic YTpi\n```"),
            "{out}"
        );
        assert!(out.starts_with("---\nname: A\n---\n\nProse stays.\n\n```http\n"));
        assert!(out.ends_with("```\n\nMore prose.\n"));
    }
}
