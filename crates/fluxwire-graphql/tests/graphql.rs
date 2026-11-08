use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::routing::{get, post};
use axum::{Json, http::HeaderMap};
use fluxwire_graphql::GraphqlProtocol;
use fluxwire_model::{BlockKind, FluxDoc};
use fluxwire_protocol::{CheckCtx, Event, ExecCtx, Outcome, Protocol};
use fluxwire_vars::VarScope;
use futures::StreamExt;
use serde_json::{Value, json};

async fn gql(headers: HeaderMap, Json(v): Json<Value>) -> Json<Value> {
    assert_eq!(headers["content-type"], "application/json");
    let id = v["variables"]["id"].as_str().unwrap_or("none");
    Json(json!({"data": {"payment": {"id": id, "status": "PAID"}},
        "echo": {"operationName": v["operationName"], "has_query": v["query"].is_string()}}))
}

async fn gql_get(Query(q): Query<std::collections::HashMap<String, String>>) -> Json<Value> {
    Json(json!({"data": {"q": q.get("query")}}))
}

async fn errors() -> Json<Value> {
    Json(
        json!({"data": null, "errors": [{"message": "boom", "locations": [{"line": 1, "column": 3}]}]}),
    )
}

async fn serve(app: Router) -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    a
}

async fn server() -> SocketAddr {
    serve(
        Router::new()
            .route("/graphql", post(gql).get(gql_get))
            .route("/errors", post(errors)),
    )
    .await
}

async fn stalling() -> SocketAddr {
    serve(
        Router::new().route(
            "/graphql",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                "x"
            })
            .post(|| async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                "x"
            }),
        ),
    )
    .await
}

fn doc(method: &str, url: &str, extra: &str) -> FluxDoc {
    fluxwire_format::parse(&format!(
        "---\nname: t\nrequires: [graphql]\n---\n\n```http\n{method} {url}\n```\n\n```graphql query\nquery Pay($id: ID!) {{ payment(id: $id) {{ id status }} }}\n```\n\n```json variables\n{{\"id\": \"p1\"}}\n```\n{extra}"
    ))
}

async fn run(d: &FluxDoc) -> Vec<Event> {
    let p = GraphqlProtocol;
    let plan = p.plan(d, &VarScope::default()).expect("plan");
    p.execute(plan, ExecCtx::default()).collect().await
}

fn outcome(ev: &[Event]) -> &Outcome {
    match ev.last() {
        Some(Event::Done(o)) => o,
        other => panic!("{other:?}"),
    }
}

fn body(o: &Outcome) -> Value {
    let x = o.exchange.as_ref().unwrap();
    serde_json::from_slice(&x.body.read_prefix(1 << 20).unwrap()).unwrap()
}

#[tokio::test]
async fn post_sends_query_variables_and_operation_name() {
    let a = server().await;
    let ev = run(&doc("POST", &format!("http://{a}/graphql"), "")).await;
    let v = body(outcome(&ev));
    assert_eq!(v["data"]["payment"]["id"], "p1");
    assert_eq!(v["echo"]["operationName"], "Pay");
    assert_eq!(v["echo"]["has_query"], true);
}

#[tokio::test]
async fn get_puts_the_query_in_the_query_string() {
    let a = server().await;
    let ev = run(&doc("GET", &format!("http://{a}/graphql"), "")).await;
    assert!(
        body(outcome(&ev))["data"]["q"]
            .as_str()
            .unwrap()
            .starts_with("query Pay")
    );
}

fn check(d: &FluxDoc, o: &Outcome) -> Vec<fluxwire_protocol::TestResult> {
    let i = d
        .blocks
        .iter()
        .position(|b| b.kind == BlockKind::Expect)
        .expect("expect block");
    let fluxwire_model::BlockData::Expect(e) = &d.data[i] else {
        panic!("not an expect block")
    };
    let scope = VarScope::default();
    GraphqlProtocol.check(
        o,
        e,
        &CheckCtx {
            doc: Some(d),
            scope: &scope,
        },
    )
}

#[tokio::test]
async fn expect_matches_partially_on_data() {
    let a = server().await;
    let d = doc(
        "POST",
        &format!("http://{a}/graphql"),
        "\n```http expect\n200\n\n{\"payment\": {\"status\": \"PAID\"}}\n```\n",
    );
    let ev = run(&d).await;
    let r = check(&d, outcome(&ev));
    assert!(r.iter().all(|t| t.passed), "{r:?}");

    let bad = doc(
        "POST",
        &format!("http://{a}/graphql"),
        "\n```http expect\n200\n\n{\"payment\": {\"status\": \"NOPE\"}}\n```\n",
    );
    let ev = run(&bad).await;
    assert!(check(&bad, outcome(&ev)).iter().any(|t| !t.passed));
}

#[tokio::test]
async fn errors_reach_the_response_body() {
    let a = server().await;
    let ev = run(&doc("POST", &format!("http://{a}/errors"), "")).await;
    assert_eq!(body(outcome(&ev))["errors"][0]["message"], "boom");
}

#[tokio::test]
async fn history_records_graphql() {
    let a = server().await;
    let ev = run(&doc("POST", &format!("http://{a}/graphql"), "")).await;
    let rec = GraphqlProtocol.history_record(outcome(&ev));
    assert_eq!(rec.protocol, "graphql");
    assert!(GraphqlProtocol.history_to_outcome(&rec).is_some());
}

#[tokio::test]
async fn claims_only_documents_with_a_query_block() {
    let with = doc("POST", "http://x.test/graphql", "");
    assert!(GraphqlProtocol.claims(&with));
    let plain = fluxwire_format::parse("```http\nGET http://x.test\n```\n");
    assert!(!GraphqlProtocol.claims(&plain));
}

#[tokio::test]
async fn graphql_passes_the_protocol_conformance_suite() {
    let a = server().await;
    let s = stalling().await;
    let fast = doc(
        "POST",
        &format!("http://{a}/graphql"),
        "\n```http expect\n200\n```\n",
    );
    let slow = doc("POST", &format!("http://{s}/graphql"), "");
    fluxwire_protocol::conformance::run(
        &GraphqlProtocol,
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

#[tokio::test]
async fn introspection_plan_fetches_and_parses_a_schema() {
    use axum::routing::post;
    let app = Router::new().route(
        "/graphql",
        post(|Json(v): Json<Value>| async move {
            assert!(v["query"].as_str().unwrap().contains("__schema"));
            Json(json!({"data": {"__schema": {
                "queryType": {"name": "Query"},
                "types": [{"kind": "OBJECT", "name": "Query", "fields": [
                    {"name": "hello", "args": [], "type": {"kind": "SCALAR", "name": "String"}, "isDeprecated": false}]}]}}}))
        }),
    );
    let a = serve(app).await;
    let d = doc("POST", &format!("http://{a}/graphql"), "");
    let plan = fluxwire_graphql::introspection_plan(&d, &VarScope::default()).unwrap();
    let ev: Vec<Event> = GraphqlProtocol
        .execute(plan, ExecCtx::default())
        .collect()
        .await;
    let (schema, _raw) = fluxwire_graphql::schema_from_outcome(outcome(&ev)).unwrap();
    assert!(schema.ty("Query").unwrap().field("hello").is_some());
}
