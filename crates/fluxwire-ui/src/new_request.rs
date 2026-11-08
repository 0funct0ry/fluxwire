//! Pure model of the New request dialog (M9.07, ADR-017). No GPUI: the dialog view reads
//! and writes this form; file writes go through `fluxwire_format::fsops`.

use fluxwire_format::{InitialBody, RequestSpec, render_request, request_path, slug};
use fluxwire_protocol::PasteSummary;
use fluxwire_runner::ProtocolDesc;
use std::path::{Path, PathBuf};

pub const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const HTTP_SCHEMES: [&str; 2] = ["http://", "https://"];
const WS_SCHEMES: [&str; 2] = ["ws://", "wss://"];
const NAME_MAX: usize = 120;

/// What kind of request the dialog creates. `Other` is a registered Protocol without
/// dialog-specific fields; it gets a name, a location and a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Http,
    WebSocket,
    Curl,
    Other(&'static str),
}

impl NewKind {
    pub fn label(self, descs: &[ProtocolDesc]) -> String {
        match self {
            NewKind::Http => "HTTP".into(),
            NewKind::WebSocket => "WebSocket".into(),
            NewKind::Curl => "From cURL".into(),
            NewKind::Other(id) => descs
                .iter()
                .find(|d| d.id == id)
                .map_or(id.to_string(), |d| d.label.to_string()),
        }
    }
}

