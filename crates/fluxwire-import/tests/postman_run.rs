//! Acceptance: import a Postman collection with test scripts, run the folder, and see the
//! tests execute through the `pm` shim.

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use fluxwire_import::{Options, Source, apply, plan};
use fluxwire_runner::{FolderRunEvent, RunPlan, ScriptOptions, runner};
use serde_json::json;

const COLLECTION: &str = r#"{
  "info": { "name": "Run me", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
  "variable": [{ "key": "baseUrl", "value": "http://127.0.0.1:PORT" }],
  "item": [{
    "name": "Flow",
    "item": [
      { "name": "Login",
        "request": { "method": "POST", "header": [], "url": "{{baseUrl}}/login" },
        "event": [{ "listen": "test", "script": { "exec": [
          "pm.test('status is 200', function () { pm.response.to.have.status(200); });",
          "const body = pm.response.json();",
          "pm.test('has token', () => pm.expect(body).to.have.property('token'));",
          "pm.environment.set('token', body.token);"
        ] } }] },
      { "name": "Me",
        "request": { "method": "GET", "header": [{ "key": "Authorization", "value": "Bearer {{token}}" }], "url": "{{baseUrl}}/me" },
        "event": [{ "listen": "test", "script": { "exec": [
          "pm.test('is the demo user', () => pm.expect(pm.response.json().user).to.equal('demo'));"
        ] } }] }
    ]
  }]
}"#;

#[tokio::test]
async fn imported_postman_tests_execute_via_the_shim() {
    let app = Router::new()
        .route("/login", post(|| async { Json(json!({"token": "t-1"})) }))
        .route(
            "/me",
            get(|h: HeaderMap| async move {
                if h.get("authorization").is_some_and(|v| v == "Bearer t-1") {
                    (StatusCode::OK, Json(json!({"user": "demo"})))
                } else {
                    (StatusCode::UNAUTHORIZED, Json(json!({})))
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("c.postman_collection.json");
    std::fs::write(&src, COLLECTION.replace("PORT", &port.to_string())).unwrap();
    let mut p = plan(Source::Postman, &src, &Options::default()).unwrap();
    assert!(p.report.lossy.is_empty(), "{:?}", p.report.lossy);
    let out = dir.path().join("out");
    apply(&mut p, &out, &|_| false).unwrap();

    let mut h = runner().run_folder(RunPlan {
        collection_root: out,
        folder: "flow".into(),
        iterations: 1,
        scripts: ScriptOptions {
            trusted: true,
            ..Default::default()
        },
        ..Default::default()
    });
    while let Some(ev) = h.events.recv().await {
        if let FolderRunEvent::Finished(r) = ev {
            let s = r.summary();
            assert_eq!(r.error, None);
            assert_eq!(
                (s.passed, s.failed),
                (2, 0),
                "{:?}",
                r.requests
                    .iter()
                    .map(|x| x.result.error.as_ref().map(|e| e.describe()))
                    .collect::<Vec<_>>()
            );
            assert_eq!((s.tests, s.tests_failed), (3, 0));
            return;
        }
    }
    panic!("no Finished event");
}
