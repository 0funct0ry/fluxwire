//! Execute-class engine tools: send, run a folder, cancel. Results are redacted; script trust
//! follows `trust.toml` and is never written (ADR-009).

use super::record::{Meta, record};
use super::{Engine, Out, args, existing, mask_header, object, scrub, workspace_of};
use crate::ToolClass;
use crate::registry::Registry;
use crate::tools::def_in;
use fluxwire_protocol::Body;
use fluxwire_runner::prepare::{FolderOpts, Prep, collection_root_of, has_scripts};
use fluxwire_runner::{FolderRunEvent, RunEvent, RunResult, runner};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const BODY_CAP: usize = 64 * 1024;

#[derive(Deserialize, JsonSchema)]
struct SendIn {
    /// The request file.
    path: String,
    /// Environment name.
    env: Option<String>,
    /// Name to pass to `cancel` while the send is running.
    cancel_key: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RunIn {
    /// A folder inside a Collection, or the Collection root.
    path: String,
    env: Option<String>,
    /// One request at a time, in order. Default: concurrent.
    sequential: Option<bool>,
    concurrency: Option<u32>,
    iterations: Option<u32>,
    /// JSON or CSV data file; one iteration per row.
    data_file: Option<String>,
    /// Matches request names, paths or methods.
    filter: Option<String>,
    /// Collection-relative request paths to run.
    only: Option<Vec<String>>,
    bail: Option<bool>,
    delay_ms: Option<u64>,
    timeout_ms: Option<u64>,
    /// Name to pass to `cancel` while the run is going. Default: the run id.
    cancel_key: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CancelIn {
    /// The `cancel_key` (or run id) of a send or run in progress.
    key: String,
}

fn scripts_status(present: bool, trusted: bool) -> &'static str {
    match (present, trusted) {
        (false, _) => "none",
        (true, true) => "run",
        (true, false) => {
            "not run: the Collection is not trusted (trust.toml), so the request was not sent"
        }
    }
}

fn body_text(b: &Body) -> (String, bool) {
    match b {
        Body::Memory(bytes) => {
            let cut = bytes.len().min(BODY_CAP);
            (
                String::from_utf8_lossy(&bytes[..cut]).into_owned(),
                bytes.len() > BODY_CAP,
            )
        }
        Body::File { len, .. } => (
            format!("(body of {len} bytes kept in a temporary file)"),
            true,
        ),
    }
}

fn send_json(res: &RunResult, scripts: &str) -> Value {
    let o = res.outcome.as_deref();
    let x = o.and_then(|o| o.exchange.as_ref());
    let headers: Vec<Value> = x.map_or_else(Vec::new, |x| {
        x.headers
            .iter()
            .map(|(k, v)| json!({"name": k, "value": mask_header(k, v)}))
            .collect()
    });
    let (body, truncated) = x.map_or((String::new(), false), |x| body_text(&x.body));
    let error = res.error.as_ref().map(|e| {
        let (message, fix) = e.describe();
        json!({"phase": e.phase(), "message": message, "fix": fix})
    });
    json!({
        "passed": res.passed(),
        "scripts": scripts,
        "status": x.map(|x| x.status),
        "status_text": x.map(|x| x.status_text.clone()),
        "version": x.map(|x| x.version.clone()),
        "headers": headers,
        "content_type": x.and_then(|x| x.content_type.clone()),
        "body": body,
        "body_truncated": truncated,
        "final_url": o.map(|o| o.final_url.clone()),
        "total_ms": o.map(|o| o.total_ms),
        "tests": res.tests.iter().map(|t| json!({
            "name": t.name, "passed": t.passed, "expected": t.expected,
            "actual": t.actual, "message": t.message,
        })).collect::<Vec<_>>(),
        "script_errors": res.script_errors.iter()
            .map(|s| format!("{} ({})", s.message, s.location())).collect::<Vec<_>>(),
        "console": res.console.iter().map(|c| c.text.clone()).collect::<Vec<_>>(),
        "warnings": res.warnings.iter().map(|d| d.message.clone()).collect::<Vec<_>>(),
        "skipped": res.skipped,
        "error": error,
    })
}

fn collection_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn register(r: &mut Registry, e: Arc<Engine>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let eng = e.clone();
    r.register(def_in::<SendIn, Out>(
        ToolClass::Execute,
        "send_request",
        "Send one request from its file on disk and return the redacted response, tests, script errors and console. Scripts run only in a Collection trusted in trust.toml; this tool never trusts one.",
        move |ctx, a| {
            let a: SendIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let root = collection_root_of(&abs).ok_or("not inside a Collection (no fluxwire.toml)")?;
            let ws = workspace_of(ctx, &abs).ok_or("outside the allowed roots")?;
            let src = std::fs::read_to_string(&abs).map_err(|e| e.to_string())?;
            let doc = fluxwire_format::parse(&src);
            if doc.is_websocket() {
                return Err("WebSocket requests open sessions and are not supported by send_request".into());
            }
            let prep = Prep::new(eng.config_dir.clone(), a.env.clone(), Some(ws.clone()));
            let trusted = prep.trusted(&root);
            let scripts = scripts_status(has_scripts(&doc, Some(&root), &abs), trusted);
            let handle = runner().send(&doc, prep.send_options(&doc, &abs, &root));
            let key = a
                .cancel_key
                .unwrap_or_else(|| format!("send-{}", SEQ.fetch_add(1, Ordering::Relaxed)));
            let token = handle.canceller();
            eng.track(key.clone(), move || token.cancel());
            let mut rx = handle.events;
            let mut finished = None;
            while let Some(ev) = rx.blocking_recv() {
                if let RunEvent::Finished(res) = ev {
                    finished = Some(res);
                    break;
                }
            }
            eng.untrack(&key);
            let res = finished.ok_or("the send ended without a result")?;
            let rel = fluxwire_format::rel_key(&root, &abs);
            record(
                &eng,
                &res,
                Meta {
                    workspace: &ws,
                    collection: &collection_name(&root),
                    request_path: &rel,
                    request_name: doc.name().unwrap_or(&rel),
                    method: None,
                    env: a.env.as_deref(),
                    run_id: None,
                },
            );
            let mut v = send_json(&res, scripts);
            scrub(&res.secrets, &mut v);
            object(v)
        },
    ));

    let eng = e.clone();
    r.register(def_in::<RunIn, Out>(
        ToolClass::Execute,
        "run_folder",
        "Run a folder or Collection (SPEC §12) and return the run report (the same JSON as `report::to_json`), redacted. Scripts run only in a Collection trusted in trust.toml.",
        move |ctx, a| {
            let a: RunIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let root = collection_root_of(&abs).ok_or("not inside a Collection (no fluxwire.toml)")?;
            let ws = workspace_of(ctx, &abs).ok_or("outside the allowed roots")?;
            let rel = abs
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let data = match &a.data_file {
                Some(p) => fluxwire_runner::load_data_file(&existing(ctx, p)?)?,
                None => Vec::new(),
            };
            let prep = Prep::new(eng.config_dir.clone(), a.env.clone(), Some(ws.clone()));
            let trusted = prep.trusted(&root);
            let plan = prep.run_plan(
                &root,
                &rel,
                FolderOpts {
                    sequential: a.sequential.unwrap_or(false),
                    concurrency: a.concurrency,
                    iterations: a.iterations,
                    data,
                    filter: a.filter,
                    only: a.only,
                    bail: a.bail.unwrap_or(false),
                    delay_ms: a.delay_ms.unwrap_or(0),
                    timeout_ms: a.timeout_ms,
                },
            );
            let handle = runner().run_folder(plan);
            let run_id = handle.run_id.clone();
            let key = a.cancel_key.unwrap_or_else(|| run_id.clone());
            let token = handle.canceller();
            eng.track(key.clone(), move || token.cancel());
            let mut rx = handle.events;
            let mut finished = None;
            while let Some(ev) = rx.blocking_recv() {
                if let FolderRunEvent::Finished(res) = ev {
                    finished = Some(res);
                    break;
                }
            }
            eng.untrack(&key);
            let res = finished.ok_or("the run ended without a result")?;
            let cname = collection_name(&root);
            for r in &res.requests {
                record(
                    &eng,
                    &r.result,
                    Meta {
                        workspace: &ws,
                        collection: &cname,
                        request_path: &r.request.key,
                        request_name: &r.request.name,
                        method: Some(&r.request.method),
                        env: a.env.as_deref(),
                        run_id: Some(&run_id),
                    },
                );
            }
            let mut report: Value =
                serde_json::from_str(&fluxwire_runner::to_json(&res)).map_err(|e| e.to_string())?;
            let secrets: Vec<(String, String)> = res
                .requests
                .iter()
                .flat_map(|r| r.result.secrets.clone())
                .collect();
            scrub(&secrets, &mut report);
            object(json!({
                "run_id": run_id,
                "scripts": if trusted { "run" } else { "not run: the Collection is not trusted (trust.toml); requests with scripts fail with a trust error" },
                "report": report,
            }))
        },
    ));

    let eng = e;
    r.register(def_in::<CancelIn, Out>(
        ToolClass::Execute,
        "cancel",
        "Cancel a send or folder run in progress by its cancel_key (or run id).",
        move |_, a| {
            let a: CancelIn = args(a)?;
            object(json!({ "cancelled": eng.cancel(&a.key) }))
        },
    ));
}
