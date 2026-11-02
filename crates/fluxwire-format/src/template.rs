//! Default layouts for new files (SPEC §7.10).

pub fn new_request(name: &str, description: &str, method: &str, url: &str) -> String {
    let mut s = format!(
        "---\nname: {}\n---\n\n# {name}\n\n",
        crate::encode_scalar(name)
    );
    if !description.trim().is_empty() {
        s.push_str(description.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("```http\n{method} {url}\n```\n"));
    s
}

/// Initial body of a new HTTP request (New request dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitialBody {
    #[default]
    None,
    Json,
    Text,
    Form,
}

impl InitialBody {
    /// POST, PUT and PATCH default to JSON; everything else to no body.
    pub fn default_for(method: &str) -> Self {
        match method {
            "POST" | "PUT" | "PATCH" => Self::Json,
            _ => Self::None,
        }
    }
}

/// Like [`new_request`], with an initial body: a `Content-Type` header and, for JSON, a `{}`
/// body inside the request block; for forms, an `http form` block.
pub fn new_request_with_body(
    name: &str,
    description: &str,
    method: &str,
    url: &str,
    body: InitialBody,
) -> String {
    let mut s = format!(
        "---\nname: {}\n---\n\n# {name}\n\n",
        crate::encode_scalar(name)
    );
    if !description.trim().is_empty() {
        s.push_str(description.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("```http\n{method} {url}\n"));
    match body {
        InitialBody::None => {}
        InitialBody::Json => s.push_str("Content-Type: application/json\n\n{}\n"),
        InitialBody::Text => s.push_str("Content-Type: text/plain\n"),
        InitialBody::Form => s.push_str("Content-Type: application/x-www-form-urlencoded\n"),
    }
    s.push_str("```\n");
    if body == InitialBody::Form {
        s.push_str("\n```http form\nkey: value\n```\n");
    }
    s
}

pub fn new_websocket(name: &str, description: &str, url: &str) -> String {
    let mut s = format!(
        "---\nname: {}\n---\n\n# {name}\n\n",
        crate::encode_scalar(name)
    );
    if !description.trim().is_empty() {
        s.push_str(description.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("```ws\nGET {url}\n```\n"));
    s
}

pub fn new_folder(name: &str) -> String {
    format!(
        "---\nname: {}\n---\n\n# {name}\n",
        crate::encode_scalar(name)
    )
}

pub fn new_collection_file(name: &str) -> String {
    format!(
        "---\nname: {}\n---\n\n# {name}\n",
        crate::encode_scalar(name)
    )
}

pub fn new_collection_toml(name: &str) -> String {
    format!("[collection]\nname = {}\nformat = 1\n", toml_string(name))
}

pub(crate) fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub const GITIGNORE: &str = "*.local.env\nfluxwire.local.toml\n.fluxwire/cache/\n";
