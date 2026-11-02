//! Script scanning for the `pm` shim (SPEC §11.4) and `.http` `client.*` handlers.

use crate::plan::ImportReport;

const PM_TOP: &[&str] = &[
    "environment",
    "variables",
    "collectionVariables",
    "globals",
    "info",
    "test",
    "expect",
    "sendRequest",
    "request",
    "response",
];
const PM_VARS: &[&str] = &["get", "set", "unset", "has", "toObject", "name"];
const PM_RESPONSE: &[&str] = &[
    "code",
    "status",
    "responseTime",
    "responseSize",
    "headers",
    "json",
    "text",
    "to",
];
const PM_REQUEST: &[&str] = &["url", "method", "headers", "body"];

fn ident_after(code: &str, at: usize) -> &str {
    let rest = &code[at..];
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(rest.len());
    &rest[..end]
}

/// All `prefix.member` chains in `code`, as (member, next member).
fn chains<'a>(code: &'a str, prefix: &str) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    let needle = format!("{prefix}.");
    let mut from = 0;
    while let Some(i) = code[from..].find(&needle) {
        let at = from + i;
        let before_ok = at == 0
            || !code[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$');
        let start = at + needle.len();
        from = start;
        if !before_ok {
            continue;
        }
        let member = ident_after(code, start);
        if member.is_empty() {
            continue;
        }
        let after = start + member.len();
        let next = if code[after..].starts_with('.') {
            ident_after(code, after + 1)
        } else {
            ""
        };
        out.push((member, next));
    }
    out
}

/// Reports unsupported `pm.*` calls, legacy `postman.*` and `require()` in a Postman script.
pub fn scan_pm(code: &str, file: &str, report: &mut ImportReport) {
    for (m, next) in chains(code, "pm") {
        if !PM_TOP.contains(&m) {
            report.lossy(
                file,
                format!("pm.{m}"),
                "Not supported by the pm shim; the call throws `Unsupported in Fluxwire` when run",
            );
            continue;
        }
        let bad = match m {
            "environment" | "variables" | "collectionVariables" | "globals" => {
                !next.is_empty() && !PM_VARS.contains(&next)
            }
            "response" => !next.is_empty() && !PM_RESPONSE.contains(&next),
            "request" => !next.is_empty() && !PM_REQUEST.contains(&next),
            _ => false,
        };
        if bad {
            report.lossy(
                file,
                format!("pm.{m}.{next}"),
                "Not supported by the pm shim; the call throws `Unsupported in Fluxwire` when run",
            );
        }
    }
    if !chains(code, "postman").is_empty() {
        report.lossy(
            file,
            "postman.* legacy API",
            "The legacy `postman` object is not available; rewrite with `pm.*` or `fw.*`",
        );
    }
    let mut from = 0;
    while let Some(i) = code[from..].find("require(") {
        let at = from + i + "require(".len();
        from = at;
        let arg: String = code[at..]
            .trim_start()
            .trim_start_matches(['\'', '"'])
            .chars()
            .take_while(|c| !matches!(c, '\'' | '"' | ')'))
            .collect();
        let hint = match arg.as_str() {
            "crypto-js" => " (use `fw.crypto.sha256/sha512/hmac` and `btoa/atob`)",
            "moment" => " (use the built-in `Date`)",
            _ => "",
        };
        report.lossy(
            file,
            format!("require('{arg}')"),
            format!("npm packages are not available{hint}; import from lib/ instead"),
        );
    }
}

const CLIENT_MEMBERS: &[&str] = &["global", "test", "assert", "log"];
const RESPONSE_MEMBERS: &[&str] = &["status", "body", "headers", "contentType"];

/// Converts a `.http` response handler body to a `ts test` script and reports leftovers.
pub fn convert_http_handler(code: &str, file: &str, report: &mut ImportReport) -> String {
    for (m, _) in chains(code, "client") {
        if !CLIENT_MEMBERS.contains(&m) {
            report.lossy(
                file,
                format!("client.{m}"),
                "Response handler API not available in Fluxwire",
            );
        }
    }
    for (m, _) in chains(code, "response") {
        if !RESPONSE_MEMBERS.contains(&m) {
            report.lossy(
                file,
                format!("response.{m}"),
                "Response handler API not available in Fluxwire",
            );
        }
    }
    let mut out = String::from(
        "const client = {\n  global: { set: (k, v) => fw.vars.set(k, v), get: (k) => fw.vars.get(k) },\n  test: (name, fn) => fw.test(name, fn),\n  assert: (cond, msg) => { if (!cond) throw new Error(msg ?? \"assertion failed\"); },\n  log: (...a) => console.log(...a),\n};\nconst response = {\n  get status() { return fw.response.status; },\n  get body() { return fw.response.json(); },\n  headers: { valueOf: (n) => fw.response.headers.get(n) },\n  contentType: { get mimeType() { return (fw.response.headers.get(\"content-type\") ?? \"\").split(\";\")[0]; } },\n};\n",
    );
    out.push_str(&dedent(code));
    out
}

fn dedent(code: &str) -> String {
    let code = code.trim_matches('\n');
    let indent = code
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    code.lines()
        .map(|l| l.get(indent..).unwrap_or(l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
