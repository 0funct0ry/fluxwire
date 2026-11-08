//! OAuth2 token acquisition (SPEC §9.4): client_credentials, authorization_code (+PKCE,
//! loopback redirect), password and refresh_token, with a per-environment cache.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use bytes::Bytes;
use fluxwire_protocol::{Body, EnvState, Event, ExecCtx, ExecError, Plan, Token, now_unix};
use rand::RngExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::auth::{Auth, OAuth2Config};
use crate::plan::{HttpPlan, SettingsOverride};

/// How long the loopback listener waits for the browser redirect (PROMPTS.md M4).
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// Refresh this many seconds before expiry.
const SKEW: i64 = 30;

type Opener = Arc<dyn Fn(&str) + Send + Sync>;

fn opener() -> &'static Mutex<Option<Opener>> {
    static O: OnceLock<Mutex<Option<Opener>>> = OnceLock::new();
    O.get_or_init(|| Mutex::new(None))
}

/// Replace how the authorization URL is opened (tests drive the "browser" themselves).
pub fn set_browser_opener(f: impl Fn(&str) + Send + Sync + 'static) {
    *opener().lock().unwrap() = Some(Arc::new(f));
}

fn open_browser(url: &str) {
    let hook = opener().lock().unwrap().clone();
    if let Some(f) = hook {
        return f(url);
    }
    let _ = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
}

fn auth_err(message: impl Into<String>, fix: impl Into<String>) -> ExecError {
    ExecError {
        phase: "auth".into(),
        message: message.into(),
        fix: Some(fix.into()),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub callback_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            callback_timeout: CALLBACK_TIMEOUT,
        }
    }
}

/// A valid token from the cache, a refresh, or a fresh grant. `force` skips the cache.
pub async fn ensure_token(
    cfg: &OAuth2Config,
    ctx: &ExecCtx,
    force: bool,
) -> Result<Token, ExecError> {
    ensure_token_with(cfg, ctx, force, Options::default()).await
}

pub async fn ensure_token_with(
    cfg: &OAuth2Config,
    ctx: &ExecCtx,
    force: bool,
    opts: Options,
) -> Result<Token, ExecError> {
    let env = ctx.env.clone().unwrap_or_default();
    let key = cfg.cache_key();
    let cached = env.tokens.get(&key);
    if !force
        && let Some(t) = &cached
        && !t.is_stale(SKEW)
    {
        return Ok(t.clone());
    }
    // Stale (or forced) token with a refresh token: try silently first.
    let refresh = cached
        .as_ref()
        .and_then(|t| t.refresh_token.clone())
        .or_else(|| (!cfg.refresh_token.is_empty()).then(|| cfg.refresh_token.clone()));
    if (cfg.grant == "refresh_token" || cached.is_some())
        && let Some(rt) = refresh
    {
        match request_token(
            cfg,
            ctx,
            vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", rt.clone()),
            ],
        )
        .await
        {
            Ok(mut t) => {
                t.refresh_token.get_or_insert(rt);
                env.tokens.put(&key, t.clone());
                return Ok(t);
            }
            Err(e) if cfg.grant == "refresh_token" => return Err(e),
            Err(_) => env.tokens.remove(&key), // fall back to a full grant
        }
    }
    let t = match cfg.grant.as_str() {
        "client_credentials" => {
            request_token(cfg, ctx, vec![("grant_type", "client_credentials".into())]).await?
        }
        "password" => {
            request_token(
                cfg,
                ctx,
                vec![
                    ("grant_type", "password".into()),
                    ("username", cfg.username.clone()),
                    ("password", cfg.password.clone()),
                ],
            )
            .await?
        }
        "refresh_token" => {
            return Err(auth_err(
                "The refresh_token grant needs a `refresh_token` value.",
                "Set `refresh_token` in the auth block, or use authorization_code to obtain one.",
            ));
        }
        "authorization_code" => authorization_code(cfg, ctx, opts).await?,
        g => {
            return Err(auth_err(
                format!("Unknown oauth2 grant `{g}`."),
                "Use client_credentials, authorization_code, password or refresh_token.",
            ));
        }
    };
    env.tokens.put(&key, t.clone());
    Ok(t)
}

