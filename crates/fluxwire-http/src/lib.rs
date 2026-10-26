//! HTTP protocol engine (SPEC §9): `Protocol` implementation on hyper 1.x + rustls.

mod auth;
mod body;
mod curl;
mod engine;
mod error;
mod plan;
mod tls;

use std::sync::OnceLock;

use fluxwire_model::{Diagnostic, Edit, ExpectBlock, FluxDoc, StatusMatch};
use fluxwire_protocol::{
    Event, ExecCtx, HistoryRecord, Interaction, Outcome, Plan, Protocol, ProtocolRegistry,
    RoleSpec, TestResult,
};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use futures::stream::BoxStream;

pub use auth::Auth;
pub use plan::HttpPlan;

pub struct HttpProtocol;

fn info() -> &'static fluxwire_protocol::ProtocolInfo {
    static INFO: OnceLock<fluxwire_protocol::ProtocolInfo> = OnceLock::new();
    INFO.get_or_init(|| {
        ProtocolRegistry::builtin()
            .protocols()
            .iter()
            .find(|p| p.id == "http")
            .expect("http is registered")
            .clone()
    })
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Protocol for HttpProtocol {
    fn id(&self) -> &'static str {
        "http"
    }

    fn feature(&self) -> Option<&'static str> {
        None
    }

    fn request_languages(&self) -> &[&'static str] {
        info().request_languages
    }

    fn roles(&self) -> &[RoleSpec] {
        info().roles
    }

    fn interaction(&self) -> Interaction {
        Interaction::Exchange
    }

    fn plan(&self, doc: &FluxDoc, scope: &VarScope) -> Result<Plan, Vec<Diagnostic>> {
        let p = plan::plan(doc, scope)?;
        let label = format!("{} {}", p.method, p.url);
        Ok(Plan::new("http", label, p))
    }

    fn execute(&self, plan: Plan, ctx: ExecCtx) -> BoxStream<'static, Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let guard = AbortOnDrop(tokio::spawn(engine::run(plan, ctx, tx)));
        futures::stream::unfold((rx, guard), |(mut rx, g)| async move {
            rx.recv().await.map(|e| (e, (rx, g)))
        })
        .boxed()
    }

    fn check(&self, result: &Outcome, expect: &ExpectBlock) -> Vec<TestResult> {
        let Some(x) = &result.exchange else {
            return vec![TestResult {
                name: "response received".into(),
                passed: false,
                expected: None,
                actual: None,
                message: Some("There was no response to check.".into()),
            }];
        };
        let mut out = Vec::new();
        let status_ok = match expect.status {
            StatusMatch::Exact(c) => (x.status == c, format!("status is {c}")),
            StatusMatch::Class(c) => (x.status / 100 == c as u16, format!("status is {c}xx")),
            StatusMatch::Any => (true, "status is any".to_string()),
        };
        out.push(TestResult {
            name: status_ok.1.clone(),
            passed: status_ok.0,
            expected: Some(status_ok.1.trim_start_matches("status is ").to_string()),
            actual: Some(x.status.to_string()),
            message: None,
        });
        if let Some(v) = &expect.version {
            let want = v.trim_start_matches("HTTP/").trim_end_matches(".0");
            let have = x.version.trim_start_matches("HTTP/");
            out.push(TestResult {
                name: format!("version is {v}"),
                passed: want == have,
                expected: Some(v.clone()),
                actual: Some(x.version.clone()),
                message: None,
            });
        }
        for h in &expect.headers {
            let actual = x.header(&h.name);
            let passed = match actual {
                None => false,
                Some(_) if h.value.trim() == "{{*}}" => true,
                Some(a) => match h
                    .value
                    .trim()
                    .strip_prefix("~/")
                    .and_then(|r| r.strip_suffix('/'))
                {
                    Some(re) => regex::Regex::new(re).is_ok_and(|re| re.is_match(a)),
                    None => a.trim() == h.value.trim(),
                },
            };
            out.push(TestResult {
                name: format!("header {} matches", h.name),
                passed,
                expected: Some(h.value.clone()),
                actual: actual.map(str::to_string),
                message: None,
            });
        }
        out
    }

    fn history_record(&self, r: &Outcome) -> HistoryRecord {
        let x = r.exchange.as_ref();
        HistoryRecord {
            protocol: "http".into(),
            summary: format!(
                "{} {}",
                x.map(|x| x.status.to_string())
                    .unwrap_or_else(|| "-".into()),
                r.final_url
            ),
            payload: serde_json::json!({
                "url": r.final_url,
                "status": x.map(|x| x.status),
                "total_ms": r.total_ms,
                "wire_size": x.map(|x| x.wire_size),
                "decoded_size": x.map(|x| x.decoded_size),
                "redirects": r.redirects.len(),
            }),
        }
    }

    fn to_curl(&self, plan: &Plan, mask_secrets: bool) -> Option<String> {
        plan.payload::<HttpPlan>()
            .map(|p| curl::to_curl(p, mask_secrets))
    }

    fn edits_from_paste(&self, text: &str, doc: &FluxDoc) -> Option<Vec<Edit>> {
        curl::edits_from_paste(text, doc)
    }
}
