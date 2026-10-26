//! Mock data for views that later milestones replace: sync (M12).
//! M9 removed the folder-run mock. The tree, request and workspace mocks were removed in M2.

#[derive(Clone, Debug)]
pub struct Kv {
    pub key: &'static str,
    pub value: &'static str,
    pub enabled: bool,
}

const fn kv(key: &'static str, value: &'static str) -> Kv {
    Kv {
        key,
        value,
        enabled: true,
    }
}

#[derive(Clone, Debug)]
pub struct MockResponse {
    pub status: u16,
    pub status_text: &'static str,
    pub time_ms: u32,
    pub size: &'static str,
    pub protocol: &'static str,
    pub body: &'static str,
    pub headers: Vec<Kv>,
    /// (phase, milliseconds)
    pub timeline: Vec<(&'static str, u32)>,
    /// (name, passed)
    pub tests: Vec<(&'static str, bool)>,
    pub console: Vec<&'static str>,
}

pub struct SyncChange {
    pub arrow: &'static str,
    pub operation: &'static str,
    pub field: &'static str,
}

pub struct Mock {
    pub response: MockResponse,
    pub sync: Vec<SyncChange>,
}

const PAYMENT_JSON: &str = r#"{
  "id": "pay_8f2k1x9",
  "object": "payment",
  "amount": 4200,
  "currency": "usd",
  "status": "requires_capture",
  "customer": "cus_Q81pLm",
  "metadata": {
    "order_id": "ord_1042",
    "channel": "web"
  },
  "created": 1767225600,
  "refunded": false
}"#;

fn json_headers() -> Vec<Kv> {
    vec![
        kv("Content-Type", "application/json; charset=utf-8"),
        kv("X-Request-Id", "req_5d1c0a77"),
        kv("Cache-Control", "no-store"),
        kv("Strict-Transport-Security", "max-age=63072000"),
    ]
}

fn timeline() -> Vec<(&'static str, u32)> {
    vec![
        ("DNS", 6),
        ("Connect", 18),
        ("TLS", 31),
        ("TTFB", 74),
        ("Download", 13),
    ]
}

fn response(status: u16, text: &'static str, ms: u32, body: &'static str) -> MockResponse {
    MockResponse {
        status,
        status_text: text,
        time_ms: ms,
        size: "1.2 KB",
        protocol: "h2",
        body,
        headers: json_headers(),
        timeline: timeline(),
        tests: vec![
            ("status is 2xx", status / 100 == 2),
            ("body has an id", true),
            ("currency is usd", true),
        ],
        console: vec![
            "[12:04:31.220] log  signed request",
            "[12:04:31.402] log  captured paymentId",
        ],
    }
}

pub fn payments_api() -> Mock {
    Mock {
        response: response(201, "Created", 142, PAYMENT_JSON),
        sync: vec![
            SyncChange {
                arrow: "↓",
                operation: "POST /v1/payments",
                field: "description changed in spec",
            },
            SyncChange {
                arrow: "↓",
                operation: "GET /v1/payments/{id}",
                field: "new query parameter: expand",
            },
            SyncChange {
                arrow: "↓",
                operation: "POST /v1/refunds",
                field: "required field: reason",
            },
            SyncChange {
                arrow: "↑",
                operation: "PATCH /v1/payments/{id}",
                field: "request renamed: Update payment",
            },
        ],
    }
}

impl Mock {}
