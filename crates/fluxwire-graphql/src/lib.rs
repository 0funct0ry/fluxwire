//! GraphQL protocol (SPEC §7.9, ADR-025): an `http` request block plus `graphql query` and
//! `json variables` blocks. It plans with the shared HTTP planner, builds the GraphQL body, and
//! sends through the shared engine (`fluxwire-httpcore`). It never depends on `fluxwire-http`.

pub mod builder;
pub mod complete;
pub mod operation;
pub mod parse;
pub mod schema;
pub mod validate;

use std::sync::OnceLock;

use fluxwire_httpcore::{self as core, HttpPlan, curl, expect, history, script};
use fluxwire_model::{BlockKind, Diagnostic, ExpectBlock, FluxDoc};
use fluxwire_protocol::{
    CheckCtx, Event, ExecCtx, HistoryRecord, Interaction, Outcome, Plan, Protocol,
    ProtocolRegistry, RoleSpec, TestResult,
};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use futures::stream::BoxStream;

pub use operation::Operation;

pub struct GraphqlProtocol;

fn info() -> &'static fluxwire_protocol::ProtocolInfo {
    static INFO: OnceLock<fluxwire_protocol::ProtocolInfo> = OnceLock::new();
    INFO.get_or_init(|| {
        ProtocolRegistry::builtin()
            .protocols()
            .iter()
            .find(|p| p.id == "graphql")
            .expect("graphql is registered")
            .clone()
    })
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn has_header(h: &[(String, String)], name: &str) -> bool {
    h.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

/// Apply the operation to a planned HTTP request: JSON body for POST, query string for GET.
pub fn apply_operation(plan: &mut HttpPlan, op: &Operation) {
    if !has_header(&plan.headers, "accept") {
        plan.headers.push((
            "Accept".into(),
            "application/graphql-response+json, application/json".into(),
        ));
    }
    if plan.method.eq_ignore_ascii_case("GET") {
        let mut pairs = plan.url.query_pairs_mut();
        pairs.append_pair("query", &op.query);
        if let Some(v) = &op.variables {
            pairs.append_pair(
                "variables",
                &serde_json::Value::Object(v.clone()).to_string(),
            );
        }
        if let Some(n) = &op.operation_name {
            pairs.append_pair("operationName", n);
        }
        return;
    }
    plan.body = bytes::Bytes::from(op.to_json().to_string());
    plan.headers
        .retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
    plan.headers
        .push(("Content-Type".into(), "application/json".into()));
}

pub(crate) fn plan_doc(doc: &FluxDoc, scope: &VarScope) -> Result<HttpPlan, Vec<Diagnostic>> {
    let mut plan = core::plan(doc, scope)?;
    let mut warnings = Vec::new();
    let op = operation::from_doc(doc, scope, &mut warnings)?;
    apply_operation(&mut plan, &op);
    plan.warnings.extend(warnings);
    Ok(plan)
}

/// A plan that sends the introspection query to the endpoint of `doc` (its URL, headers and
/// auth), whatever the document's own query says. Run it with `GraphqlProtocol::execute`.
pub fn introspection_plan(doc: &FluxDoc, scope: &VarScope) -> Result<Plan, Vec<Diagnostic>> {
    let mut p = core::plan(doc, scope)?;
    p.method = "POST".into();
    let op = Operation {
        query: schema::INTROSPECTION_QUERY.into(),
        variables: None,
        operation_name: Some("IntrospectionQuery".into()),
    };
    apply_operation(&mut p, &op);
    let label = format!("introspect {}", p.url);
    Ok(Plan::new("graphql", label, p))
}

/// The schema in an introspection outcome, with the raw JSON for the cache.
pub fn schema_from_outcome(o: &Outcome) -> Result<(schema::Schema, serde_json::Value), String> {
    let x = o.exchange.as_ref().ok_or("there was no response")?;
    if !(200..300).contains(&x.status) {
        return Err(format!(
            "the server answered {} {}",
            x.status, x.status_text
        ));
    }
    let bytes = x.body.read_prefix(64 << 20).map_err(|e| e.to_string())?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("the response is not JSON: {e}"))?;
    if let Some(msg) = v
        .pointer("/errors/0/message")
        .and_then(serde_json::Value::as_str)
        .filter(|_| v.pointer("/data/__schema").is_none())
    {
        return Err(msg.to_string());
    }
    schema::Schema::from_introspection(&v).map(|s| (s, v))
}

/// Run an introspection plan to completion and build the schema. Must run inside Tokio.
pub async fn fetch_schema(
    plan: Plan,
    ctx: ExecCtx,
) -> Result<(schema::Schema, serde_json::Value), String> {
    let events: Vec<Event> = GraphqlProtocol.execute(plan, ctx).collect().await;
    match events.into_iter().last() {
        Some(Event::Done(o)) => schema_from_outcome(&o),
        Some(Event::Failed(e)) => Err(match e.fix {
            Some(f) => format!("{} {f}", e.message),
            None => e.message,
        }),
        _ => Err("the request produced no result".into()),
    }
}

impl Protocol for GraphqlProtocol {
    fn id(&self) -> &'static str {
        "graphql"
    }

    fn feature(&self) -> Option<&'static str> {
        Some("graphql")
    }

    fn request_languages(&self) -> &[&'static str] {
        // Listed after `http` in the runner, so the HTTP protocol wins the language lookup
        // and this one is chosen through `claims`.
        info().request_languages
    }

    fn roles(&self) -> &[RoleSpec] {
        info().roles
    }

    fn interaction(&self) -> Interaction {
        Interaction::Exchange
    }

    fn claims(&self, doc: &FluxDoc) -> bool {
        doc.blocks
            .iter()
            .any(|b| b.top_level && b.kind == BlockKind::GraphqlQuery)
    }

    fn plan(&self, doc: &FluxDoc, scope: &VarScope) -> Result<Plan, Vec<Diagnostic>> {
        let p = plan_doc(doc, scope)?;
        let label = format!("{} {}", p.method, p.url);
        let warnings = p.warnings.clone();
        let mut plan = Plan::new("graphql", label, p);
        plan.warnings = warnings;
        Ok(plan)
    }

    fn execute(&self, plan: Plan, ctx: ExecCtx) -> BoxStream<'static, Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let guard = AbortOnDrop(tokio::spawn(core::run(plan, ctx, tx)));
        futures::stream::unfold((rx, guard), |(mut rx, g)| async move {
            rx.recv().await.map(|e| (e, (rx, g)))
        })
        // The shared engine labels its outcome `http`; this protocol owns the result.
        .map(|e| match e {
            Event::Done(mut o) => {
                o.protocol = "graphql";
                Event::Done(o)
            }
            other => other,
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
        expect::check_opts(x, expect, cx, true)
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
        let mut rec = history::record(r);
        rec.protocol = "graphql".into();
        rec
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

    fn to_curl(&self, plan: &Plan, mask_secrets: bool) -> Option<String> {
        plan.payload::<HttpPlan>()
            .map(|p| curl::to_curl(p, mask_secrets))
    }

    fn label(&self) -> &'static str {
        "GraphQL"
    }

    fn url_schemes(&self) -> &[&'static str] {
        &[]
    }
}