/// The types the build can create: registered Protocols, plus From cURL when one can import it.
pub fn available_kinds(descs: &[ProtocolDesc], can_import_curl: bool) -> Vec<NewKind> {
    let mut out = Vec::new();
    for d in descs {
        match d.id {
            "http" => out.push(NewKind::Http),
            "ws" => out.push(NewKind::WebSocket),
            id => out.push(NewKind::Other(id)),
        }
    }
    if can_import_curl && descs.iter().any(|d| d.id == "http") {
        out.push(NewKind::Curl);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Url,
    Method,
    Curl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct NewRequestForm {
    pub kind: NewKind,
    pub name: String,
    pub location: PathBuf,
    pub url: String,
    pub method: String,
    pub curl: String,
    pub description: String,
    /// `None` follows the method (POST/PUT/PATCH → JSON).
    pub body: Option<InitialBody>,
    pub options_open: bool,
    /// Set by the view from `Runner::preview_paste` whenever `curl` changes.
    pub curl_preview: Option<PasteSummary>,
}

/// Everything the UI needs to write the file.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatePlan {
    pub kind: NewKind,
    pub dir: PathBuf,
    pub name: String,
    pub description: String,
    pub method: String,
    pub url: String,
    pub body: InitialBody,
    /// The command to import after the template is written (From cURL).
    pub curl: Option<String>,
}

impl NewRequestForm {
    pub fn new(location: PathBuf, kind: NewKind) -> Self {
        Self {
            kind,
            name: String::new(),
            location,
            url: String::new(),
            method: "GET".into(),
            curl: String::new(),
            description: String::new(),
            body: None,
            options_open: false,
            curl_preview: None,
        }
    }

    /// Switching keeps name, location, description and a typed URL; the URL is re-validated.
    pub fn set_kind(&mut self, kind: NewKind) {
        self.kind = kind;
    }

    pub fn body_kind(&self) -> InitialBody {
        self.body
            .unwrap_or_else(|| InitialBody::default_for(&self.method))
    }

    fn schemes(&self) -> Option<&'static [&'static str]> {
        match self.kind {
            NewKind::Http => Some(&HTTP_SCHEMES),
            NewKind::WebSocket => Some(&WS_SCHEMES),
            _ => None,
        }
    }

    /// The name used when the field is empty, derived from the method and URL.
    pub fn suggestion(&self) -> Option<String> {
        match self.kind {
            NewKind::Curl => self
                .curl_preview
                .as_ref()
                .and_then(|p| suggest_name(NewKind::Http, &p.method, &p.url)),
            k => suggest_name(k, &self.method, &self.url),
        }
    }

    /// The name that will be written: the typed one, else the suggestion.
    pub fn effective_name(&self) -> Option<String> {
        let n = self.name.trim();
        if n.is_empty() {
            self.suggestion()
        } else {
            Some(n.to_string())
        }
    }

    pub fn validate(&self) -> Vec<FieldError> {
        let mut out = Vec::new();
        let err = |field, m: &str| FieldError {
            field,
            message: m.to_string(),
        };
        let name = self.name.trim();
        if name.is_empty() {
            if self.suggestion().is_none() {
                out.push(err(Field::Name, "Enter a name"));
            }
        } else if name.chars().count() > NAME_MAX {
            out.push(err(Field::Name, "Name is longer than 120 characters"));
        } else if name.contains(['/', '\\']) {
            out.push(err(Field::Name, "Name cannot contain / or \\"));
        } else if name.starts_with('.') {
            out.push(err(Field::Name, "Name cannot start with a dot"));
        } else if !name.chars().any(char::is_alphanumeric) {
            out.push(err(Field::Name, "Name needs a letter or digit"));
        }
        match self.kind {
            NewKind::Curl => {
                if self.curl.trim().is_empty() {
                    out.push(err(Field::Curl, "Paste a cURL command"));
                } else if self.curl_preview.is_none() {
                    out.push(err(Field::Curl, "Not a cURL command that can be imported"));
                }
            }
            _ => {
                let url = self.url.trim();
                if let Some(schemes) = self.schemes()
                    && !url.is_empty()
                    && !url.starts_with("{{")
                    && !schemes.iter().any(|s| url.starts_with(s))
                {
                    out.push(err(
                        Field::Url,
                        &format!("URL must start with {}", schemes.join(" or ")),
                    ));
                }
                if self.kind == NewKind::Http && !HTTP_METHODS.contains(&self.method.as_str()) {
                    out.push(err(Field::Method, "Unknown method"));
                }
            }
        }
        out
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Why Create is disabled, for its tooltip.
    pub fn first_error(&self) -> Option<String> {
        self.validate().into_iter().next().map(|e| e.message)
    }

    /// `POST https://… · 2 headers · JSON body 143 B` for the cURL summary line.
    pub fn curl_summary(&self) -> Option<String> {
        let p = self.curl_preview.as_ref()?;
        let n = p.header_count;
        let mut s = format!(
            "{} {} · {n} header{}",
            p.method,
            p.url,
            if n == 1 { "" } else { "s" }
        );
        if let Some((kind, bytes)) = &p.body {
            s.push_str(&format!(" · {kind} body {bytes} B"));
        }
        Some(s)
    }

    /// Warnings shown under the cURL field (ignored flags, credentials).
    pub fn curl_warnings(&self) -> Vec<String> {
        self.curl_preview
            .as_ref()
            .map(|p| p.warnings.clone())
            .unwrap_or_default()
    }

    /// The file the dialog will create and whether the name collided with an existing file,
    /// e.g. `("get-users-2.flux.md", true)`.
    pub fn file_preview(&self) -> Option<(String, bool)> {
        let name = self.effective_name()?;
        let p = request_path(&self.location, &name);
        let file = p.file_name()?.to_string_lossy().into_owned();
        let plain = format!("{}.flux.md", slug(&name));
        let collided = file != plain;
        Some((file, collided))
    }

    /// `default_url` fills an empty URL (`default_url_for` in envs.rs).
    pub fn plan(&self, default_url: &str) -> Option<CreatePlan> {
        if !self.is_valid() {
            return None;
        }
        let name = self.effective_name()?;
        let url = if self.url.trim().is_empty() {
            default_url.to_string()
        } else {
            self.url.trim().to_string()
        };
        let http = self.kind == NewKind::Http;
        Some(CreatePlan {
            kind: self.kind,
            dir: self.location.clone(),
            name,
            description: self.description.clone(),
            method: if http {
                self.method.clone()
            } else if self.kind == NewKind::Other("graphql") {
                "POST".into()
            } else {
                "GET".into()
            },
            url,
            body: if http {
                self.body_kind()
            } else {
                InitialBody::None
            },
            curl: (self.kind == NewKind::Curl).then(|| self.curl.trim().to_string()),
        })
    }
}

impl CreatePlan {
    /// The file text before any cURL edits are applied.
    pub fn render(&self) -> String {
        let spec = RequestSpec {
            name: &self.name,
            description: &self.description,
            method: &self.method,
            url: &self.url,
            websocket: self.kind == NewKind::WebSocket,
            body: self.body,
        };
        let text = render_request(&spec);
        match self.kind {
            NewKind::Other("graphql") => {
                let text = format!("{text}\n```graphql query\n{{\n  __typename\n}}\n```\n");
                fluxwire_format::requires::sync_requires(&text, fluxwire_model::FileKind::Request)
            }
            NewKind::Other(id) => text.replacen("```http", &format!("```{id}"), 1),
            _ => text,
        }
    }

    pub fn path(&self) -> PathBuf {
        request_path(&self.dir, &self.name)
    }
}

/// `METHOD last-segment` ("GET users"); WebSocket falls back to the host. `{{…}}` hosts,
/// query strings and fragments are dropped. `None` when the URL gives nothing to go on.
pub fn suggest_name(kind: NewKind, method: &str, url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or("");
    let last = parts
        .filter(|s| !s.is_empty() && !s.contains("{{"))
        .next_back();
    let host_ok = !host.is_empty() && !host.contains("{{");
    match kind {
        NewKind::WebSocket | NewKind::Other(_) => last
            .map(str::to_string)
            .or_else(|| host_ok.then(|| host.to_string())),
        _ => {
            let m = if method.is_empty() { "GET" } else { method };
            last.map(|s| format!("{m} {s}"))
                .or_else(|| host_ok.then(|| format!("{m} {host}")))
        }
    }
}

/// Relative path of `location` under `root`, for the Save in field.
pub fn relative_to(root: &Path, location: &Path) -> String {
    location
        .strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| location.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descs(ids: &[(&'static str, &'static str)]) -> Vec<ProtocolDesc> {
        ids.iter()
            .map(|(id, label)| ProtocolDesc {
                id,
                label,
                url_schemes: vec![],
            })
            .collect()
    }

    fn form(kind: NewKind) -> NewRequestForm {
        NewRequestForm::new(std::env::temp_dir().join("fw-nr-none"), kind)
    }

    fn preview(method: &str, url: &str) -> PasteSummary {
        PasteSummary {
            method: method.into(),
            url: url.into(),
            header_count: 1,
            body: Some(("JSON".into(), 7)),
            warnings: vec![],
        }
    }

    #[test]
    fn kinds_follow_registered_protocols() {
        let d = descs(&[("http", "HTTP"), ("ws", "WebSocket")]);
        assert_eq!(
            available_kinds(&d, true),
            [NewKind::Http, NewKind::WebSocket, NewKind::Curl]
        );
        assert_eq!(available_kinds(&d, false).len(), 2);
        let only_ws = descs(&[("ws", "WebSocket")]);
        assert_eq!(available_kinds(&only_ws, true), [NewKind::WebSocket]);
        let more = descs(&[("http", "HTTP"), ("graphql", "GraphQL")]);
        assert!(available_kinds(&more, false).contains(&NewKind::Other("graphql")));
        assert_eq!(NewKind::Other("graphql").label(&more), "GraphQL");
    }

    #[test]
    fn suggests_names() {
        let k = NewKind::Http;
        assert_eq!(
            suggest_name(k, "GET", "{{baseUrl}}/users?x=1").as_deref(),
            Some("GET users")
        );
        assert_eq!(
            suggest_name(k, "POST", "https://x.test/v1/p").as_deref(),
            Some("POST p")
        );
        assert_eq!(
            suggest_name(k, "GET", "https://api.test/").as_deref(),
            Some("GET api.test")
        );
        assert_eq!(suggest_name(k, "GET", "{{baseUrl}}/"), None);
        assert_eq!(suggest_name(k, "GET", ""), None);
        assert_eq!(
            suggest_name(NewKind::WebSocket, "GET", "wss://feed.test/").as_deref(),
            Some("feed.test")
        );
    }

    #[test]
    fn name_rules() {
        let mut f = form(NewKind::Http);
        assert_eq!(f.first_error().as_deref(), Some("Enter a name"));
        for bad in ["a/b", "a\\b", ".hidden", "***"] {
            f.name = bad.into();
            assert!(!f.is_valid(), "{bad}");
        }
        f.name = "x".repeat(121);
        assert!(!f.is_valid());
        f.name = "List users".into();
        assert!(f.is_valid());
        // an empty name is fine while the URL gives a suggestion
        f.name.clear();
        f.url = "{{baseUrl}}/users".into();
        assert!(f.is_valid());
        assert_eq!(f.effective_name().as_deref(), Some("GET users"));
    }

    #[test]
    fn url_schemes_per_kind() {
        let mut f = form(NewKind::Http);
        f.name = "n".into();
        f.url = "ws://x".into();
        assert_eq!(f.validate()[0].field, Field::Url);
        f.url = "https://x".into();
        assert!(f.is_valid());
        f.url = "{{baseUrl}}/x".into();
        assert!(f.is_valid());
        f.set_kind(NewKind::WebSocket);
        f.url = "https://x".into();
        assert!(!f.is_valid());
        f.url = "wss://x".into();
        assert!(f.is_valid());
        f.method = "NOPE".into();
        assert!(f.is_valid(), "method only matters for HTTP");
        f.set_kind(NewKind::Http);
        f.url = "wss://x".into();
        assert_eq!(f.validate()[0].field, Field::Url);
    }

    #[test]
    fn curl_needs_a_parsing_command() {
        let mut f = form(NewKind::Curl);
        f.name = "n".into();
        assert_eq!(f.first_error().as_deref(), Some("Paste a cURL command"));
        f.curl = "curl --weird".into();
        assert!(!f.is_valid());
        f.curl = "curl -X POST https://x.test/v1/p".into();
        f.curl_preview = Some(preview("POST", "https://x.test/v1/p"));
        assert!(f.is_valid());
        assert_eq!(
            f.curl_summary().as_deref(),
            Some("POST https://x.test/v1/p · 1 header · JSON body 7 B")
        );
        f.name.clear();
        assert_eq!(f.suggestion().as_deref(), Some("POST p"));
        assert!(f.plan("").unwrap().curl.is_some());
    }

    #[test]
    fn plan_uses_defaults_and_method_aware_body() {
        let mut f = form(NewKind::Http);
        f.name = "List users".into();
        f.method = "POST".into();
        f.url = "{{baseUrl}}/users".into();
        let p = f.plan("{{baseUrl}}/").unwrap();
        assert_eq!(p.body, InitialBody::Json);
        assert!(
            p.render()
                .contains("POST {{baseUrl}}/users\nContent-Type: application/json")
        );
        f.url.clear();
        f.method = "GET".into();
        let p = f.plan("{{baseUrl}}/").unwrap();
        assert_eq!(p.url, "{{baseUrl}}/");
        assert_eq!(p.body, InitialBody::None);
        f.body = Some(InitialBody::Text);
        assert_eq!(f.plan("").unwrap().body, InitialBody::Text);
        f.name.clear();
        f.url.clear();
        assert!(f.plan("x").is_none(), "no name and nothing to suggest");
    }

    #[test]
    fn websocket_and_other_render_their_fences() {
        let mut f = form(NewKind::WebSocket);
        f.name = "Feed".into();
        let p = f.plan("wss://{{host}}/").unwrap();
        assert!(p.render().contains("```ws\nGET wss://{{host}}/"));
        f.set_kind(NewKind::Other("graphql"));
        f.url = "https://x.test/graphql".into();
        let p = f.plan("").unwrap();
        let text = p.render();
        assert!(text.contains("```http\nPOST https://x.test/graphql"));
        assert!(text.contains("```graphql query\n"));
        assert!(text.contains("requires: [graphql]"));
    }

    #[test]
    fn file_preview_reports_collisions() {
        let dir = std::env::temp_dir().join(format!("fw-nr-prev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = NewRequestForm::new(dir.clone(), NewKind::Http);
        f.name = "Get users".into();
        assert_eq!(f.file_preview(), Some(("get-users.flux.md".into(), false)));
        std::fs::write(dir.join("get-users.flux.md"), "x").unwrap();
        assert_eq!(f.file_preview(), Some(("get-users-2.flux.md".into(), true)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn switching_kind_keeps_shared_fields() {
        let mut f = form(NewKind::Http);
        f.name = "A".into();
        f.description = "d".into();
        f.url = "https://x".into();
        f.set_kind(NewKind::WebSocket);
        assert_eq!(
            (f.name.as_str(), f.description.as_str(), f.url.as_str()),
            ("A", "d", "https://x")
        );
        assert_eq!(relative_to(Path::new("/w"), Path::new("/w/pay/a")), "pay/a");
    }
}
