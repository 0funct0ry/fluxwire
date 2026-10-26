use crate::{Item, Kind};

/// Deterministic generated requests for benches and tests.
pub fn generate_requests(n: usize) -> Vec<Item> {
    const VERBS: [&str; 6] = ["Create", "Get", "List", "Update", "Delete", "Refund"];
    const NOUNS: [&str; 8] = [
        "payment",
        "customer",
        "invoice",
        "refund",
        "webhook",
        "token",
        "order",
        "subscription",
    ];
    const METHODS: [&str; 6] = ["POST", "GET", "GET", "PUT", "DELETE", "POST"];
    (0..n)
        .map(|i| {
            let v = i % VERBS.len();
            let noun = NOUNS[(i / VERBS.len()) % NOUNS.len()];
            let folder = format!("svc{:03}/{}", i / 48, noun);
            let path = format!("{folder}/{}-{i}.flux.md", VERBS[v].to_lowercase());
            Item::new(
                format!("req:{path}"),
                Kind::Request,
                format!("{} {noun} {i}", VERBS[v]),
            )
            .detail(path)
            .extra(format!("{} {{{{baseUrl}}}}/v1/{noun}s/{i}", METHODS[v]))
        })
        .collect()
}
