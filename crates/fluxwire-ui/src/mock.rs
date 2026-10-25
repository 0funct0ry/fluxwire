//! Mock "payments-api" workspace. Everything in here is deleted by later milestones: the tree
//! and request views in M2, the rest as response, history and sync land (M3–M12).

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
    pub cookies: Vec<Kv>,
    /// (phase, milliseconds)
    pub timeline: Vec<(&'static str, u32)>,
    /// (name, passed)
    pub tests: Vec<(&'static str, bool)>,
    pub console: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct MockRequest {
    pub id: &'static str,
    pub name: &'static str,
    /// HTTP method, or "WS".
    pub method: &'static str,
    pub url: &'static str,
    pub params: Vec<Kv>,
    pub headers: Vec<Kv>,
    pub body: &'static str,
    pub auth: &'static str,
    pub scripts: usize,
    pub tests: usize,
    pub docs: &'static str,
    pub response: MockResponse,
}

pub enum TreeNode {
    Folder {
        path: &'static str,
        name: &'static str,
        children: Vec<TreeNode>,
    },
    Request(&'static str),
}

pub struct HistoryEntry {
    pub day: &'static str,
    pub method: &'static str,
    pub name: &'static str,
    pub status: u16,
    pub time: &'static str,
    pub ms: u32,
}

pub struct Environment {
    pub name: &'static str,
    pub vars: Vec<(&'static str, &'static str, bool)>,
}

pub struct SyncChange {
    pub arrow: &'static str,
    pub operation: &'static str,
    pub field: &'static str,
}

pub struct Problem {
    pub file: &'static str,
    pub line: u32,
    pub message: &'static str,
    pub code: &'static str,
}

pub struct RunRow {
    pub name: &'static str,
    pub method: &'static str,
    pub passed: bool,
    pub ms: u32,
}

pub struct Mock {
    pub workspace: &'static str,
    pub branch: &'static str,
    pub tree: Vec<TreeNode>,
    pub requests: Vec<MockRequest>,
    pub history: Vec<HistoryEntry>,
    pub environments: Vec<Environment>,
    pub sync: Vec<SyncChange>,
    pub problems: Vec<Problem>,
    pub run: Vec<RunRow>,
    pub ws_log: Vec<(&'static str, &'static str, &'static str)>,
    pub recent_workspaces: Vec<&'static str>,
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
        cookies: vec![kv("session", "s%3A9c1e…; HttpOnly; Secure")],
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

#[allow(clippy::too_many_arguments)]
fn req(
    id: &'static str,
    name: &'static str,
    method: &'static str,
    url: &'static str,
    body: &'static str,
    status: u16,
    text: &'static str,
) -> MockRequest {
    MockRequest {
        id,
        name,
        method,
        url,
        params: vec![
            kv("expand", "customer"),
            Kv {
                key: "limit",
                value: "20",
                enabled: false,
            },
        ],
        headers: vec![
            kv("Authorization", "Bearer {{token}}"),
            kv("Idempotency-Key", "{{$uuid}}"),
        ],
        body,
        auth: "Bearer token ({{token}})",
        scripts: 1,
        tests: 3,
        docs: "Creates a payment for a customer. Amounts are in the smallest currency unit.",
        response: response(
            status,
            text,
            142,
            if status == 204 { "" } else { PAYMENT_JSON },
        ),
    }
}

pub fn payments_api() -> Mock {
    let create_body =
        "{\n  \"amount\": 4200,\n  \"currency\": \"usd\",\n  \"customer\": \"{{customerId}}\"\n}";
    let requests = vec![
        req(
            "auth/login",
            "Log in",
            "POST",
            "{{baseUrl}}/v1/auth/login",
            "{\n  \"email\": \"{{email}}\",\n  \"password\": \"{{password}}\"\n}",
            200,
            "OK",
        ),
        req(
            "auth/refresh",
            "Refresh token",
            "POST",
            "{{baseUrl}}/v1/auth/refresh",
            "{\n  \"refreshToken\": \"{{refreshToken}}\"\n}",
            200,
            "OK",
        ),
        req(
            "payments/create",
            "Create payment",
            "POST",
            "{{baseUrl}}/v1/payments",
            create_body,
            201,
            "Created",
        ),
        req(
            "payments/get",
            "Get payment",
            "GET",
            "{{baseUrl}}/v1/payments/:id",
            "",
            200,
            "OK",
        ),
        req(
            "payments/list",
            "List payments",
            "GET",
            "{{baseUrl}}/v1/payments",
            "",
            200,
            "OK",
        ),
        req(
            "payments/update",
            "Update payment",
            "PATCH",
            "{{baseUrl}}/v1/payments/:id",
            "{\n  \"metadata\": { \"channel\": \"app\" }\n}",
            200,
            "OK",
        ),
        req(
            "payments/cancel",
            "Cancel payment",
            "DELETE",
            "{{baseUrl}}/v1/payments/:id",
            "",
            404,
            "Not Found",
        ),
        req(
            "refunds/create",
            "Create refund",
            "POST",
            "{{baseUrl}}/v1/refunds",
            "{\n  \"payment\": \"{{paymentId}}\",\n  \"amount\": 1000\n}",
            201,
            "Created",
        ),
        req(
            "refunds/get",
            "Get refund",
            "GET",
            "{{baseUrl}}/v1/refunds/:id",
            "",
            200,
            "OK",
        ),
        req(
            "events/order-stream",
            "Order stream",
            "WS",
            "wss://{{host}}/v1/events/orders",
            "{\n  \"type\": \"subscribe\",\n  \"topic\": \"orders\"\n}",
            101,
            "Switching Protocols",
        ),
    ];
    let tree = vec![
        TreeNode::Folder {
            path: "auth",
            name: "auth",
            children: vec![
                TreeNode::Request("auth/login"),
                TreeNode::Request("auth/refresh"),
            ],
        },
        TreeNode::Folder {
            path: "payments",
            name: "payments",
            children: vec![
                TreeNode::Request("payments/create"),
                TreeNode::Request("payments/get"),
                TreeNode::Request("payments/list"),
                TreeNode::Request("payments/update"),
                TreeNode::Request("payments/cancel"),
            ],
        },
        TreeNode::Folder {
            path: "refunds",
            name: "refunds",
            children: vec![
                TreeNode::Request("refunds/create"),
                TreeNode::Request("refunds/get"),
            ],
        },
        TreeNode::Folder {
            path: "events",
            name: "events",
            children: vec![TreeNode::Request("events/order-stream")],
        },
    ];
    Mock {
        workspace: "payments-api",
        branch: "main",
        tree,
        requests,
        history: vec![
            HistoryEntry {
                day: "Today",
                method: "POST",
                name: "Create payment",
                status: 201,
                time: "12:04",
                ms: 142,
            },
            HistoryEntry {
                day: "Today",
                method: "GET",
                name: "Get payment",
                status: 200,
                time: "12:05",
                ms: 88,
            },
            HistoryEntry {
                day: "Today",
                method: "DELETE",
                name: "Cancel payment",
                status: 404,
                time: "11:48",
                ms: 64,
            },
            HistoryEntry {
                day: "Yesterday",
                method: "POST",
                name: "Create refund",
                status: 500,
                time: "17:30",
                ms: 910,
            },
            HistoryEntry {
                day: "Yesterday",
                method: "GET",
                name: "List payments",
                status: 200,
                time: "16:12",
                ms: 230,
            },
        ],
        environments: vec![
            Environment {
                name: "local",
                vars: vec![
                    ("baseUrl", "http://localhost:8080", false),
                    ("host", "localhost:8080", false),
                ],
            },
            Environment {
                name: "staging",
                vars: vec![
                    ("baseUrl", "https://staging.payments.example.com", false),
                    ("host", "staging.payments.example.com", false),
                    ("token", "••••••••", true),
                ],
            },
            Environment {
                name: "production",
                vars: vec![
                    ("baseUrl", "https://api.payments.example.com", false),
                    ("token", "••••••••", true),
                ],
            },
        ],
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
        problems: vec![
            Problem {
                file: "payments/cancel.flux.md",
                line: 12,
                message: "Unresolved variable {{paymentId}}",
                code: "V101",
            },
            Problem {
                file: "refunds/get.flux.md",
                line: 4,
                message: "Unknown role 'sample'; use 'example'",
                code: "F201",
            },
        ],
        run: vec![
            RunRow {
                name: "Log in",
                method: "POST",
                passed: true,
                ms: 120,
            },
            RunRow {
                name: "Create payment",
                method: "POST",
                passed: true,
                ms: 142,
            },
            RunRow {
                name: "Get payment",
                method: "GET",
                passed: true,
                ms: 88,
            },
            RunRow {
                name: "Cancel payment",
                method: "DELETE",
                passed: false,
                ms: 64,
            },
        ],
        ws_log: vec![
            (
                "out",
                "12:10:01.004",
                "{\"type\":\"subscribe\",\"topic\":\"orders\"}",
            ),
            (
                "in",
                "12:10:01.121",
                "{\"type\":\"subscribed\",\"topic\":\"orders\"}",
            ),
            (
                "in",
                "12:10:03.480",
                "{\"type\":\"order.created\",\"id\":\"ord_1043\"}",
            ),
            (
                "in",
                "12:10:07.915",
                "{\"type\":\"order.paid\",\"id\":\"ord_1043\"}",
            ),
        ],
        recent_workspaces: vec![
            "~/code/payments-api",
            "~/code/billing-service",
            "~/code/fixtures",
        ],
    }
}

impl Mock {
    pub fn request(&self, id: &str) -> Option<&MockRequest> {
        self.requests.iter().find(|r| r.id == id)
    }

    pub fn environment(&self, name: &str) -> Option<&Environment> {
        self.environments.iter().find(|e| e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(nodes: &[TreeNode], out: &mut Vec<&'static str>) {
        for n in nodes {
            match n {
                TreeNode::Folder { children, .. } => walk(children, out),
                TreeNode::Request(id) => out.push(id),
            }
        }
    }

    #[test]
    fn every_tree_request_exists_and_has_one_websocket() {
        let m = payments_api();
        let mut ids = vec![];
        walk(&m.tree, &mut ids);
        assert_eq!(ids.len(), m.requests.len());
        for id in ids {
            assert!(m.request(id).is_some(), "{id}");
        }
        assert_eq!(m.requests.iter().filter(|r| r.method == "WS").count(), 1);
    }
}
