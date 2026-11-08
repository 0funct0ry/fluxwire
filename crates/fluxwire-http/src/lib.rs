//! HTTP protocol engine (SPEC §9): `Protocol` implementation on hyper 1.x + rustls.

use std::sync::OnceLock;

use fluxwire_httpcore::{curl, expect, history, script};
use fluxwire_model::{Diagnostic, Edit, ExpectBlock, FluxDoc};
use fluxwire_protocol::{
    CheckCtx, Event, ExecCtx, HistoryRecord, Interaction, Outcome, Plan, Protocol,
    ProtocolRegistry, RoleSpec, TestResult,
};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use futures::stream::BoxStream;

pub use fluxwire_httpcore::{Auth, HttpPlan, OAuth2Config, get_text, oauth2};

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
        let p = fluxwire_httpcore::plan(doc, scope)?;
        let label = format!("{} {}", p.method, p.url);
        let warnings = p.warnings.clone();
        let mut plan = Plan::new("http", label, p);
        plan.warnings = warnings;
        Ok(plan)
    }

    fn execute(&self, plan: Plan, ctx: ExecCtx) -> BoxStream<'static, Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let guard = AbortOnDrop(tokio::spawn(fluxwire_httpcore::run(plan, ctx, tx)));
        futures::stream::unfold((rx, guard), |(mut rx, g)| async move {
            rx.recv().await.map(|e| (e, (rx, g)))
        })
        .boxed()
    }

    fn check(&self, result: &Outcome, expect: &ExpectBlock, cx: &CheckCtx<'_>) -> Vec<TestResult> {
        let Some(x) = &result.exchange else {
            return vec![TestResult {
                name: "response received".into(),
                passed: false,
                expected: None,
                actual: None,
                message: Some("There was no response to check.".into()),
            }];
        };
        expect::check(x, expect, cx)
    }

    fn script_request(
        &self,
        doc: &FluxDoc,
        scope: &VarScope,
        plan: Option<&Plan>,
    ) -> Option<serde_json::Value> {
        script::request_view(doc, scope, plan)
    }

    fn apply_script_ops(&self, plan: &Plan, ops: &[serde_json::Value]) -> Result<Plan, String> {
        script::apply_ops(plan, ops)
    }

    fn script_response(&self, result: &Outcome) -> Option<serde_json::Value> {
        script::response_view(result)
    }

    fn capture(&self, result: &Outcome, expr: &str) -> Result<Option<String>, String> {
        script::capture(result, expr)
    }

    fn history_record(&self, r: &Outcome) -> HistoryRecord {
        history::record(r)
    }

    fn history_request(&self, plan: &Plan) -> serde_json::Value {
        plan.payload::<HttpPlan>()
            .map(history::request_snapshot)
            .unwrap_or_default()
    }

    fn history_secrets(&self, plan: &Plan) -> Vec<(String, String)> {
        plan.payload::<HttpPlan>()
            .map(history::secrets)
            .unwrap_or_default()
    }

    fn history_to_curl(&self, record: &HistoryRecord) -> Option<String> {
        history::to_curl(record)
    }

    fn history_to_outcome(&self, record: &HistoryRecord) -> Option<Outcome> {
        history::to_outcome(record)
    }

    fn token_info(
        &self,
        plan: &Plan,
        env: &fluxwire_protocol::EnvState,
    ) -> Option<fluxwire_protocol::TokenInfo> {
        match &plan.payload::<HttpPlan>()?.auth {
            fluxwire_httpcore::Auth::OAuth2(cfg) => {
                oauth2::cached_token(cfg, env).as_ref().map(Into::into)
            }
            _ => None,
        }
    }

    fn fetch_token(
        &self,
        plan: Plan,
        ctx: ExecCtx,
        force: bool,
    ) -> Option<
        futures::future::BoxFuture<
            'static,
            Result<fluxwire_protocol::TokenInfo, fluxwire_protocol::ExecError>,
        >,
    > {
        let fluxwire_httpcore::Auth::OAuth2(cfg) = plan.payload::<HttpPlan>()?.auth.clone() else {
            return None;
        };
        Some(Box::pin(async move {
            let t = oauth2::ensure_token(&cfg, &ctx, force).await?;
            Ok((&t).into())
        }))
    }

    fn to_curl(&self, plan: &Plan, mask_secrets: bool) -> Option<String> {
        plan.payload::<HttpPlan>()
            .map(|p| curl::to_curl(p, mask_secrets))
    }

    fn edits_from_paste(&self, text: &str, doc: &FluxDoc) -> Option<Vec<Edit>> {
        curl::edits_from_paste(text, doc)
    }

    fn preview_paste(&self, text: &str) -> Option<fluxwire_protocol::PasteSummary> {
        curl::preview_curl(text)
    }

    fn label(&self) -> &'static str {
        "HTTP"
    }

    fn url_schemes(&self) -> &[&'static str] {
        &["http://", "https://"]
    }
}
