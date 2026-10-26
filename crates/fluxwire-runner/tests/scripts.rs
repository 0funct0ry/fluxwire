//! Script phases through the runner: order, capture, expect + tests, trust, errors.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fluxwire_runner::{
    RunError, RunEvent, RunResult, ScriptOptions, SendOptions, VarScope, runner,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Seen = Arc<Mutex<Vec<String>>>;

/// Answers every request with a 201 JSON payment and records the raw requests.
async fn server() -> (u16, Seen) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            let seen = s2.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = r#"{"id":"p_1","status":"requires_capture","amount":4200,"extra":true}"#;
                let _ = s
                    .write_all(
                        format!(
                            "HTTP/1.1 201 Created\r\ncontent-length: {}\r\ncontent-type: application/json\r\nlocation: /v1/payments/p_1\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    (port, seen)
}

fn collection(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fw-runner-scripts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::create_dir_all(d.join("payments")).unwrap();
    std::fs::write(
        d.join("fluxwire.toml"),
        "[collection]\nname = \"c\"\nformat = 1\n",
    )
    .unwrap();
    std::fs::write(
        d.join("lib/sign.ts"),
        "export function sign(body: string, key: string): string {\n  return 'sha256=' + fw.crypto.hmac('sha256', key, body);\n}\n",
    )
    .unwrap();
    d
}

async fn send(root: &Path, rel: &str, port: u16, trusted: bool) -> RunResult {
    let file = root.join(rel);
    let doc = fluxwire_format::parse(&std::fs::read_to_string(&file).unwrap());
    let mut scope = VarScope::default();
    scope.set_literal("baseUrl", format!("http://127.0.0.1:{port}"));
    scope.set_literal("customerId", "cus_1");
    scope.set_literal("hmacKey", "k");
    let mut h = runner().send(
        &doc,
        SendOptions {
            scope,
            scripts: ScriptOptions {
                trusted,
                collection_root: Some(root.to_path_buf()),
                file: Some(file),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    while let Some(e) = h.events.recv().await {
        if let RunEvent::Finished(r) = e {
            return *r;
        }
    }
    panic!("no result");
}

const CREATE: &str = r#"---
name: Create payment
capture:
  paymentId: $.id
  location: header Location
---

# Create payment

```http
POST {{baseUrl}}/v1/payments
Content-Type: application/json
Idempotency-Key: {{idemKey}}

{ "amount": 4200, "currency": "INR", "customer": "{{customerId}}" }
```

```http expect
HTTP 201
Content-Type: application/json

{ "status": "requires_capture", "amount": 4200 }
```

```ts pre
import { sign } from "lib/sign.ts";
fw.vars.set("idemKey", crypto.randomUUID());
fw.request.headers.set("X-Signature", sign(fw.request.body.text(), fw.env.get("hmacKey") ?? ""));
console.log("pre ran");
```

```ts test
fw.test("has an id", () => fw.expect(fw.response.json()).toHaveProperty("id"));
fw.test("captured", () => fw.expect(fw.vars.get("paymentId")).toBe("p_1"));
```
"#;

#[tokio::test]
async fn create_payment_end_to_end() {
    let (port, seen) = server().await;
    let root = collection("create");
    std::fs::write(root.join("payments/create.flux.md"), CREATE).unwrap();
    let r = send(&root, "payments/create.flux.md", port, true).await;

    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(r.script_errors.is_empty(), "{:?}", r.script_errors);
    let by: Vec<(&str, bool)> = r
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.passed))
        .collect();
    assert_eq!(
        by,
        [
            ("expect status", true),
            ("expect header Content-Type", true),
            ("expect body $.status", true),
            ("expect body $.amount", true),
            ("has an id", true),
            ("captured", true),
        ],
        "expect checks come before test blocks (SPEC §11.2)"
    );

    let changes: Vec<_> = r.var_changes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(changes, ["idemKey", "paymentId", "location"]);
    assert_eq!(r.console[0].text, "pre ran");

    let req = seen.lock().unwrap()[0].clone().to_lowercase();
    let idem = r.var_changes[0].value.clone().unwrap();
    assert!(req.contains(&format!("idempotency-key: {idem}")), "{req}");
    assert!(req.contains("x-signature: sha256="), "{req}");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn untrusted_collections_do_not_run_scripts_or_send() {
    let (port, seen) = server().await;
    let root = collection("untrusted");
    std::fs::write(root.join("payments/create.flux.md"), CREATE).unwrap();
    let r = send(&root, "payments/create.flux.md", port, false).await;
    assert!(
        matches!(r.error, Some(RunError::Untrusted)),
        "{:?}",
        r.error
    );
    assert!(seen.lock().unwrap().is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn collection_and_folder_scripts_run_in_spec_order() {
    let (port, _seen) = server().await;
    let root = collection("order");
    std::fs::write(
        root.join("collection.flux.md"),
        "---\nname: c\n---\n\n```ts pre\nconsole.log('c-pre');\n```\n\n```ts post\nconsole.log('c-post');\n```\n\n```ts test\nconsole.log('c-test');\n```\n",
    )
    .unwrap();
    std::fs::write(
        root.join("payments/folder.flux.md"),
        "---\nname: p\n---\n\n```ts pre\nconsole.log('f-pre');\n```\n\n```ts post\nconsole.log('f-post');\n```\n\n```ts test\nconsole.log('f-test');\n```\n",
    )
    .unwrap();
    std::fs::write(
        root.join("payments/r.flux.md"),
        "---\nname: r\n---\n\n```http\nGET {{baseUrl}}/x\n```\n\n```ts pre\nconsole.log('r-pre');\n```\n\n```ts post\nconsole.log('r-post');\n```\n\n```ts test\nconsole.log('r-test');\n```\n",
    )
    .unwrap();
    let r = send(&root, "payments/r.flux.md", port, true).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    let order: Vec<_> = r.console.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        order,
        [
            "c-pre", "f-pre", "r-pre", "r-post", "f-post", "c-post", "c-test", "f-test", "r-test"
        ]
    );
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn pre_script_error_blocks_the_send_and_points_at_the_file_line() {
    let (port, seen) = server().await;
    let root = collection("err");
    let src = "---\nname: r\n---\n\n```http\nGET {{baseUrl}}/x\n```\n\n```ts pre\nconst a = 1;\nthrow new Error('nope');\n```\n";
    std::fs::write(root.join("payments/r.flux.md"), src).unwrap();
    let r = send(&root, "payments/r.flux.md", port, true).await;
    let Some(RunError::Script(errs)) = &r.error else {
        panic!("{:?}", r.error)
    };
    assert_eq!(errs[0].message, "nope");
    assert_eq!(errs[0].file, "payments/r.flux.md");
    // line of `throw` in the file: the fence opens on line 9, so content starts on 10 and throw is line 11.
    assert_eq!(errs[0].line, 11);
    assert!(seen.lock().unwrap().is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn skip_and_send_request_and_post_errors() {
    let (port, seen) = server().await;
    let root = collection("misc");
    std::fs::write(
        root.join("payments/skip.flux.md"),
        "---\nname: s\n---\n\n```http\nGET {{baseUrl}}/x\n```\n\n```ts pre\nfw.skip('not now');\n```\n",
    )
    .unwrap();
    let r = send(&root, "payments/skip.flux.md", port, true).await;
    assert_eq!(r.skipped.as_deref(), Some("not now"));
    assert!(r.error.is_none() && seen.lock().unwrap().is_empty());

    std::fs::write(
        root.join("payments/chain.flux.md"),
        "---\nname: c\n---\n\n```http\nGET {{baseUrl}}/a\n```\n\n```ts post\nconst r = await fw.sendRequest('payments/skip-target.flux.md', { vars: { who: 'x' } });\nfw.vars.set('nested', String(r.status));\nthrow new Error('after');\n```\n",
    )
    .unwrap();
    std::fs::write(
        root.join("payments/skip-target.flux.md"),
        "---\nname: t\n---\n\n```http\nGET {{baseUrl}}/b/{{who}}\n```\n",
    )
    .unwrap();
    let r = send(&root, "payments/chain.flux.md", port, true).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(
        r.outcome.is_some(),
        "a failing post script keeps the response"
    );
    assert_eq!(r.script_errors[0].message, "after");
    assert!(
        r.var_changes
            .iter()
            .any(|c| c.name == "nested" && c.value.as_deref() == Some("201"))
    );
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|s| s.starts_with("GET /b/x ")), "{seen:?}");
    std::fs::remove_dir_all(root).ok();
}

#[tokio::test]
async fn secrets_set_by_scripts_are_redacted_in_console() {
    let (port, _) = server().await;
    let root = collection("secret");
    std::fs::write(
        root.join("payments/r.flux.md"),
        "---\nname: r\n---\n\n```http\nGET {{baseUrl}}/x\nX-Token: {{tok}}\n```\n\n```ts pre\nfw.vars.setSecret('tok', 'hunter2hunter2');\nconsole.log('token is hunter2hunter2');\n```\n",
    )
    .unwrap();
    let r = send(&root, "payments/r.flux.md", port, true).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.console[0].text, "token is ‹secret:tok›");
    assert!(!r.label.contains("hunter2"));
    std::fs::remove_dir_all(root).ok();
}
