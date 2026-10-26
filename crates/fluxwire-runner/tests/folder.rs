//! Folder runs (SPEC §12) against an in-process axum server and a temp collection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use fluxwire_runner::*;
use serde_json::json;

async fn server() -> (u16, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/login",
            post(|| async { Json(json!({"access_token": "tok-123"})) }),
        )
        .route(
            "/items",
            post(|h: HeaderMap| async move {
                let ok = h
                    .get("authorization")
                    .is_some_and(|v| v == "Bearer tok-123");
                (
                    if ok {
                        axum::http::StatusCode::CREATED
                    } else {
                        axum::http::StatusCode::UNAUTHORIZED
                    },
                    Json(json!({"id": "item-9"})),
                )
            }),
        )
        .route(
            "/items/item-9",
            get(|h: HeaderMap| async move {
                let ok = h
                    .get("authorization")
                    .is_some_and(|v| v == "Bearer tok-123");
                (
                    if ok {
                        axum::http::StatusCode::OK
                    } else {
                        axum::http::StatusCode::UNAUTHORIZED
                    },
                    Json(json!({"id": "item-9"})),
                )
            }),
        )
        .route(
            "/json",
            get(|State(h): State<Arc<AtomicUsize>>| async move {
                h.fetch_add(1, Ordering::SeqCst);
                Json(json!({"ok": true}))
            }),
        )
        .route(
            "/echo/{v}",
            get(
                |axum::extract::Path(v): axum::extract::Path<String>| async move {
                    Json(json!({"v": v}))
                },
            ),
        )
        .route(
            "/boom",
            get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                "late"
            }),
        )
        .with_state(hits.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (port, hits)
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn collection(tag: &str, port: u16) -> PathBuf {
    let root = std::env::temp_dir().join(format!("fw-folder-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(
        &root,
        "fluxwire.toml",
        "[collection]\nname = \"t\"\nformat = 1\n",
    );
    write(
        &root,
        "collection.flux.md",
        &format!("---\nvars:\n  base: http://127.0.0.1:{port}\n---\n\n# T\n"),
    );
    root
}

fn req(name: &str, seq: u32, extra_fm: &str, line: &str, tail: &str) -> String {
    format!(
        "---\nname: {name}\nseq: {seq}\n{extra_fm}---\n\n# {name}\n\n```http\n{line}\n```\n{tail}"
    )
}

fn plan(root: &Path, folder: &str) -> RunPlan {
    RunPlan {
        collection_root: root.to_path_buf(),
        folder: folder.into(),
        iterations: 1,
        scripts: ScriptOptions {
            trusted: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn finish(mut h: FolderRunHandle) -> (FolderRunResult, usize) {
    let mut finished = 0;
    while let Some(ev) = h.events.recv().await {
        match ev {
            FolderRunEvent::RequestFinished(_) => finished += 1,
            FolderRunEvent::Finished(r) => return (*r, finished),
            _ => {}
        }
    }
    panic!("no Finished event");
}

#[tokio::test]
async fn sequential_chain_passes_variables() {
    let (port, _) = server().await;
    let root = collection("chain", port);
    write(
        &root,
        "api/folder.flux.md",
        "---\nname: Api\nsettings:\n  run_mode: sequential\nauth:\n  type: bearer\n  token: \"{{tok}}\"\n---\n\n# Api\n",
    );
    write(
        &root,
        "api/1-login.flux.md",
        &req(
            "Login",
            1,
            "capture:\n  tok: $.access_token\n",
            "POST {{base}}/login",
            "\n```http expect\nHTTP 200\n```\n",
        ),
    );
    write(
        &root,
        "api/2-create.flux.md",
        &req(
            "Create",
            2,
            "capture:\n  itemId: $.id\n",
            "POST {{base}}/items",
            "\n```http expect\nHTTP 201\n```\n",
        ),
    );
    write(
        &root,
        "api/3-get.flux.md",
        &req(
            "Get",
            3,
            "",
            "GET {{base}}/items/{{itemId}}",
            "\n```http expect\nHTTP 200\n```\n",
        ),
    );
    let (r, n) = finish(runner().run_folder(plan(&root, "api"))).await;
    assert_eq!(n, 3);
    let s = r.summary();
    assert_eq!(
        (s.passed, s.failed),
        (3, 0),
        "{:#?}",
        r.requests
            .iter()
            .map(|x| (
                &x.request.key,
                x.result.error.as_ref().map(|e| e.describe())
            ))
            .collect::<Vec<_>>()
    );
    let order: Vec<_> = r.requests.iter().map(|x| x.request.name.as_str()).collect();
    assert_eq!(order, ["Login", "Create", "Get"]);
}

#[tokio::test]
async fn concurrent_scopes_are_isolated() {
    let (port, _) = server().await;
    let root = collection("iso", port);
    // `a` captures `who`; `b` reads it and must NOT see it (isolated scopes).
    write(
        &root,
        "x/a.flux.md",
        &req("A", 1, "capture:\n  who: $.id\n", "POST {{base}}/items", ""),
    );
    write(
        &root,
        "x/b.flux.md",
        &req("B", 2, "", "GET {{base}}/echo/{{who}}", ""),
    );
    let (r, _) = finish(runner().run_folder(plan(&root, "x"))).await;
    let b = r.requests.iter().find(|x| x.request.name == "B").unwrap();
    assert!(
        b.result.warnings.iter().any(|w| w.message.contains("who")) || b.result.error.is_some(),
        "B must not see A's capture"
    );
}

#[tokio::test]
async fn bail_skips_the_rest() {
    let (port, _) = server().await;
    let root = collection("bail", port);
    write(
        &root,
        "f/1.flux.md",
        &req(
            "Bad",
            1,
            "",
            "GET {{base}}/boom",
            "\n```http expect\nHTTP 200\n```\n",
        ),
    );
    write(
        &root,
        "f/2.flux.md",
        &req("Next", 2, "", "GET {{base}}/json", ""),
    );
    let mut p = plan(&root, "f");
    p.mode = RunMode::Sequential;
    p.bail = true;
    let (r, _) = finish(runner().run_folder(p)).await;
    let s = r.summary();
    assert_eq!((s.failed, s.skipped), (1, 1));
    assert!(r.bailed);
}

#[tokio::test]
async fn filters_by_tag_and_glob_and_only() {
    let (port, _) = server().await;
    let root = collection("filter", port);
    write(
        &root,
        "f/a.flux.md",
        &req("Alpha", 1, "tags: [smoke]\n", "GET {{base}}/json", ""),
    );
    write(
        &root,
        "f/b.flux.md",
        &req("Beta", 2, "", "GET {{base}}/json", ""),
    );
    let mut p = plan(&root, "f");
    p.filter = Some("tag:smoke".into());
    let (r, _) = finish(runner().run_folder(p.clone())).await;
    assert_eq!(r.requests.len(), 1);
    assert_eq!(r.requests[0].request.name, "Alpha");
    p.filter = Some("be*".into());
    let (r, _) = finish(runner().run_folder(p.clone())).await;
    assert_eq!(r.requests[0].request.name, "Beta");
    p.filter = None;
    p.only = Some(vec!["f/b.flux.md".into()]);
    let (r, _) = finish(runner().run_folder(p)).await;
    assert_eq!(r.requests.len(), 1);
}

#[tokio::test]
async fn data_rows_become_iterations_with_row_variables() {
    let (port, _) = server().await;
    let root = collection("data", port);
    write(
        &root,
        "f/a.flux.md",
        &req(
            "Echo",
            1,
            "",
            "GET {{base}}/echo/{{name}}",
            "\n```http expect\nHTTP 200\n```\n",
        ),
    );
    let mut p = plan(&root, "f");
    p.data = parse_csv_rows("name\none\ntwo\nthree\n");
    let (r, _) = finish(runner().run_folder(p)).await;
    assert_eq!(r.iterations, 3);
    assert_eq!(r.requests.len(), 3);
    let urls: Vec<_> = r
        .requests
        .iter()
        .map(|x| x.result.outcome.as_ref().unwrap().final_url.clone())
        .collect();
    assert!(
        urls[0].ends_with("/echo/one") && urls[2].ends_with("/echo/three"),
        "{urls:?}"
    );
}

#[tokio::test]
async fn timeout_and_cancel_stop_the_run() {
    let (port, _) = server().await;
    let root = collection("cancel", port);
    write(
        &root,
        "f/slow.flux.md",
        &req("Slow", 1, "", "GET {{base}}/slow", ""),
    );
    let mut p = plan(&root, "f");
    p.timeout_ms = Some(300);
    let t = Instant::now();
    let (r, _) = finish(runner().run_folder(p.clone())).await;
    assert!(r.timed_out && t.elapsed().as_secs() < 5);
    p.timeout_ms = None;
    let h = runner().run_folder(p);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    h.cancel();
    let (r, _) = finish(h).await;
    assert!(r.cancelled);
    assert_eq!(r.summary().skipped, 1);
}

#[tokio::test]
async fn serializers_redact_and_escape() {
    let (port, _) = server().await;
    let root = collection("ser", port);
    write(
        &root,
        "f/a.flux.md",
        &req(
            "A <&>",
            1,
            "",
            "GET {{base}}/boom",
            "\n```http expect\nHTTP 200\n```\n",
        ),
    );
    let (r, _) = finish(runner().run_folder(plan(&root, "f"))).await;
    let j: serde_json::Value = serde_json::from_str(&to_json(&r)).unwrap();
    assert_eq!(j["summary"]["failed"], 1);
    let x = to_junit(&r);
    assert!(
        x.contains("A &lt;&amp;&gt;") && x.contains("<failure"),
        "{x}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thousand_requests_at_concurrency_32_under_three_seconds() {
    let (port, hits) = server().await;
    let root = collection("bench", port);
    for i in 0..1000 {
        write(
            &root,
            &format!("bulk/r{i:04}.flux.md"),
            &req(
                &format!("R{i}"),
                i,
                "",
                "GET {{base}}/json",
                "\n```http expect\nHTTP 200\n```\n",
            ),
        );
    }
    let mut p = plan(&root, "bulk");
    p.concurrency = Some(32);
    let t = Instant::now();
    let (r, n) = finish(runner().run_folder(p)).await;
    let el = t.elapsed();
    assert_eq!(n, 1000);
    assert_eq!(r.summary().passed, 1000);
    assert_eq!(hits.load(Ordering::SeqCst), 1000);
    assert!(el.as_secs_f64() < 3.0, "took {el:?}");
    let _ = BTreeMap::<String, String>::new();
}
