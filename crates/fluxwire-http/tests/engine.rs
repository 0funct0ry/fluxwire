mod common;

use std::time::{Duration, Instant};

use common::*;
use fluxwire_protocol::{Body, CancellationToken, ClientCert, Event, ExecCtx, NetConfig};

fn names(o: &fluxwire_protocol::Outcome) -> Vec<&str> {
    o.timings.iter().map(|t| t.name.as_str()).collect()
}

#[tokio::test]
async fn http1_get_has_status_body_and_phase_timings() {
    let s = http_server().await;
    let ev = send(&get_doc(&s.url("/json")), ExecCtx::default()).await;
    let o = outcome(&ev);
    let x = o.exchange.as_ref().unwrap();
    assert_eq!((x.status, x.version.as_str()), (200, "HTTP/1.1"));
    let v: serde_json::Value = serde_json::from_str(&body_text(o)).unwrap();
    assert_eq!(v, serde_json::json!({"ok": true, "n": [1, 2, 3]}));
    assert_eq!(
        names(o),
        [
            "queued", "dns", "connect", "tls", "sent", "waiting", "download", "total"
        ]
    );
    assert!(o.timings.iter().find(|t| t.name == "tls").unwrap().skipped);
    assert!(ev.iter().filter(|e| matches!(e, Event::Phase(_))).count() >= 7);
}

#[tokio::test]
async fn second_request_reuses_the_connection() {
    let s = http_server().await;
    let d = get_doc(&s.url("/json"));
    let first = send(&d, ExecCtx::default()).await;
    assert!(!outcome(&first).connection_reused);
    let second = send(&d, ExecCtx::default()).await;
    let o = outcome(&second);
    assert!(o.connection_reused);
    assert!(
        o.timings
            .iter()
            .filter(|t| ["dns", "connect", "tls"].contains(&t.name.as_str()))
            .all(|t| t.skipped)
    );
}

#[tokio::test]
async fn post_body_headers_query_and_form_reach_the_server() {
    let s = http_server().await;
    let d = doc(&format!(
        "---\nname: t\n---\n\n```http\nPOST {}\n    ?a=1\n    # &off=1\nX-Test: yes\n# X-Off: no\nContent-Type: text/plain\n\nhello body\n```\n",
        s.url("/echo")
    ));
    let ev = send(&d, ExecCtx::default()).await;
    let v: serde_json::Value = serde_json::from_str(&body_text(outcome(&ev))).unwrap();
    assert_eq!(v["method"], "POST");
    assert_eq!(v["query"], "a=1");
    assert_eq!(v["headers"]["x-test"], "yes");
    assert!(v["headers"].get("x-off").is_none());
    assert_eq!(v["body"], "hello body");

    let f = doc(&format!(
        "---\nname: t\n---\n\n```http\nPOST {}\n```\n\n```http form\nname: A B\nq: x&y\n```\n",
        s.url("/echo")
    ));
    let ev = send(&f, ExecCtx::default()).await;
    let v: serde_json::Value = serde_json::from_str(&body_text(outcome(&ev))).unwrap();
    assert_eq!(v["body"], "name=A+B&q=x%26y");
    assert_eq!(
        v["headers"]["content-type"],
        "application/x-www-form-urlencoded"
    );
}

