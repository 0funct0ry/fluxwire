use criterion::{Criterion, criterion_group, criterion_main};
use fluxwire_search::{Index, generate_requests};

fn bench(c: &mut Criterion) {
    let mut ix = Index::new();
    for it in generate_requests(5000) {
        ix.upsert(it);
    }
    // SPEC §20: keystroke to results under 16 ms at 5,000 requests.
    for q in ["c", "cre", "create pay", "svc012/inv", "zzzz"] {
        c.bench_function(&format!("query_5000/{q}"), |b| b.iter(|| ix.query(q, 50)));
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
