//! Turn a parsed `ws` block into a `WsPlan` (SPEC §10): the HTTP upgrade request with the
//! same variables, inherited headers, and auth as HTTP.

use std::time::Duration;

use base64::Engine as _;
use fluxwire_model::{Diagnostic, FluxDoc, FmValue, Severity, Span};
use fluxwire_vars::VarScope;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    None,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
    ApiKey {
        key: String,
        value: String,
        in_query: bool,
    },
}

impl Auth {
    pub fn label(&self) -> &'static str {
        match self {
            Auth::None => "none",
            Auth::Basic { .. } => "basic",
            Auth::Bearer { .. } => "bearer",
            Auth::ApiKey { .. } => "apikey",
        }
    }

    /// The header this auth adds to the upgrade request, if any.
    pub fn header(&self) -> Option<(String, String)> {
        match self {
            Auth::Basic { username, password } => Some((
                "Authorization".into(),
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{password}"))
                ),
            )),
            Auth::Bearer { token } => Some(("Authorization".into(), format!("Bearer {token}"))),
            Auth::ApiKey {
                key,
                value,
                in_query: false,
            } => Some((key.clone(), value.clone())),
            _ => None,
        }
    }

    fn from_front(keyword: Option<&str>, fields: &[(String, String)]) -> Result<Auth, String> {
        let field = |n: &str| {
            fields
                .iter()
                .find(|(k, _)| k == n)
                .map_or("", |(_, v)| v.as_str())
        };
        if fields.is_empty() {
            return match keyword.map(str::trim) {
                None | Some("") | Some("none") | Some("inherit") => Ok(Auth::None),
                Some(other) => Err(format!(
                    "Unknown auth `{other}`. Use none, inherit, or a map with `type`."
                )),
            };
        }
        let ty = field("type");
        let need = |n: &str| match field(n) {
            "" => Err(format!("auth type `{ty}` needs `{n}`.")),
            v => Ok(v.to_string()),
        };
        match ty {
            "none" => Ok(Auth::None),
            "basic" => Ok(Auth::Basic {
                username: need("username")?,
                password: field("password").to_string(),
            }),
            "bearer" => Ok(Auth::Bearer {
                token: need("token")?,
            }),
            "apikey" => Ok(Auth::ApiKey {
                key: need("key")?,
                value: field("value").to_string(),
                in_query: field("in") == "query",
            }),
            "digest" | "oauth2" => Err(format!(
                "auth type `{ty}` is not supported for WebSocket requests yet. Use basic, bearer or apikey."
            )),
            other => Err(format!(
                "Unknown auth type `{other}`. Use none, basic, bearer or apikey."
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WsPlan {
    /// Where to connect, including any api-key query parameter.
    pub url: Url,
    /// `url` without secrets from auth, for display and history.
    pub display_url: String,
    /// Request headers, auth excluded.
    pub headers: Vec<(String, String)>,
    pub auth: Auth,
    pub timeout: Option<Duration>,
    pub verify_tls: Option<bool>,
    pub warnings: Vec<Diagnostic>,
    /// `(name, value)` of secret variables in scope, for value-based redaction.
    pub secrets: Vec<(String, String)>,
}

fn err(code: &'static str, span: Span, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, span, msg)
}

fn map_entries(doc: &FluxDoc, key: &str) -> Vec<(String, String)> {
    match doc.front.get(key).map(|e| &e.value) {
        Some(FmValue::Map(m)) => m.clone(),
        _ => Vec::new(),
    }
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

pub fn plan(doc: &FluxDoc, scope: &VarScope) -> Result<WsPlan, Vec<Diagnostic>> {
    let Some(req) = doc.request() else {
        return Err(vec![err(
            "P100",
            Span::at(0),
            "This file has no ws request block.",
        )]);
    };
    let mut missing: Vec<String> = Vec::new();
    let mut schemes: Vec<String> = Vec::new();
    let mut i = |text: &str| {
        let r = fluxwire_vars::interpolate(text, scope);
        missing.extend(r.missing);
        schemes.extend(r.schemes);
        r.text
    };

    let mut target = i(&req.target);
    for q in req.query.iter().filter(|q| q.enabled) {
        let name = i(&q.name);
        let piece = match &q.value {
            Some(v) => format!("{name}={}", i(v)),
            None => name,
        };
        target.push(if target.contains('?') { '&' } else { '?' });
        target.push_str(&piece);
    }

    let own: Vec<(String, String)> = req
        .headers
        .iter()
        .filter(|h| h.enabled)
        .map(|h| (h.name.clone(), i(&h.value)))
        .collect();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (k, v) in &scope.inherited.headers {
        if !has_header(&own, k) {
            headers.push((k.clone(), i(v)));
        }
    }
    headers.extend(own);

    let own_fields = map_entries(doc, "auth");
    let own_kw = doc.front.str("auth").map(str::to_string);
    let inherits = own_fields.is_empty()
        && matches!(
            own_kw.as_deref().map(str::trim),
            None | Some("") | Some("inherit")
        );
    let (kw, fields) = if inherits {
        match &scope.inherited.auth {
            Some((kw, f)) => (kw.clone(), f.clone()),
            None => (None, Vec::new()),
        }
    } else {
        (own_kw, own_fields)
    };
    let fields: Vec<(String, String)> = fields.into_iter().map(|(k, v)| (k, i(&v))).collect();
    let auth = Auth::from_front(kw.as_deref(), &fields);

    let mut timeout = None;
    let mut verify_tls = None;
    let own_settings = map_entries(doc, "settings");
    for (k, v) in scope.inherited.settings.iter().cloned().chain(own_settings) {
        match k.as_str() {
            "timeout_ms" => timeout = v.trim().parse().ok().map(Duration::from_millis),
            "verify_tls" => verify_tls = v.trim().parse().ok(),
            _ => {}
        }
    }

    let mut diags = Vec::new();
    for s in schemes {
        diags.push(err(
            "P105",
            req.target_span,
            format!(
                "Unknown variable scheme in {{{{{s}}}}}: no resolver is registered for it in this version."
            ),
        ));
    }
    missing.sort();
    missing.dedup();
    let auth = match auth {
        Ok(a) => Some(a),
        Err(e) => {
            diags.push(err("P103", Span::at(0), e));
            None
        }
    };
    let url = match Url::parse(&target) {
        Ok(u) if matches!(u.scheme(), "ws" | "wss") && u.host().is_some() => Some(u),
        Ok(u) => {
            diags.push(err(
                "P104",
                req.target_span,
                format!(
                    "Unsupported URL scheme `{}`. Use ws:// or wss://.",
                    u.scheme()
                ),
            ));
            None
        }
        Err(e) => {
            if missing.is_empty() {
                diags.push(err(
                    "P104",
                    req.target_span,
                    format!("Invalid URL `{target}`: {e}. Start it with ws:// or wss://."),
                ));
            }
            None
        }
    };

    let unresolved = |sev: Severity| {
        Diagnostic::new(
            "P101",
            sev,
            req.target_span,
            format!(
                "Unresolved variable{}: {}. Select an environment or define {} in an environment file.",
                if missing.len() == 1 { "" } else { "s" },
                missing
                    .iter()
                    .map(|m| format!("{{{{{m}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if missing.len() == 1 { "it" } else { "them" },
            ),
        )
    };
    match (url, auth) {
        (Some(mut url), Some(auth)) if diags.is_empty() => {
            let display_url = url.to_string();
            if let Auth::ApiKey {
                key,
                value,
                in_query: true,
            } = &auth
            {
                url.query_pairs_mut().append_pair(key, value);
            }
            let mut warnings = Vec::new();
            if !missing.is_empty() {
                warnings.push(unresolved(Severity::Warning));
            }
            Ok(WsPlan {
                url,
                display_url,
                headers,
                auth,
                timeout,
                verify_tls,
                warnings,
                secrets: scope
                    .vars
                    .iter()
                    .filter(|(_, v)| v.secret)
                    .map(|(n, v)| (n.clone(), v.value.clone()))
                    .collect(),
            })
        }
        _ => {
            if !missing.is_empty() {
                diags.insert(0, unresolved(Severity::Error));
            }
            Err(diags)
        }
    }
}
