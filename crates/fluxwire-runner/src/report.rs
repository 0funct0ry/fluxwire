//! Serializers for folder-run results: JSON and JUnit XML (shared with the CLI reporters).
//! Secret values are replaced with `‹secret:name›` in everything written.

use serde_json::{Value, json};

use crate::{FolderRunResult, RequestRun, RequestStatus};

fn status_name(s: RequestStatus) -> &'static str {
    match s {
        RequestStatus::Passed => "passed",
        RequestStatus::Failed => "failed",
        RequestStatus::Skipped => "skipped",
    }
}

fn error_text(r: &RequestRun) -> Option<String> {
    let res = &r.result;
    let mut parts = Vec::new();
    if let Some(e) = &res.error {
        parts.push(e.describe().0);
    }
    for s in &res.script_errors {
        parts.push(format!("{} ({})", s.message, s.location()));
    }
    (!parts.is_empty()).then(|| res.redact(&parts.join("\n")))
}

fn request_json(r: &RequestRun) -> Value {
    let res = &r.result;
    let status = res
        .outcome
        .as_ref()
        .and_then(|o| o.exchange.as_ref())
        .map(|e| e.status);
    json!({
        "iteration": r.iteration,
        "key": r.request.key,
        "name": r.request.name,
        "method": r.request.method,
        "folder": r.request.folder,
        "status": status_name(r.status()),
        "http_status": status,
        "started_ms": r.started_ms,
        "duration_ms": r.duration_ms,
        "error": error_text(r),
        "skipped": res.skipped,
        "tests": res.tests.iter().map(|t| json!({
            "name": res.redact(&t.name),
            "passed": t.passed,
            "expected": t.expected.as_ref().map(|s| res.redact(s)),
            "actual": t.actual.as_ref().map(|s| res.redact(s)),
            "message": t.message.as_ref().map(|s| res.redact(s)),
        })).collect::<Vec<_>>(),
    })
}

/// Pretty-printed JSON for a finished run.
pub fn to_json(run: &FolderRunResult) -> String {
    let s = run.summary();
    let started = run
        .started_at
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let iterations: Vec<Value> = (0..run.iterations)
        .map(|i| {
            json!({
                "index": i,
                "row": run.rows.get(i as usize),
                "requests": run
                    .requests
                    .iter()
                    .filter(|r| r.iteration == i)
                    .map(|r| request_json(r))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let v = json!({
        "run_id": run.run_id,
        "folder": run.folder,
        "started_at_ms": started,
        "total_ms": run.total_ms,
        "cancelled": run.cancelled,
        "timed_out": run.timed_out,
        "bailed": run.bailed,
        "error": run.error,
        "summary": {
            "requests": s.requests,
            "passed": s.passed,
            "failed": s.failed,
            "skipped": s.skipped,
            "tests": s.tests,
            "tests_failed": s.tests_failed,
        },
        "iterations": iterations,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

fn secs(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// JUnit XML: one `testsuite` per folder, one `testcase` per test (or per request when it
/// has none), so CI systems show requests and assertions alike.
pub fn to_junit(run: &FolderRunResult) -> String {
    let mut folders: Vec<&str> = Vec::new();
    for r in &run.requests {
        if !folders.contains(&r.request.folder.as_str()) {
            folders.push(&r.request.folder);
        }
    }
    let s = run.summary();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\" time=\"{}\">\n",
        esc(if run.folder.is_empty() {
            "collection"
        } else {
            &run.folder
        }),
        s.requests,
        s.failed,
        s.skipped,
        secs(run.total_ms),
    ));
    for f in folders {
        let reqs: Vec<&std::sync::Arc<RequestRun>> = run
            .requests
            .iter()
            .filter(|r| r.request.folder == f)
            .collect();
        let failures = reqs
            .iter()
            .filter(|r| r.status() == RequestStatus::Failed)
            .count();
        let skipped = reqs
            .iter()
            .filter(|r| r.status() == RequestStatus::Skipped)
            .count();
        let time: u64 = reqs.iter().map(|r| r.duration_ms).sum();
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\" time=\"{}\">\n",
            esc(if f.is_empty() { "." } else { f }),
            reqs.len(),
            secs(time),
        ));
        for r in reqs {
            let res = &r.result;
            let class = esc(&r.request.key);
            let iter = if run.iterations > 1 {
                format!(" [#{}]", r.iteration + 1)
            } else {
                String::new()
            };
            let name = format!("{} {}{iter}", r.request.method, r.request.name);
            out.push_str(&format!(
                "    <testcase classname=\"{class}\" name=\"{}\" time=\"{}\"",
                esc(&name),
                secs(r.duration_ms)
            ));
            match r.status() {
                RequestStatus::Skipped => {
                    let why = res.skipped.clone().unwrap_or_else(|| "cancelled".into());
                    out.push_str(&format!(
                        ">\n      <skipped message=\"{}\"/>\n    </testcase>\n",
                        esc(&why)
                    ));
                    continue;
                }
                RequestStatus::Failed if res.tests.iter().all(|t| t.passed) => {
                    let msg = error_text(r).unwrap_or_else(|| "request failed".into());
                    out.push_str(&format!(
                        ">\n      <failure message=\"{}\">{}</failure>\n    </testcase>\n",
                        esc(msg.lines().next().unwrap_or("")),
                        esc(&msg)
                    ));
                    continue;
                }
                _ => {}
            }
            if res.tests.is_empty() {
                out.push_str("/>\n");
                continue;
            }
            out.push_str(">\n");
            for t in &res.tests {
                if t.passed {
                    continue;
                }
                let detail = format!(
                    "{}{}{}",
                    t.message.clone().unwrap_or_default(),
                    t.expected
                        .as_ref()
                        .map(|e| format!("\nexpected: {e}"))
                        .unwrap_or_default(),
                    t.actual
                        .as_ref()
                        .map(|a| format!("\nactual: {a}"))
                        .unwrap_or_default(),
                );
                let detail = res.redact(detail.trim());
                out.push_str(&format!(
                    "      <failure message=\"{}\">{}</failure>\n",
                    esc(&res.redact(&t.name)),
                    esc(&detail)
                ));
            }
            out.push_str("    </testcase>\n");
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}
