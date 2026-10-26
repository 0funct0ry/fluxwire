//! WebSocket protocol engine (SPEC §10): the first `Interaction::Session` protocol.

mod engine;
mod error;
mod history;
mod plan;
mod tls;

use std::sync::OnceLock;

use fluxwire_model::{Diagnostic, ExpectBlock, FluxDoc, StatusMatch};
use fluxwire_protocol::{
    CheckCtx, Event, ExecCtx, HistoryRecord, Interaction, Outcome, Plan, Protocol,
    ProtocolRegistry, RoleSpec, TestResult,
};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use futures::stream::BoxStream;

pub use plan::{Auth, WsPlan};

pub struct WsProtocol;

fn info() -> &'static fluxwire_protocol::ProtocolInfo {
    static INFO: OnceLock<fluxwire_protocol::ProtocolInfo> = OnceLock::new();
    INFO.get_or_init(|| {
        ProtocolRegistry::builtin()
            .protocols()
            .iter()
            .find(|p| p.id == "ws")
            .expect("ws is registered")
            .clone()
    })
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Protocol for WsProtocol {
    fn id(&self) -> &'static str {
        "ws"
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
        Interaction::Session
    }

    fn plan(&self, doc: &FluxDoc, scope: &VarScope) -> Result<Plan, Vec<Diagnostic>> {
        let p = plan::plan(doc, scope)?;
        let label = format!("WS {}", p.display_url);
        let warnings = p.warnings.clone();
        let mut plan = Plan::new("ws", label, p);
        plan.warnings = warnings;
        Ok(plan)
    }

    fn execute(&self, plan: Plan, ctx: ExecCtx) -> BoxStream<'static, Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let guard = AbortOnDrop(tokio::spawn(engine::run(plan, ctx, tx)));
        futures::stream::unfold((rx, guard), |(mut rx, g)| async move {
            rx.recv().await.map(|e| (e, (rx, g)))
        })
        .boxed()
    }

    /// Checks the upgrade response (`HTTP 101`); expectations over frames are reserved for a
    /// later `ws expect` role (SPEC §10).
    fn check(&self, result: &Outcome, expect: &ExpectBlock, cx: &CheckCtx<'_>) -> Vec<TestResult> {
        let Some(x) = &result.exchange else {
            return vec![TestResult {
                name: "upgrade response received".into(),
                passed: false,
                expected: None,
                actual: None,
                message: Some("There was no upgrade response to check.".into()),
            }];
        };
        let mut out = Vec::new();
        let (ok, want) = match expect.status {
            StatusMatch::Exact(c) => (x.status == c, c.to_string()),
            StatusMatch::Class(c) => (x.status / 100 == c as u16, format!("{c}xx")),
            StatusMatch::Any => (true, "any".to_string()),
        };
        out.push(TestResult {
            name: "expect status".into(),
            passed: ok,
            expected: Some(want),
            actual: Some(x.status.to_string()),
            message: None,
        });
        for h in &expect.headers {
            let want = fluxwire_vars::interpolate(&h.value, cx.scope).text;
            let want = want.trim();
            let actual = x.header(&h.name);
            let passed = match actual {
                None => false,
                Some(_) if want == "{{*}}" => true,
                Some(a) => match want.strip_prefix("~/").and_then(|r| r.strip_suffix('/')) {
                    Some(re) => regex::Regex::new(re).is_ok_and(|re| re.is_match(a)),
                    None => a.trim() == want,
                },
            };
            out.push(TestResult {
                name: format!("expect header {}", h.name),
                passed,
                expected: Some(want.to_string()),
                actual: actual.map(str::to_string),
                message: None,
            });
        }
        out
    }

    fn history_record(&self, r: &Outcome) -> HistoryRecord {
        history::record(r)
    }

    fn history_request(&self, plan: &Plan) -> serde_json::Value {
        plan.payload::<WsPlan>()
            .map(history::request_snapshot)
            .unwrap_or_default()
    }

    fn history_secrets(&self, plan: &Plan) -> Vec<(String, String)> {
        plan.payload::<WsPlan>()
            .map(history::secrets)
            .unwrap_or_default()
    }

    fn history_to_outcome(&self, record: &HistoryRecord) -> Option<Outcome> {
        history::to_outcome(record)
    }
}
