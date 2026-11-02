//! Engine tools through an rmcp client only (M9.2), against an axum server, a temp Workspace
//! and a copy of fixtures/payments-api.

use fluxwire_agent::engine::Engine;
use fluxwire_agent::tools::builtin_with;
use fluxwire_agent::{AllowSet, FluxwireServer, Roots};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SECRET: &str = "s3cr3t-value";

type Client = RunningService<rmcp::RoleClient, ()>;

struct Env {
    _tmp: tempfile::TempDir,
    ws: PathBuf,
    config: PathBuf,
    hits: Arc<AtomicUsize>,
    client: Client,
}

async fn axum_server() -> (u16, Arc<AtomicUsize>) {
    use axum::routing::get;
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let app = axum::Router::new()
        .route(
            "/echo-key",
            get(move |headers: axum::http::HeaderMap| {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    let k = headers
                        .get("x-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    axum::Json(json!({ "got": k }))
                }
            }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                "late"
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (port, hits)
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

async fn setup(allow: &str) -> Env {
    let (port, hits) = axum_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let ws = base.join("ws");
    put(
        &ws,
        "fluxwire.toml",
        "[collection]\nname = \"t\"\nformat = 1\n[secrets]\nnames = [\"apiKey\"]\n",
    );
    put(&ws, "collection.flux.md", "---\nname: t\n---\n\n# T\n");
    put(
        &ws,
        "environments/local.env",
        &format!("baseUrl=http://127.0.0.1:{port}\napiKey={SECRET}\n"),
    );
    put(
        &ws,
        "plain/ping.flux.md",
        "---\nname: Ping\nseq: 1\n---\n\n# Ping\n\n```http\nGET {{baseUrl}}/echo-key\nX-Key: {{apiKey}}\n```\n\n```http expect\nHTTP 200\n```\n",
    );
    put(
        &ws,
        "scripted/s.flux.md",
        "---\nname: Scripted\n---\n\n# S\n\n```http\nGET {{baseUrl}}/echo-key\nX-Key: {{apiKey}}\n```\n\n```ts post\nconsole.log('script-ran');\n```\n",
    );
    let config = base.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let engine = Engine::new(Some(config.clone()), Some(base.join("data")));
    let server = FluxwireServer::with_registry(
        builtin_with(engine),
        AllowSet::parse(&[allow]).unwrap(),
        vec!["stdio"],
        Roots::new(std::slice::from_ref(&ws)).unwrap(),
        None,
    );
    let (a, b) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let _ = server.serve(a).await.unwrap().waiting().await;
    });
    let client = ().serve(b).await.unwrap();
    Env {
        _tmp: tmp,
        ws,
        config,
        hits,
        client,
    }
}

async fn call_raw(c: &Client, name: &'static str, args: Value) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name);
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    c.peer().call_tool(p).await.unwrap()
}

async fn call(c: &Client, name: &'static str, args: Value) -> Value {
    let r = call_raw(c, name, args).await;
    assert_ne!(r.is_error, Some(true), "{name} failed: {r:?}");
    r.structured_content.unwrap()
}

fn p(root: &Path, rel: &str) -> String {
    root.join(rel).display().to_string()
}

fn assert_no_secret(v: &Value) {
    assert!(!v.to_string().contains(SECRET), "secret leaked: {v}");
}

#[tokio::test]
async fn read_edit_format_send_and_run() {
    let e = setup("read,write,execute").await;
    let c = &e.client;

    let v = call(c, "list_collections", json!({"path": e.ws})).await;
    assert_eq!(v["collections"].as_array().unwrap().len(), 1);
    let v = call(c, "list_requests", json!({"path": e.ws})).await;
    assert_eq!(v["requests"].as_array().unwrap().len(), 2);

    let ping = p(&e.ws, "plain/ping.flux.md");
    let v = call(c, "read_request", json!({"path": ping})).await;
    let src = v["source"].as_str().unwrap().to_string();
    let at = src.find("# Ping").unwrap();

    // Edit one span; everything else stays byte for byte.
    let v = call(
        c,
        "write_request",
        json!({"path": ping, "edits": [{"start": at, "end": at + 6, "text": "# Ping it"}]}),
    )
    .await;
    assert_eq!(v["written"], true);
    assert!(v["diff"].as_str().unwrap().contains("+# Ping it"));
    let new = std::fs::read_to_string(&ping).unwrap();
    assert_eq!(new, src.replacen("# Ping", "# Ping it", 1));

    // Dry run writes nothing.
    let v = call(
        c,
        "write_request",
        json!({"path": ping, "edits": [{"start": at, "end": at + 9, "text": "# Ping"}], "dry_run": true}),
    )
    .await;
    assert_eq!(v["written"], false);
    assert_eq!(std::fs::read_to_string(&ping).unwrap(), new);

    // Send: the secret is masked everywhere in the result.
    let v = call(c, "send_request", json!({"path": ping, "env": "local"})).await;
    assert_eq!(v["status"], 200, "{v}");
    assert_eq!(v["passed"], true);
    assert_no_secret(&v);
    assert!(v["body"].as_str().unwrap().contains("secret"));

    // Folder run, recorded in history with origin `agent` and the run id.
    let v = call(
        c,
        "run_folder",
        json!({"path": p(&e.ws, "plain"), "env": "local"}),
    )
    .await;
    assert_no_secret(&v);
    assert_eq!(v["report"]["summary"]["passed"], 1, "{v}");
    let run_id = v["run_id"].as_str().unwrap().to_string();
    let h = call(c, "history_query", json!({"path": e.ws, "run_id": run_id})).await;
    let rows = h["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["origin"], "agent");
    let all = call(c, "history_query", json!({"path": e.ws, "origin": "agent"})).await;
    assert_eq!(all["entries"].as_array().unwrap().len(), 2);
    assert_no_secret(&all);
}

#[tokio::test]
async fn environments_and_vars_never_expose_secret_values() {
    let e = setup("read").await;
    let c = &e.client;
    let v = call(c, "list_environments", json!({"path": e.ws})).await;
    assert_no_secret(&v);
    let vars = &v["environments"][0]["variables"];
    assert!(
        vars.as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "apiKey" && x["secret"] == true)
    );
    let v = call(
        c,
        "resolve_vars",
        json!({"path": p(&e.ws, "plain/ping.flux.md"), "env": "local", "text": "{{baseUrl}} {{apiKey}} {{nope}}"}),
    )
    .await;
    assert_no_secret(&v);
    assert_eq!(v["missing"], json!(["nope"]));
    assert!(v["text"].as_str().unwrap().starts_with("http://127.0.0.1"));
}

#[tokio::test]
async fn fmt_matches_the_canonical_formatter_and_preserves_unknown_content() {
    let e = setup("read,write").await;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/payments-api");
    let copy = e.ws.join("payments-api");
    copy_dir(&fixture, &copy);
    // Make one file non-canonical and add unknown front matter and content.
    let target = copy.join("payments/get-payment.flux.md");
    let messy = std::fs::read_to_string(&target)
        .unwrap()
        .replacen("---\n", "---\nx-keep: 1\n", 1)
        + "\n\n\n```x-unknown\nkeep me\n```\n\n";
    std::fs::write(&target, &messy).unwrap();

    let path = target.display().to_string();
    let v = call(&e.client, "fmt", json!({"path": path, "dry_run": true})).await;
    assert_eq!(std::fs::read_to_string(&target).unwrap(), messy);
    let want = fluxwire_format::format(&messy, fluxwire_format::file_kind_for(&target));
    assert_eq!(v["files"][0]["changed"], want != messy);

    call(&e.client, "fmt", json!({"path": path})).await;
    assert_eq!(std::fs::read_to_string(&target).unwrap(), want);
    assert!(want.contains("x-keep: 1") && want.contains("keep me"));
    let v = call(&e.client, "fmt", json!({"path": path})).await;
    assert_eq!(v["files"][0]["changed"], false);

    // The whole fixture formats and reads back through the tools.
    call(
        &e.client,
        "fmt",
        json!({"path": copy.display().to_string(), "dry_run": true}),
    )
    .await;
    let v = call(
        &e.client,
        "list_requests",
        json!({"path": copy.display().to_string()}),
    )
    .await;
    assert!(v["requests"].as_array().unwrap().len() >= 5);
    let d = call(&e.client, "diagnostics", json!({"path": path})).await;
    assert!(d["diagnostics"].is_array());
}

#[tokio::test]
async fn paths_outside_the_roots_are_refused() {
    let e = setup("read,write,execute").await;
    let outside = e.ws.parent().unwrap().join("escape.flux.md");
    for (name, args) in [
        (
            "write_request",
            json!({"path": outside, "content": "# x\n"}),
        ),
        ("read_request", json!({"path": "/etc/hosts"})),
        (
            "send_request",
            json!({"path": format!("{}/../x.flux.md", e.ws.display())}),
        ),
    ] {
        let r = call_raw(&e.client, name, args).await;
        assert_eq!(r.is_error, Some(true), "{name}");
    }
    assert!(!outside.exists());
    let r = call_raw(
        &e.client,
        "write_request",
        json!({"path": p(&e.ws, "a.txt"), "content": "x"}),
    )
    .await;
    assert_eq!(r.is_error, Some(true));
}

#[tokio::test]
async fn untrusted_scripts_do_not_run_and_trust_is_never_written() {
    let e = setup("read,execute").await;
    let path = p(&e.ws, "scripted/s.flux.md");
    let v = call(
        &e.client,
        "send_request",
        json!({"path": path, "env": "local"}),
    )
    .await;
    assert_eq!(
        e.hits.load(Ordering::SeqCst),
        0,
        "request must not be sent: {v}"
    );
    assert_eq!(v["error"]["phase"], "trust");
    assert!(v["scripts"].as_str().unwrap().starts_with("not run"));
    assert!(!v["console"].to_string().contains("script-ran"));
    assert!(!e.config.join("trust.toml").exists());

    // The user trusts the Collection in the app; MCP then honours it.
    fluxwire_format::set_trusted(&e.config, &e.ws, true).unwrap();
    let v = call(
        &e.client,
        "send_request",
        json!({"path": path, "env": "local"}),
    )
    .await;
    assert_eq!(v["scripts"], "run");
    assert!(v["console"].to_string().contains("script-ran"), "{v}");
    assert_no_secret(&v);
}

#[tokio::test]
async fn disallowed_classes_are_not_listed_or_callable() {
    let e = setup("read").await;
    let names: Vec<String> = e
        .client
        .peer()
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(names.contains(&"list_requests".to_string()));
    for n in [
        "write_request",
        "fmt",
        "send_request",
        "run_folder",
        "cancel",
    ] {
        assert!(!names.contains(&n.to_string()), "{n} listed");
    }
    let r = e
        .client
        .peer()
        .call_tool(CallToolRequestParams::new("send_request"))
        .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cancel_stops_a_running_folder() {
    let e = setup("read,execute").await;
    let (port, _) = axum_server().await;
    put(
        &e.ws,
        "slow/s.flux.md",
        &format!("---\nname: Slow\n---\n\n# S\n\n```http\nGET http://127.0.0.1:{port}/slow\n```\n"),
    );
    let c = Arc::new(e.client);
    let run = {
        let (c, path) = (c.clone(), p(&e.ws, "slow"));
        tokio::spawn(async move {
            call(&c, "run_folder", json!({"path": path, "cancel_key": "k1"})).await
        })
    };
    let mut cancelled = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if call(&c, "cancel", json!({"key": "k1"})).await["cancelled"] == true {
            cancelled = true;
            break;
        }
    }
    assert!(cancelled);
    let v = tokio::time::timeout(std::time::Duration::from_secs(10), run)
        .await
        .expect("run ends after cancel")
        .unwrap();
    assert_eq!(v["report"]["cancelled"], true);
}