async fn authorization_code(
    cfg: &OAuth2Config,
    ctx: &ExecCtx,
    opts: Options,
) -> Result<Token, ExecError> {
    let listener = TcpListener::bind(("127.0.0.1", cfg.redirect_port))
        .await
        .map_err(|e| {
            auth_err(
                format!(
                    "Cannot listen on 127.0.0.1:{} for the redirect: {e}.",
                    cfg.redirect_port
                ),
                "Pick a free `redirect_port`, or omit it to use a random one.",
            )
        })?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state: String = hex(&rand::rng().random::<[u8; 16]>());
    let verifier = URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 48]>());

    let mut auth_url = Url::parse(&cfg.auth_url).map_err(|e| {
        auth_err(
            format!("Invalid auth_url `{}`: {e}.", cfg.auth_url),
            "Use a full URL such as https://idp.example.com/authorize.",
        )
    })?;
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &cfg.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("state", &state);
        if !cfg.scope.is_empty() {
            q.append_pair("scope", &cfg.scope);
        }
        if !cfg.audience.is_empty() {
            q.append_pair("audience", &cfg.audience);
        }
        if cfg.pkce {
            q.append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            )
            .append_pair("code_challenge_method", "S256");
        }
    }
    open_browser(auth_url.as_str());

    let code = tokio::select! {
        _ = ctx.cancel.cancelled() => return Err(auth_err("Cancelled while waiting for the browser login.", "Send again to retry.")),
        r = tokio::time::timeout(opts.callback_timeout, wait_for_callback(&listener, &state)) => match r {
            Ok(r) => r?,
            Err(_) => return Err(auth_err(
                format!("No login redirect arrived within {} seconds.", opts.callback_timeout.as_secs()),
                "Click \"Fetch new token\" and finish the login in your browser.",
            )),
        },
    };
    drop(listener);

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];
    if cfg.pkce {
        form.push(("code_verifier", verifier));
    }
    request_token(cfg, ctx, form).await
}

