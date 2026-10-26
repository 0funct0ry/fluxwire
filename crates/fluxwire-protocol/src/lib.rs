//! The `Protocol` trait with its `Plan`/`Event`/`Outcome` types, and the registry that tells
//! the format parser which request languages and roles are known (SPEC §5.5).

mod exec;
mod state;
pub use exec::*;
pub use state::*;

pub mod conformance;

/// One request body or message block shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// Protocol message head + body (HTTP request/response).
    Message,
    /// `name: value` lines.
    KvLines,
    /// Header lines only.
    HeaderLines,
    /// Free-form text (messages, config).
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    AtMostOne,
    Any,
}

/// A role a protocol adds on top of its request language.
#[derive(Debug, Clone, Copy)]
pub struct RoleSpec {
    pub role: &'static str,
    /// Language the role is written in; `None` = any language (e.g. `message`).
    pub language: Option<&'static str>,
    pub content: ContentKind,
    pub cardinality: Cardinality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    Exchange,
    Session,
}

/// Static description of a protocol, enough for parsing. Execution lives in the protocol crates.
#[derive(Debug, Clone)]
pub struct ProtocolInfo {
    pub id: &'static str,
    /// Feature name used in `requires`; `None` for base-format protocols.
    pub feature: Option<&'static str>,
    pub request_languages: &'static [&'static str],
    pub roles: &'static [RoleSpec],
    pub interaction: Interaction,
}

const HTTP_ROLES: &[RoleSpec] = &[
    RoleSpec {
        role: "expect",
        language: Some("http"),
        content: ContentKind::Message,
        cardinality: Cardinality::AtMostOne,
    },
    RoleSpec {
        role: "form",
        language: Some("http"),
        content: ContentKind::KvLines,
        cardinality: Cardinality::AtMostOne,
    },
    RoleSpec {
        role: "multipart",
        language: Some("http"),
        content: ContentKind::KvLines,
        cardinality: Cardinality::AtMostOne,
    },
    RoleSpec {
        role: "defaults",
        language: Some("http"),
        content: ContentKind::HeaderLines,
        cardinality: Cardinality::AtMostOne,
    },
];

const WS_ROLES: &[RoleSpec] = &[RoleSpec {
    role: "message",
    language: None,
    content: ContentKind::Text,
    cardinality: Cardinality::Any,
}];

/// Roles owned by the core format, independent of protocol.
pub const SCRIPT_ROLES: &[&str] = &["pre", "post", "test"];
pub const SCRIPT_LANGUAGES: &[&str] = &["ts", "js"];
pub const CORE_OTHER_ROLES: &[&str] = &["config", "example"];

/// Roles reserved for future use (SPEC §7.5).
pub const RESERVED_ROLES: &[&str] = &[
    "query",
    "variables",
    "arguments",
    "schema",
    "step",
    "response",
    "fixture",
    "mock",
    "stream",
    "metadata",
    "resource",
    "prompt",
];

/// Request languages reserved for later protocols (SPEC §7.9).
pub const RESERVED_REQUEST_LANGUAGES: &[&str] =
    &["grpc", "mcp", "sse", "mqtt", "amqp", "kafka", "nats"];

/// Languages whose second info-string token is interpreted as a role. Other fences are plain prose.
pub const ROLE_CAPABLE_LANGUAGES: &[&str] = &[
    "http", "ws", "ts", "js", "json", "yaml", "xml", "text", "binary", "graphql", "sparql",
    "proto", "grpc", "mcp", "sse", "mqtt", "amqp", "kafka", "nats",
];

#[derive(Debug, Clone)]
pub struct ProtocolRegistry {
    protocols: Vec<ProtocolInfo>,
    /// Features this version can run. Empty in v1.0 (SPEC §7.9).
    supported_features: Vec<&'static str>,
}

impl Default for ProtocolRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ProtocolRegistry {
    pub fn builtin() -> Self {
        Self {
            protocols: vec![
                ProtocolInfo {
                    id: "http",
                    feature: None,
                    request_languages: &["http"],
                    roles: HTTP_ROLES,
                    interaction: Interaction::Exchange,
                },
                ProtocolInfo {
                    id: "ws",
                    feature: None,
                    request_languages: &["ws"],
                    roles: WS_ROLES,
                    interaction: Interaction::Session,
                },
            ],
            supported_features: Vec::new(),
        }
    }

    pub fn protocols(&self) -> &[ProtocolInfo] {
        &self.protocols
    }

    pub fn protocol_for_language(&self, lang: &str) -> Option<&ProtocolInfo> {
        self.protocols
            .iter()
            .find(|p| p.request_languages.contains(&lang))
    }

    pub fn is_request_language(&self, lang: &str) -> bool {
        self.protocol_for_language(lang).is_some()
    }

    pub fn is_reserved_request_language(&self, lang: &str) -> bool {
        !self.is_request_language(lang) && RESERVED_REQUEST_LANGUAGES.contains(&lang)
    }

    pub fn is_reserved_role(&self, role: &str) -> bool {
        RESERVED_ROLES.contains(&role)
            && !self
                .protocols
                .iter()
                .any(|p| p.roles.iter().any(|r| r.role == role))
    }

    /// Protocol role for `(language, role)`, if registered.
    pub fn role_spec(&self, lang: &str, role: &str) -> Option<&RoleSpec> {
        self.protocols
            .iter()
            .flat_map(|p| p.roles.iter())
            .find(|r| r.role == role && r.language.is_none_or(|l| l == lang))
    }

    pub fn is_role_capable(&self, lang: &str) -> bool {
        ROLE_CAPABLE_LANGUAGES.contains(&lang)
    }

    pub fn supports_feature(&self, feature: &str) -> bool {
        self.supported_features.contains(&feature)
    }

    /// Features named by the registry for reserved languages, e.g. `grpc`.
    pub fn feature_for_language<'a>(&self, lang: &'a str) -> Option<&'a str> {
        if self.is_reserved_request_language(lang) {
            Some(lang)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_and_ws_registered() {
        let r = ProtocolRegistry::builtin();
        assert!(r.is_request_language("http"));
        assert!(r.is_request_language("ws"));
        assert!(r.is_reserved_request_language("grpc"));
        assert!(r.role_spec("http", "expect").is_some());
        assert!(r.role_spec("json", "message").is_some());
        assert!(r.is_reserved_role("schema"));
        assert!(!r.supports_feature("grpc"));
    }
}
