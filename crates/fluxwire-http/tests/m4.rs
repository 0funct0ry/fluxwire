//! M4: inheritance, variables, cookies, secrets in cURL, OAuth2 against a mock server.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use common::*;
use fluxwire_http::HttpProtocol;
use fluxwire_http::oauth2::{self, Options};
use fluxwire_protocol::{EnvState, Event, ExecCtx, Protocol};
use fluxwire_vars::{Layer, Source, Var, VarScope};
use futures::StreamExt;
use sha2::{Digest, Sha256};

async fn send_with(src: &str, scope: &VarScope, ctx: ExecCtx) -> Vec<Event> {
    let p = HttpProtocol;
    let plan = p.plan(&doc(src), scope).expect("plan");
    p.execute(plan, ctx).collect().await
}

fn echo_json(ev: &[Event]) -> serde_json::Value {
    serde_json::from_str(&body_text(outcome(ev))).unwrap()
}

fn var(v: &str, secret: bool) -> Var {
    Var {
        value: v.into(),
        secret,
        source: Source::new("t.env", Some(1)),
    }
}

fn scope_with(base: &str) -> VarScope {
    let mut s = VarScope::default();
    s.set(Layer::Env, "baseUrl", var(base, false));
    s
}

#[tokio::test]
async fn variables_inherited_headers_path_params_and_auth() {
    let s = http_server().await;
    let mut scope = scope_with(&format!("http://{}", s.addr));
    scope.set(Layer::Env, "id", var("a b", false));
    scope.inherited.headers = vec![
        ("X-Folder".into(), "f-{{id}}".into()),
        ("accept".into(), "text/inherited".into()),
    ];
    scope.inherited.auth = Some((
        None,
        vec![
            ("type".into(), "bearer".into()),
            ("token".into(), "tok-{{id}}".into()),
        ],
    ));
    // request overrides Accept (case-insensitive), inherits auth, `:id` falls back to the variable
    let src = "---\nname: t\n---\n\n```http\nGET {{baseUrl}}/echo/:id?x=1\nAccept: text/own\n```\n";
    // /echo/:id is not routed; use the echo path via a path param that stays in the path
    let ev = send_with(
        &src.replace("/echo/:id", "/echo"),
        &scope,
        ExecCtx::default(),
    )
    .await;
    let v = echo_json(&ev);
    assert_eq!(v["headers"]["accept"], "text/own");
    assert_eq!(v["headers"]["x-folder"], "f-a b");
    assert_eq!(v["headers"]["authorization"], "Bearer tok-a b");

    let plan = HttpProtocol.plan(&doc(src), &scope).unwrap();
    assert!(plan.label.ends_with("/echo/a%20b?x=1"), "{}", plan.label);
}

#[tokio::test]
async fn front_matter_path_overrides_variable_and_auth_none_blocks_inheritance() {
    let mut scope = scope_with("http://h.test");
    scope.set(Layer::Env, "id", var("fromvar", false));
    scope.inherited.auth = Some((
        None,
        vec![
            ("type".into(), "bearer".into()),
            ("token".into(), "t".into()),
        ],
    ));
    let src = "---\nname: t\npath:\n  id: fromfm\nauth: none\n---\n\n```http\nGET {{baseUrl}}/u/:id\n```\n";
    let plan = HttpProtocol.plan(&doc(src), &scope).unwrap();
    assert_eq!(plan.label, "GET http://h.test/u/fromfm");
    let ev = HttpProtocol
        .plan(&doc(&src.replace("auth: none\n", "")), &scope)
        .unwrap();
    assert_eq!(ev.label, "GET http://h.test/u/fromfm");
}

#[tokio::test]
async fn unresolved_variable_warns_and_unknown_scheme_errors() {
    let scope = VarScope::default();
    let src = "---\nname: t\n---\n\n```http\nGET http://127.0.0.1:1/{{nope}}\n```\n";
    let plan = HttpProtocol
        .plan(&doc(src), &scope)
        .expect("proceeds with a warning");
    assert_eq!(plan.warnings.len(), 1);
    assert_eq!(plan.warnings[0].code, "P101");
    let bad = "---\nname: t\n---\n\n```http\nGET http://127.0.0.1:1/{{vault:a#b}}\n```\n";
    let e = HttpProtocol.plan(&doc(bad), &scope).expect_err("error");
    assert!(e.iter().any(|d| d.code == "P105"));
    let url_missing = "---\nname: t\n---\n\n```http\nGET {{baseUrl}}/x\n```\n";
    let e = HttpProtocol
        .plan(&doc(url_missing), &scope)
        .expect_err("error");
    assert_eq!(e[0].code, "P101");
}

