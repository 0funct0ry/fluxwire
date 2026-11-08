//! A plain GET through the normal engine (TLS, proxy and CA settings apply). Used by the
//! update check; sends no cookies and nothing but a `User-Agent` and `Accept` header.

use bytes::Bytes;
use fluxwire_protocol::{Body, Event, ExecCtx, NetConfig, Plan};
use url::Url;

use crate::auth::Auth;
use crate::plan::{HttpPlan, SettingsOverride};

/// GET `url`; returns the status and body text, or a one-line failure message.
pub async fn get_text(
    url: &str,
    user_agent: &str,
    net: NetConfig,
) -> Result<(u16, String), String> {
    let parsed = Url::parse(url).map_err(|e| format!("Invalid URL `{url}`: {e}"))?;
    let http = HttpPlan {
        method: "GET".into(),
        url: parsed,
        headers: vec![
            ("User-Agent".into(), user_agent.into()),
            ("Accept".into(), "application/vnd.github+json".into()),
        ],
        body: Bytes::new(),
        auth: Auth::None,
        settings: SettingsOverride::default(),
        warnings: Vec::new(),
        secrets: Vec::new(),
    };
    let plan = Plan::new("http", format!("GET {url}"), http);
    let ctx = ExecCtx {
        net,
        ..ExecCtx::default()
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    Box::pin(crate::engine::run(plan, ctx, tx)).await;
    let mut outcome = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Done(o) => outcome = Some(*o),
            Event::Failed(e) => return Err(format!("{}: {}", e.phase, e.message)),
            _ => {}
        }
    }
    let x = outcome
        .and_then(|o| o.exchange)
        .ok_or("No response received.")?;
    match &x.body {
        Body::Memory(b) => Ok((x.status, String::from_utf8_lossy(b).into_owned())),
        Body::File { .. } => Err("The response was unexpectedly large.".into()),
    }
}
