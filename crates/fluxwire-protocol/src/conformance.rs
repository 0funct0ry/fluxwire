//! Conformance suite every `Protocol` implementation must pass (SPEC §23).
//!
//! The caller supplies two documents that target a test server: one that
//! answers quickly (`fast`) and one that stalls (`slow`) so cancellation can be
//! observed. Call [`run`] from the protocol crate's integration tests.

use std::time::Duration;

use fluxwire_model::{Body as DocBody, ExpectBlock, FluxDoc, StatusMatch};
use fluxwire_vars::VarScope;
use futures::StreamExt;

use crate::{CancellationToken, Event, ExecCtx, Outcome, Protocol};

pub struct Fixture<'a> {
    pub fast: &'a FluxDoc,
    pub slow: &'a FluxDoc,
    pub scope: VarScope,
    pub ctx: ExecCtx,
    /// Status the `fast` document is expected to produce.
    pub fast_status: u16,
}

async fn collect(p: &dyn Protocol, doc: &FluxDoc, f: &Fixture<'_>, ctx: ExecCtx) -> Vec<Event> {
    let plan = p.plan(doc, &f.scope).expect("plan");
    p.execute(plan, ctx).collect().await
}

fn done(events: &[Event]) -> &Outcome {
    match events.last() {
        Some(Event::Done(o)) => o,
        other => panic!("expected Done as last event, got {other:?}"),
    }
}

/// Panics with a descriptive message on the first violation.
pub async fn run(p: &dyn Protocol, f: Fixture<'_>) {
    assert!(
        !p.request_languages().is_empty(),
        "declares request languages"
    );

    // planning is pure and deterministic
    let a = p.plan(f.fast, &f.scope).expect("plan");
    let b = p.plan(f.fast, &f.scope).expect("plan");
    assert_eq!(a.label, b.label);
    assert_eq!(a.protocol, p.id());

    // event ordering: only progress events, then exactly one terminal event
    let events = collect(p, f.fast, &f, f.ctx.clone()).await;
    let (last, rest) = events.split_last().expect("events");
    assert!(
        matches!(last, Event::Done(_)),
        "stream ends with Done: {last:?}"
    );
    assert!(
        rest.iter()
            .all(|e| matches!(e, Event::Phase(_) | Event::Redirect(_) | Event::Frame(_))),
        "no terminal event before the end"
    );
    let outcome = done(&events);
    assert_eq!(outcome.protocol, p.id());
    assert_eq!(outcome.interaction, p.interaction());
    assert!(outcome.total_ms >= 0.0);
    if let Some(x) = &outcome.exchange {
        assert_eq!(x.status, f.fast_status);
    }

    // history record shape
    let rec = p.history_record(outcome);
    assert_eq!(rec.protocol, p.id());
    assert!(!rec.summary.is_empty());

    // expect checks produce results, and a wrong status fails
    let expect = |status| ExpectBlock {
        status: StatusMatch::Exact(status),
        version: None,
        headers: Vec::new(),
        body: DocBody::None,
    };
    let ok = expect(f.fast_status);
    let res = p.check(outcome, &ok);
    assert!(!res.is_empty() && res.iter().all(|t| t.passed), "{res:?}");
    let wrong = expect(if f.fast_status == 599 { 598 } else { 599 });
    assert!(p.check(outcome, &wrong).iter().any(|t| !t.passed));

    // cancellation ends the stream with Failed, promptly
    let token = CancellationToken::new();
    let mut ctx = f.ctx.clone();
    ctx.cancel = token.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        token.cancel();
    });
    let events = tokio::time::timeout(Duration::from_secs(5), collect(p, f.slow, &f, ctx))
        .await
        .expect("cancelled run finishes promptly");
    canceller.await.ok();
    match events.last() {
        Some(Event::Failed(e)) => assert_eq!(e.phase, "cancelled"),
        other => panic!("cancelled run must end in Failed, got {other:?}"),
    }
}