#[tokio::test]
async fn secrets_are_redacted_by_value_in_curl_but_not_when_asked() {
    let mut scope = scope_with("http://h.test");
    scope.set(Layer::LocalEnv, "pw", var("hunter2", true));
    let src = "---\nname: t\n---\n\n```http\nPOST {{baseUrl}}/p?k={{pw}}\nX-Custom: {{pw}}\n\nbody={{pw}}\n```\n";
    let plan = HttpProtocol.plan(&doc(src), &scope).unwrap();
    let masked = HttpProtocol.to_curl(&plan, true).unwrap();
    assert!(!masked.contains("hunter2"), "{masked}");
    assert!(masked.contains("‹secret:pw›"));
    let open = HttpProtocol.to_curl(&plan, false).unwrap();
    assert!(open.contains("hunter2"));
}

fn cookie_app() -> Router {
    Router::new()
        .route(
            "/set",
            get(|| async {
                let mut r = StatusCode::OK.into_response();
                r.headers_mut()
                    .append(header::SET_COOKIE, "sid=abc; Path=/".parse().unwrap());
                r.headers_mut()
                    .append(header::SET_COOKIE, "pref=dark; Path=/".parse().unwrap());
                r
            }),
        )
        .route(
            "/hop",
            get(|| async {
                let mut r = StatusCode::FOUND.into_response();
                r.headers_mut()
                    .insert(header::LOCATION, "/seen".parse().unwrap());
                r.headers_mut()
                    .insert(header::SET_COOKIE, "hop=1; Path=/".parse().unwrap());
                r
            }),
        )
        .route(
            "/seen",
            get(|h: HeaderMap| async move {
                h.get(header::COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            }),
        )
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{a}")
}

#[tokio::test]
async fn cookies_persist_per_environment_and_apply_on_redirect_hops() {
    let base = serve(cookie_app()).await;
    let env = EnvState::new("staging");
    let ctx = || ExecCtx {
        env: Some(env.clone()),
        ..ExecCtx::default()
    };
    let get_src = |p: &str| format!("---\nname: t\n---\n\n```http\nGET {base}{p}\n```\n");
    send_with(&get_src("/set"), &VarScope::default(), ctx()).await;
    assert_eq!(env.cookies.list().len(), 2);
    let ev = send_with(&get_src("/seen"), &VarScope::default(), ctx()).await;
    let text = body_text(outcome(&ev));
    assert!(
        text.contains("sid=abc") && text.contains("pref=dark"),
        "{text}"
    );

    // A cookie set on a redirect hop is sent on the next hop.
    let ev = send_with(&get_src("/hop"), &VarScope::default(), ctx()).await;
    assert!(body_text(outcome(&ev)).contains("hop=1"));

    // Another environment starts empty.
    let other = ExecCtx {
        env: Some(EnvState::new("local")),
        ..ExecCtx::default()
    };
    let ev = send_with(&get_src("/seen"), &VarScope::default(), other).await;
    assert_eq!(body_text(outcome(&ev)), "");

    env.cookies.clear();
    let ev = send_with(&get_src("/seen"), &VarScope::default(), ctx()).await;
    assert_eq!(body_text(outcome(&ev)), "");
}

#[derive(Default)]
struct Idp {
    grants: Vec<String>,
    challenge: Option<String>,
    issued: u32,
}

type Shared = Arc<Mutex<Idp>>;

fn oauth_err(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({"error": msg, "error_description": "nope"})),
    )
        .into_response()
}