#[tokio::test]
async fn redirects_are_followed_and_recorded() {
    let s = http_server().await;
    let ev = send(&get_doc(&s.url("/redirect/2")), ExecCtx::default()).await;
    let o = outcome(&ev);
    assert_eq!(o.exchange.as_ref().unwrap().status, 200);
    assert_eq!(o.redirects.len(), 3);
    assert!(o.final_url.ends_with("/json"));
    assert_eq!(o.timings[0].name, "redirects");

    let mut ctx = ExecCtx::default();
    ctx.settings.follow_redirects = false;
    let ev = send(&get_doc(&s.url("/redirect")), ctx).await;
    assert_eq!(outcome(&ev).exchange.as_ref().unwrap().status, 303);

    let mut ctx = ExecCtx::default();
    ctx.settings.max_redirects = 1;
    let ev = send(&get_doc(&s.url("/redirect/5")), ctx).await;
    match ev.last().unwrap() {
        Event::Failed(e) => assert_eq!(e.phase, "redirect"),
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn post_redirect_becomes_get() {
    let s = http_server().await;
    let d = doc(&format!(
        "---\nname: t\n---\n\n```http\nPOST {}\nContent-Type: text/plain\n\nbody\n```\n",
        s.url("/post-redirect")
    ));
    let ev = send(&d, ExecCtx::default()).await;
    let v: serde_json::Value = serde_json::from_str(&body_text(outcome(&ev))).unwrap();
    assert_eq!(v["method"], "GET");
    assert_eq!(v["body"], "");
}

#[tokio::test]
async fn gzip_brotli_and_zstd_bodies_are_decoded() {
    let s = http_server().await;
    let want = "hello compressed world ".repeat(1000);
    for kind in ["gzip", "br", "zstd"] {
        let ev = send(
            &get_doc(&s.url(&format!("/compressed/{kind}"))),
            ExecCtx::default(),
        )
        .await;
        let o = outcome(&ev);
        let x = o.exchange.as_ref().unwrap();
        assert_eq!(body_text(o), want, "{kind}");
        assert_eq!(x.decoded_size, want.len() as u64);
        assert!(
            x.wire_size < x.decoded_size / 4,
            "{kind}: wire {} decoded {}",
            x.wire_size,
            x.decoded_size
        );
    }
}

#[tokio::test]
async fn hundred_megabytes_stream_to_a_temp_file() {
    let s = http_server().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = ExecCtx {
        temp_dir: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    let ev = send(&get_doc(&s.url("/big/100")), ctx).await;
    let o = outcome(&ev);
    let x = o.exchange.as_ref().unwrap();
    match &x.body {
        Body::File { path, len } => {
            assert_eq!(*len, 100 * 1024 * 1024);
            assert!(path.starts_with(dir.path()));
            assert_eq!(std::fs::metadata(&**path).unwrap().len(), *len);
        }
        b => panic!("expected a file body, got {b:?}"),
    }
    let p = match &x.body {
        Body::File { path, .. } => path.to_path_buf(),
        _ => unreachable!(),
    };
    drop(ev);
    assert!(!p.exists(), "temp file removed when the result is dropped");
}

#[tokio::test]
async fn body_cap_fails_in_the_download_phase() {
    let s = http_server().await;
    let mut ctx = ExecCtx::default();
    ctx.settings.max_body = 1024 * 1024;
    let ev = send(&get_doc(&s.url("/big/4")), ctx).await;
    match ev.last().unwrap() {
        Event::Failed(e) => {
            assert_eq!(e.phase, "download");
            assert!(e.fix.is_some());
        }
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn cancel_stops_a_slow_body_quickly() {
    let s = http_server().await;
    let token = CancellationToken::new();
    let ctx = ExecCtx {
        cancel: token.clone(),
        ..Default::default()
    };
    let t = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
    });
    let ev = send(&get_doc(&s.url("/slow")), ctx).await;
    assert!(t.elapsed() < Duration::from_secs(3));
    match ev.last().unwrap() {
        Event::Failed(e) => assert_eq!(e.phase, "cancelled"),
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn timeout_names_the_phase() {
    let s = http_server().await;
    let d = doc(&format!(
        "---\nname: t\nsettings:\n  timeout_ms: 300\n---\n\n```http\nGET {}\n```\n",
        s.url("/slow")
    ));
    let ev = send(&d, ExecCtx::default()).await;
    match ev.last().unwrap() {
        Event::Failed(e) => {
            assert_eq!(e.phase, "download");
            assert!(e.message.contains("Timed out"), "{e}");
        }
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn refused_connection_names_the_connect_phase() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let ev = send(
        &get_doc(&format!("http://127.0.0.1:{port}/")),
        ExecCtx::default(),
    )
    .await;
    match ev.last().unwrap() {
        Event::Failed(e) => {
            assert_eq!(e.phase, "connect");
            assert!(e.message.contains("refused"), "{e}");
            assert!(e.fix.is_some());
        }
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn unknown_host_names_the_dns_phase() {
    let ev = send(&get_doc("http://no-such-host.invalid/"), ExecCtx::default()).await;
    match ev.last().unwrap() {
        Event::Failed(e) => assert_eq!(e.phase, "dns"),
        o => panic!("{o:?}"),
    }
}

fn tls_ctx(pki: &Pki) -> ExecCtx {
    ExecCtx {
        net: NetConfig {
            ca_files: vec![pki.ca_pem.clone()],
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn tls_with_a_test_ca_negotiates_h2() {
    let pki = pki();
    let s = pki.tls_server(false).await;
    let ev = send(&get_doc(&s.tls_url("/echo")), tls_ctx(&pki)).await;
    let o = outcome(&ev);
    assert_eq!(o.exchange.as_ref().unwrap().version, "HTTP/2");
    let v: serde_json::Value = serde_json::from_str(&body_text(o)).unwrap();
    assert_eq!(v["version"], "HTTP/2.0");
    assert!(!o.timings.iter().find(|t| t.name == "tls").unwrap().skipped);
    // multiplexed connection is reused
    let again = send(&get_doc(&s.tls_url("/echo")), tls_ctx(&pki)).await;
    assert!(outcome(&again).connection_reused);
}

#[tokio::test]
async fn untrusted_certificate_gives_an_actionable_error() {
    let pki = pki();
    let s = pki.tls_server(false).await;
    let ev = send(&get_doc(&s.tls_url("/echo")), ExecCtx::default()).await;
    match ev.last().unwrap() {
        Event::Failed(e) => {
            assert_eq!(e.phase, "tls");
            assert!(e.message.contains("not trusted"), "{e}");
            assert!(e.fix.as_deref().unwrap().contains("Add the CA"));
        }
        o => panic!("{o:?}"),
    }
}

#[tokio::test]
async fn verify_tls_false_skips_verification() {
    let pki = pki();
    let s = pki.tls_server(false).await;
    let d = doc(&format!(
        "---\nname: t\nsettings:\n  verify_tls: false\n---\n\n```http\nGET {}\n```\n",
        s.tls_url("/json")
    ));
    let ev = send(&d, ExecCtx::default()).await;
    assert_eq!(outcome(&ev).exchange.as_ref().unwrap().status, 200);
}

#[tokio::test]
async fn mtls_requires_the_per_host_client_certificate() {
    let pki = pki();
    let s = pki.tls_server(true).await;
    let d = get_doc(&s.tls_url("/json"));
    let without = send(&d, tls_ctx(&pki)).await;
    assert!(
        matches!(without.last().unwrap(), Event::Failed(_)),
        "{:?}",
        without.last()
    );

    let (cert, key) = pki.client_cert();
    let mut ctx = tls_ctx(&pki);
    ctx.net.client_certs.push(ClientCert {
        host: "localhost".into(),
        cert,
        key,
    });
    let ev = send(&d, ctx).await;
    assert_eq!(outcome(&ev).exchange.as_ref().unwrap().status, 200);
}

#[tokio::test]
async fn digest_auth_answers_the_challenge() {
    let s = http_server().await;
    let d = doc(&format!(
        "---\nname: t\nauth:\n  type: digest\n  username: mufasa\n  password: pw\n---\n\n```http\nGET {}\n```\n",
        s.url("/digest")
    ));
    let ev = send(&d, ExecCtx::default()).await;
    let o = outcome(&ev);
    assert_eq!(o.exchange.as_ref().unwrap().status, 200);
    assert_eq!(body_text(o), "welcome");
}

#[tokio::test]
async fn bearer_and_apikey_auth_are_applied() {
    let s = http_server().await;
    let d = doc(&format!(
        "---\nname: t\nauth:\n  type: bearer\n  token: tok\n---\n\n```http\nGET {}\n```\n",
        s.url("/echo")
    ));
    let v: serde_json::Value =
        serde_json::from_str(&body_text(outcome(&send(&d, ExecCtx::default()).await))).unwrap();
    assert_eq!(v["headers"]["authorization"], "Bearer tok");

    let d = doc(&format!(
        "---\nname: t\nauth:\n  type: apikey\n  key: k\n  value: v\n  in: query\n---\n\n```http\nGET {}\n```\n",
        s.url("/echo")
    ));
    let v: serde_json::Value =
        serde_json::from_str(&body_text(outcome(&send(&d, ExecCtx::default()).await))).unwrap();
    assert_eq!(v["query"], "k=v");
}

#[tokio::test]
async fn http_passes_the_protocol_conformance_suite() {
    let s = http_server().await;
    let fast = get_doc(&s.url("/json"));
    let slow = get_doc(&s.url("/slow"));
    fluxwire_protocol::conformance::run(
        &fluxwire_http::HttpProtocol,
        fluxwire_protocol::conformance::Fixture {
            fast: &fast,
            slow: &slow,
            scope: Default::default(),
            ctx: ExecCtx::default(),
            fast_status: 200,
        },
    )
    .await;
}

#[test]
fn plan_reports_unresolved_variables_and_bad_urls() {
    use fluxwire_protocol::Protocol;
    let p = fluxwire_http::HttpProtocol;
    let d = doc("---\nname: t\n---\n\n```http\nGET {{baseUrl}}/v1\n```\n");
    let e = p.plan(&d, &Default::default()).unwrap_err();
    assert!(e[0].message.contains("{{baseUrl}}"));
    let mut scope = fluxwire_vars::VarScope::default();
    scope.set_literal("baseUrl", "http://x.test");
    assert!(p.plan(&d, &scope).is_ok());
    let d = doc("---\nname: t\n---\n\n```http\nGET not a url\n```\n");
    assert!(p.plan(&d, &Default::default()).is_err());
    let d = doc("---\nname: t\n---\n\n```http\nGET ftp://x.test/\n```\n");
    assert!(
        p.plan(&d, &Default::default()).unwrap_err()[0]
            .message
            .contains("scheme")
    );
}

#[test]
fn copy_as_curl_masks_secrets_by_default() {
    use fluxwire_protocol::Protocol;
    let p = fluxwire_http::HttpProtocol;
    let d = doc(
        "---\nname: t\nauth:\n  type: bearer\n  token: s3cret\n---\n\n```http\nPOST http://x.test/a\nContent-Type: application/json\n\n{\"a\":1}\n```\n",
    );
    let plan = p.plan(&d, &Default::default()).unwrap();
    let masked = p.to_curl(&plan, true).unwrap();
    assert!(
        !masked.contains("s3cret") && masked.contains("Bearer ***"),
        "{masked}"
    );
    assert!(masked.contains("--data-raw '{\"a\":1}'"));
    assert!(p.to_curl(&plan, false).unwrap().contains("Bearer s3cret"));
}
