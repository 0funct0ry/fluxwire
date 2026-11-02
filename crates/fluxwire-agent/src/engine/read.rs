//! Read-class engine tools.

use super::{Engine, Out, args, checked, existing, object, workspace_of};
use crate::registry::Registry;
use crate::tools::def;
use fluxwire_format::{Collection, file_kind_for, load_config};
use fluxwire_model::{Diagnostic, FluxDoc, TreeNode};
use fluxwire_runner::prepare::{Prep, collection_root_of};
use fluxwire_store::{Filter, workspace_key};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize, JsonSchema)]
struct PathIn {
    /// A path inside an allowed root.
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct ResolveIn {
    /// The request file whose scope is used.
    path: String,
    /// Text containing `{{variables}}`.
    text: String,
    /// Environment name; none means no environment layer.
    env: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct HistoryIn {
    /// A path inside the Workspace whose history is queried.
    path: String,
    run_id: Option<String>,
    /// Status class: 2 for 2xx, 4 for 4xx, 5 for 5xx, 0 for no response.
    status_class: Option<u8>,
    /// Matches the request name or URL.
    text: Option<String>,
    /// `agent` for rows written through MCP.
    origin: Option<String>,
    /// Only failed sends (no response, 4xx, 5xx).
    failed: Option<bool>,
    /// Default 50, at most 500.
    limit: Option<u32>,
}

pub fn diagnostics_json(doc: &FluxDoc) -> Vec<Value> {
    doc.diagnostics.iter().map(diag_json).collect()
}

fn diag_json(d: &Diagnostic) -> Value {
    json!({
        "code": d.code,
        "severity": format!("{:?}", d.severity).to_lowercase(),
        "start": d.span.start,
        "end": d.span.end,
        "message": d.message,
        "fix": d.fix.is_some(),
    })
}

fn tree_json(nodes: &[TreeNode], out: &mut Vec<Value>) {
    for n in nodes {
        match n {
            TreeNode::Folder { children, .. } => tree_json(children, out),
            TreeNode::Request {
                name,
                path,
                method,
                kind,
            } => out.push(json!({
                "name": name, "path": path, "method": method,
                "kind": format!("{kind:?}").to_lowercase(),
            })),
            TreeNode::Doc { .. } => {}
        }
    }
}

fn collection_json(c: &Collection) -> Value {
    json!({
        "root": c.root.display().to_string(),
        "name": c.config.collection.name,
        "secret_names": c.config.secrets.names,
    })
}

pub fn register(r: &mut Registry, e: Arc<Engine>) {
    r.register(def::<PathIn, Out>(
        "list_collections",
        "Collections (folders with a fluxwire.toml) under a path, at most four levels deep.",
        |ctx, a| {
            let a: PathIn = args(a)?;
            let dir = existing(ctx, &a.path)?;
            let mut found = fluxwire_format::find_collections(&dir);
            if dir.join("fluxwire.toml").is_file() && !found.contains(&dir) {
                found.insert(0, dir);
            }
            let list: Vec<Value> = found
                .iter()
                .filter_map(|p| fluxwire_format::scan(p).ok())
                .map(|c| collection_json(&c))
                .collect();
            object(json!({ "collections": list }))
        },
    ));

    r.register(def::<PathIn, Out>(
        "list_requests",
        "Requests of a Collection (path to its root), with name, method and collection-relative path.",
        |ctx, a| {
            let a: PathIn = args(a)?;
            let dir = existing(ctx, &a.path)?;
            let c = fluxwire_format::scan(&dir).map_err(|e| format!("{e:?}"))?;
            let mut reqs = Vec::new();
            tree_json(&c.tree, &mut reqs);
            object(json!({ "collection": collection_json(&c), "requests": reqs }))
        },
    ));

    r.register(def::<PathIn, Out>(
        "read_request",
        "Source text of a .flux.md file with its blocks and diagnostics (codes and byte spans).",
        |ctx, a| {
            let a: PathIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let src = std::fs::read_to_string(&abs).map_err(|e| e.to_string())?;
            let doc = fluxwire_format::parse_with(
                &src,
                file_kind_for(&abs),
                &fluxwire_protocol::ProtocolRegistry::builtin(),
            );
            let blocks: Vec<Value> = doc
                .blocks
                .iter()
                .map(|b| {
                    json!({
                        "kind": format!("{:?}", b.kind),
                        "language": b.info.language,
                        "start": b.span.start,
                        "end": b.span.end,
                    })
                })
                .collect();
            object(json!({
                "path": abs.display().to_string(),
                "source": src,
                "runnable": doc.runnable(),
                "blocks": blocks,
                "diagnostics": diagnostics_json(&doc),
            }))
        },
    ));

    r.register(def::<PathIn, Out>(
        "diagnostics",
        "Parse diagnostics (code, severity, byte span, message) of a .flux.md file.",
        |ctx, a| {
            let a: PathIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let src = std::fs::read_to_string(&abs).map_err(|e| e.to_string())?;
            let doc = fluxwire_format::parse_with(
                &src,
                file_kind_for(&abs),
                &fluxwire_protocol::ProtocolRegistry::builtin(),
            );
            object(json!({
                "canonical": fluxwire_format::is_canonical(&src, file_kind_for(&abs)),
                "diagnostics": diagnostics_json(&doc),
            }))
        },
    ));

    r.register(def::<PathIn, Out>(
        "list_environments",
        "Environments of a Workspace (its environments/ directory): names and variable names with a secret flag. Values are never returned.",
        |ctx, a| {
            let a: PathIn = args(a)?;
            let dir = existing(ctx, &a.path)?;
            let mut secret_names: Vec<String> = Vec::new();
            for c in fluxwire_format::find_collections(&dir)
                .into_iter()
                .chain(dir.join("fluxwire.toml").is_file().then(|| dir.clone()))
            {
                if let Ok(cfg) = load_config(&c) {
                    secret_names.extend(cfg.secrets.names);
                }
            }
            let envs: Vec<Value> = fluxwire_format::list_environments(&dir)
                .into_iter()
                .map(|name| {
                    let env = fluxwire_format::load_environment(&dir, &name, &secret_names);
                    let vars: Vec<Value> = env
                        .rows
                        .iter()
                        .map(|r| json!({"name": r.name, "secret": r.secret || r.local}))
                        .collect();
                    json!({ "name": name, "variables": vars })
                })
                .collect();
            object(json!({ "environments": envs }))
        },
    ));

    let eng = e.clone();
    r.register(def::<ResolveIn, Out>(
        "resolve_vars",
        "Interpolates `{{variables}}` in text with the scope of a request (globals, collection, folder, request, environment). Secret values are masked; unresolved names are listed.",
        move |ctx, a| {
            let a: ResolveIn = args(a)?;
            let abs = existing(ctx, &a.path)?;
            let root = collection_root_of(&abs).ok_or("not inside a Collection")?;
            let src = std::fs::read_to_string(&abs).map_err(|e| e.to_string())?;
            let doc = fluxwire_format::parse(&src);
            let prep = Prep::new(eng.config_dir.clone(), a.env, workspace_of(ctx, &abs));
            let scope = prep.scope(&root, &abs, &doc);
            let i = fluxwire_vars::interpolate(&a.text, &scope);
            let used: Vec<Value> = i
                .used
                .iter()
                .map(|(n, s, secret)| json!({"name": n, "source": s.describe(), "secret": secret}))
                .collect();
            object(json!({
                "text": scope.redact(&i.text),
                "missing": i.missing,
                "used": used,
            }))
        },
    ));

    let eng = e;
    r.register(def::<HistoryIn, Out>(
        "history_query",
        "Recorded sends of a Workspace, newest first, by run_id, status class, text or origin. Rows are redacted when stored.",
        move |ctx, a| {
            let a: HistoryIn = args(a)?;
            let abs = checked(ctx, &a.path)?;
            let ws = workspace_of(ctx, &abs).ok_or("outside the allowed roots")?;
            let Some(store) = eng.store() else {
                return object(json!({ "entries": [] }));
            };
            let f = Filter {
                run_id: a.run_id,
                status_class: a.status_class,
                text: a.text,
                origin: a.origin,
                failed: a.failed.unwrap_or(false),
                limit: Some(a.limit.unwrap_or(50).min(500) as usize),
                ..Default::default()
            };
            let rows = store
                .list(&workspace_key(&ws), &f)
                .map_err(|e| e.to_string())?;
            let entries: Vec<Value> = rows
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id, "ts_ms": s.ts_ms, "collection": s.collection,
                        "request_path": s.request_path, "request_name": s.request_name,
                        "protocol": s.protocol, "method": s.method, "url": s.url,
                        "status": s.status, "total_ms": s.total_ms, "env": s.env,
                        "run_id": s.run_id, "origin": s.origin, "summary": s.summary,
                    })
                })
                .collect();
            object(json!({ "entries": entries }))
        },
    ));
}