async fn token(
    State(st): State<Shared>,
    h: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let basic_ok = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Basic {}", STANDARD.encode("cid:sec")));
    if !basic_ok && f.get("client_id").map(String::as_str) != Some("pub") {
        return oauth_err("invalid_client");
    }
    let grant = f.get("grant_type").cloned().unwrap_or_default();
    let mut s = st.lock().unwrap();
    s.grants.push(grant.clone());
    s.issued += 1;
    let n = s.issued;
    let ok = |access: String, expires: u32, refresh: Option<&str>| {
        let mut v = serde_json::json!({"access_token": access, "token_type": "Bearer", "expires_in": expires});
        if let Some(r) = refresh {
            v["refresh_token"] = r.into();
        }
        axum::Json(v).into_response()
    };
    match grant.as_str() {
        "client_credentials" if f.get("scope").map(String::as_str) == Some("short") => {
            ok(format!("cc-{n}"), 5, Some("r1"))
        }
        "client_credentials" => ok(format!("cc-{n}"), 3600, None),
        "password" => {
            if f.get("username").map(String::as_str) == Some("u")
                && f.get("password").map(String::as_str) == Some("p")
            {
                ok(format!("pw-{n}"), 3600, None)
            } else {
                oauth_err("invalid_grant")
            }
        }
        "refresh_token" => {
            if f.get("refresh_token").map(String::as_str) == Some("r1") {
                ok(format!("rf-{n}"), 3600, None)
            } else {
                oauth_err("invalid_grant")
            }
        }
        "authorization_code" => {
            if f.get("code").map(String::as_str) != Some("abc") || !f.contains_key("redirect_uri") {
                return oauth_err("invalid_grant");
            }
            if let Some(ch) = &s.challenge {
                let v = f.get("code_verifier").cloned().unwrap_or_default();
                if &URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())) != ch {
                    return oauth_err("invalid_grant");
                }
            }
            ok(format!("ac-{n}"), 3600, Some("r1"))
        }
        _ => oauth_err("unsupported_grant_type"),
    }
}

async fn idp() -> (String, Shared) {
    let st: Shared = Arc::default();
    let app = Router::new()
        .route("/token", post(token))
        .route("/echo", get(common::echo))
        .with_state(st.clone());
    (serve(app).await, st)
}

fn oauth_src(base: &str, extra: &str) -> String {
    format!(
        "---\nname: t\nauth:\n  type: oauth2\n  token_url: {base}/token\n  client_id: cid\n  client_secret: sec\n{extra}---\n\n```http\nGET {base}/echo\n```\n"
    )
}