/// Accept connections until one carries `code` or `error`; answer strays with 404.
async fn wait_for_callback(listener: &TcpListener, state: &str) -> Result<String, ExecError> {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else {
            continue;
        };
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        while n < buf.len() {
            match sock.read(&mut buf[n..]).await {
                Ok(0) | Err(_) => break,
                Ok(k) => {
                    n += k;
                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&buf[..n]).into_owned();
        let target = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let parsed = Url::parse(&format!("http://127.0.0.1{target}")).ok();
        let q: Vec<(String, String)> = parsed
            .as_ref()
            .map(|u| {
                u.query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let is_callback = parsed.as_ref().is_some_and(|u| u.path() == "/callback")
            && (get("code").is_some() || get("error").is_some());
        let (status, page) = if is_callback {
            (
                "200 OK",
                "<html><body><h3>Fluxwire</h3><p>Login complete. You can close this window.</p></body></html>",
            )
        } else {
            ("404 Not Found", "not found")
        };
        let _ = sock
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = sock.shutdown().await;
        if !is_callback {
            continue;
        }
        if let Some(e) = get("error") {
            let d = get("error_description").unwrap_or_default();
            return Err(auth_err(
                format!("The login was refused: {e} {d}").trim().to_string(),
                "Check the client's redirect URI and scopes at the identity provider.",
            ));
        }
        if get("state").as_deref() != Some(state) {
            return Err(auth_err(
                "The login redirect had the wrong `state`.",
                "Start the login again; do not reuse an old browser tab.",
            ));
        }
        return Ok(get("code").unwrap_or_default());
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// POST the token request, adding client authentication and `scope`/`audience`.
async fn request_token(
    cfg: &OAuth2Config,
    ctx: &ExecCtx,
    mut form: Vec<(&str, String)>,
) -> Result<Token, ExecError> {
    let mut headers = vec![
        (
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        ),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    if cfg.client_secret.is_empty() {
        form.push(("client_id", cfg.client_id.clone()));
    } else {
        headers.push((
            "Authorization".into(),
            format!(
                "Basic {}",
                STANDARD.encode(format!(
                    "{}:{}",
                    enc(&cfg.client_id),
                    enc(&cfg.client_secret)
                ))
            ),
        ));
    }
    let is_refresh = form
        .iter()
        .any(|(k, v)| *k == "grant_type" && v == "refresh_token");
    if !cfg.scope.is_empty() && !is_refresh {
        form.push(("scope", cfg.scope.clone()));
    }
    if !cfg.audience.is_empty() {
        form.push(("audience", cfg.audience.clone()));
    }
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str())))
        .finish();
    let url = Url::parse(&cfg.token_url).map_err(|e| {
        auth_err(
            format!("Invalid token_url `{}`: {e}.", cfg.token_url),
            "Use a full URL such as https://idp.example.com/oauth/token.",
        )
    })?;
    let http = HttpPlan {
        method: "POST".into(),
        url,
        headers,
        body: Bytes::from(body.into_bytes()),
        auth: Auth::None,
        settings: SettingsOverride::default(),
        warnings: Vec::new(),
        secrets: Vec::new(),
    };
    let plan = Plan::new("http", format!("POST {}", cfg.token_url), http);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut sub = ctx.clone();
    sub.env = None; // token endpoints never share the environment's cookies
    Box::pin(crate::engine::run(plan, sub, tx)).await;
    let mut outcome = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Done(o) => outcome = Some(*o),
            Event::Failed(e) => {
                return Err(ExecError {
                    phase: "auth".into(),
                    message: format!("Token request failed ({}): {}", e.phase, e.message),
                    fix: e.fix.or(Some("Check token_url and your network.".into())),
                });
            }
            _ => {}
        }
    }
    let x = outcome.and_then(|o| o.exchange).ok_or_else(|| {
        auth_err(
            "The token endpoint returned no response.",
            "Check token_url.",
        )
    })?;
    let bytes = match &x.body {
        Body::Memory(b) => b.clone(),
        Body::File { .. } => {
            return Err(auth_err(
                "The token response was unexpectedly large.",
                "Check token_url.",
            ));
        }
    };
    parse_token_response(x.status, &bytes)
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn parse_token_response(status: u16, body: &[u8]) -> Result<Token, ExecError> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
        auth_err(
            format!("The token endpoint answered {status} with a body that is not JSON."),
            "Check token_url; it must be the token endpoint, not the authorize page.",
        )
    })?;
    if !(200..300).contains(&status) || v.get("error").is_some() {
        let e = v.get("error").and_then(|e| e.as_str()).unwrap_or("error");
        let d = v
            .get("error_description")
            .and_then(|e| e.as_str())
            .unwrap_or("");
        return Err(auth_err(
            format!("The token endpoint answered {status}: {e} {d}")
                .trim()
                .to_string(),
            "Check client_id, client_secret, scope and grant type.",
        ));
    }
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| {
            auth_err(
                "The token response had no access_token.",
                "Check the grant type and scope.",
            )
        })?;
    let expires_in = v.get("expires_in").and_then(|e| {
        e.as_i64()
            .or_else(|| e.as_str().and_then(|s| s.parse().ok()))
    });
    Ok(Token {
        access_token: access.to_string(),
        token_type: v
            .get("token_type")
            .and_then(|t| t.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .map(str::to_string),
        expires_at: expires_in.map(|s| now_unix() + s),
        scope: v.get("scope").and_then(|t| t.as_str()).map(str::to_string),
    })
}

/// Cached token for this config in `env`, if any.
pub fn cached_token(cfg: &OAuth2Config, env: &EnvState) -> Option<Token> {
    env.tokens.get(&cfg.cache_key())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_success_and_errors() {
        let t = parse_token_response(
            200,
            br#"{"access_token":"a","expires_in":"60","refresh_token":"r"}"#,
        )
        .unwrap();
        assert_eq!(t.access_token, "a");
        assert_eq!(t.token_type, "Bearer");
        assert!(t.expires_at.unwrap() > now_unix());
        let e = parse_token_response(
            400,
            br#"{"error":"invalid_client","error_description":"bad"}"#,
        )
        .unwrap_err();
        assert!(e.message.contains("invalid_client"));
        assert!(parse_token_response(200, b"<html>").is_err());
        assert!(parse_token_response(200, br#"{"x":1}"#).is_err());
    }
}