fn bearer(ev: &[Event]) -> String {
    echo_json(ev)["headers"]["authorization"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn client_credentials_fetches_once_then_uses_the_environment_cache() {
    let (base, st) = idp().await;
    let env = EnvState::new("e");
    let ctx = || ExecCtx {
        env: Some(env.clone()),
        ..ExecCtx::default()
    };
    let src = oauth_src(&base, "  grant: client_credentials\n");
    let ev = send_with(&src, &VarScope::default(), ctx()).await;
    assert_eq!(bearer(&ev), "Bearer cc-1");
    let ev = send_with(&src, &VarScope::default(), ctx()).await;
    assert_eq!(bearer(&ev), "Bearer cc-1");
    assert_eq!(st.lock().unwrap().grants, ["client_credentials"]);

    // Another environment has its own cache.
    let other = ExecCtx {
        env: Some(EnvState::new("o")),
        ..ExecCtx::default()
    };
    let ev = send_with(&src, &VarScope::default(), other).await;
    assert_eq!(bearer(&ev), "Bearer cc-2");

    // The Auth tab can see and force-replace the token.
    let plan = HttpProtocol.plan(&doc(&src), &VarScope::default()).unwrap();
    let info = HttpProtocol.token_info(&plan, &env).unwrap();
    assert!(info.expires_at.is_some() && !info.has_refresh);
    HttpProtocol
        .fetch_token(plan, ctx(), true)
        .unwrap()
        .await
        .unwrap();
    let ev = send_with(&src, &VarScope::default(), ctx()).await;
    assert_eq!(bearer(&ev), "Bearer cc-3");
}

#[tokio::test]
async fn expiring_token_is_refreshed_automatically() {
    let (base, st) = idp().await;
    let env = EnvState::new("e");
    let ctx = || ExecCtx {
        env: Some(env.clone()),
        ..ExecCtx::default()
    };
    let src = oauth_src(&base, "  grant: client_credentials\n  scope: short\n");
    assert_eq!(
        bearer(&send_with(&src, &VarScope::default(), ctx()).await),
        "Bearer cc-1"
    );
    assert_eq!(
        bearer(&send_with(&src, &VarScope::default(), ctx()).await),
        "Bearer rf-2"
    );
    assert_eq!(
        st.lock().unwrap().grants,
        ["client_credentials", "refresh_token"]
    );
}

#[tokio::test]
async fn password_and_refresh_token_grants() {
    let (base, _) = idp().await;
    let pw = oauth_src(&base, "  grant: password\n  username: u\n  password: p\n");
    assert_eq!(
        bearer(&send_with(&pw, &VarScope::default(), ExecCtx::default()).await),
        "Bearer pw-1"
    );
    let rt = oauth_src(&base, "  grant: refresh_token\n  refresh_token: r1\n");
    assert!(
        bearer(&send_with(&rt, &VarScope::default(), ExecCtx::default()).await)
            .starts_with("Bearer rf-")
    );
    // A rejected grant surfaces as an auth-phase failure naming the fix.
    let bad = oauth_src(
        &base,
        "  grant: password\n  username: u\n  password: wrong\n",
    );
    let ev = send_with(&bad, &VarScope::default(), ExecCtx::default()).await;
    match ev.last() {
        Some(Event::Failed(e)) => {
            assert_eq!(e.phase, "auth");
            assert!(e.message.contains("invalid_grant"), "{e}");
            assert!(e.fix.is_some());
        }
        other => panic!("{other:?}"),
    }
}

async fn raw_get(url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let target = format!(
        "{}{}",
        u.path(),
        u.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    s.write_all(
        format!("GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out
}

/// Plays the browser: records the PKCE challenge like an IdP would, then follows the redirect.
fn browser(st: Shared, code: &'static str, tamper_state: bool) {
    let handle = tokio::runtime::Handle::current();
    oauth2::set_browser_opener(move |auth_url| {
        let u = url::Url::parse(auth_url).unwrap();
        let q: HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "cid");
        st.lock().unwrap().challenge = q.get("code_challenge").cloned();
        assert!(!q.contains_key("code_challenge") || q["code_challenge_method"] == "S256");
        let state = if tamper_state {
            "evil".to_string()
        } else {
            q["state"].clone()
        };
        let redirect = format!("{}?code={code}&state={state}", q["redirect_uri"]);
        handle.spawn(async move {
            raw_get(&redirect).await;
        });
    });
}

#[tokio::test]
async fn authorization_code_with_pkce_loopback_redirect() {
    let (base, st) = idp().await;
    let src = oauth_src(
        &base,
        &format!("  grant: authorization_code\n  auth_url: {base}/authorize\n  pkce: true\n"),
    );
    let env = EnvState::new("e");
    let ctx = || ExecCtx {
        env: Some(env.clone()),
        ..ExecCtx::default()
    };

    browser(st.clone(), "abc", false);
    let ev = send_with(&src, &VarScope::default(), ctx()).await;
    assert!(bearer(&ev).starts_with("Bearer ac-"));
    assert!(st.lock().unwrap().challenge.is_some());
    let plan = HttpProtocol.plan(&doc(&src), &VarScope::default()).unwrap();
    assert!(HttpProtocol.token_info(&plan, &env).unwrap().has_refresh);

    // A redirect with the wrong `state` is refused.
    browser(st.clone(), "abc", true);
    let fresh = ExecCtx {
        env: Some(EnvState::new("x")),
        ..ExecCtx::default()
    };
    let ev = send_with(&src, &VarScope::default(), fresh).await;
    match ev.last() {
        Some(Event::Failed(e)) => assert!(e.message.contains("state"), "{e}"),
        other => panic!("{other:?}"),
    }

    // No redirect within the window: a clear timeout error, not a hang.
    oauth2::set_browser_opener(|_| {});
    let cfg = fluxwire_http::OAuth2Config {
        grant: "authorization_code".into(),
        token_url: format!("{base}/token"),
        auth_url: format!("{base}/authorize"),
        client_id: "cid".into(),
        client_secret: "sec".into(),
        ..Default::default()
    };
    let r = oauth2::ensure_token_with(
        &cfg,
        &ExecCtx::default(),
        true,
        Options {
            callback_timeout: Duration::from_millis(200),
        },
    )
    .await;
    assert!(r.unwrap_err().message.contains("within"));
}
