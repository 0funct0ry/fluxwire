//! Per-environment state shared by every send in that environment: the cookie jar
//! (SPEC §9.3) and the OAuth2 token cache (SPEC §9.4). Both are protocol-neutral so
//! WebSocket handshakes and later protocols can reuse them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    /// No `Domain` attribute: matches only the exact host.
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; `None` is a session cookie.
    pub expires: Option<i64>,
    pub same_site: Option<String>,
}

impl Cookie {
    fn same_key(&self, o: &Cookie) -> bool {
        self.name == o.name && self.domain == o.domain && self.path == o.path
    }

    fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e <= now)
    }

    fn matches(&self, host: &str, path: &str, https: bool) -> bool {
        let host = host.to_ascii_lowercase();
        let dom_ok = if self.host_only {
            host == self.domain
        } else {
            host == self.domain || host.ends_with(&format!(".{}", self.domain))
        };
        dom_ok && path_match(path, &self.path) && (!self.secure || https)
    }
}

fn path_match(req: &str, cookie: &str) -> bool {
    req == cookie
        || (req.starts_with(cookie)
            && (cookie.ends_with('/') || req[cookie.len()..].starts_with('/')))
}

fn default_path(req_path: &str) -> String {
    match req_path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(i) => req_path[..i].to_string(),
    }
}

/// Parse `Wed, 21 Oct 2015 07:28:00 GMT` and the common variants.
fn parse_http_date(s: &str) -> Option<i64> {
    let toks: Vec<&str> = s.split([' ', ',', '-']).filter(|t| !t.is_empty()).collect();
    let (mut day, mut mon, mut year, mut hms) = (None, None, None, None);
    for t in toks {
        if t.contains(':') {
            let p: Vec<i64> = t.split(':').filter_map(|x| x.parse().ok()).collect();
            if p.len() == 3 {
                hms = Some((p[0], p[1], p[2]));
            }
        } else if let Some(m) = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .position(|m| t.len() >= 3 && t[..3].eq_ignore_ascii_case(m))
        {
            mon = Some(m as i64 + 1);
        } else if let Ok(n) = t.parse::<i64>() {
            if day.is_none() && n <= 31 && t.len() <= 2 {
                day = Some(n);
            } else if year.is_none() {
                year = Some(if n < 70 {
                    n + 2000
                } else if n < 100 {
                    n + 1900
                } else {
                    n
                });
            }
        }
    }
    let (d, m, mut y) = (day?, mon?, year?);
    let (h, mi, sec) = hms?;
    // days_from_civil
    if m <= 2 {
        y -= 1;
    }
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec)
}

#[derive(Debug, Default)]
pub struct CookieJar {
    cookies: Mutex<Vec<Cookie>>,
}

impl CookieJar {
    /// Record one `Set-Cookie` header received from `host` for request path `req_path`.
    pub fn store(&self, host: &str, req_path: &str, set_cookie: &str) {
        self.store_at(host, req_path, set_cookie, now_unix());
    }

    pub fn store_at(&self, host: &str, req_path: &str, set_cookie: &str, now: i64) {
        let host = host.to_ascii_lowercase();
        let mut parts = set_cookie.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut c = Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: host.clone(),
            host_only: true,
            path: default_path(req_path),
            secure: false,
            http_only: false,
            expires: None,
            same_site: None,
        };
        let mut max_age: Option<i64> = None;
        for attr in parts {
            let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "domain" if !v.is_empty() => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    if host == d || host.ends_with(&format!(".{d}")) {
                        c.domain = d;
                        c.host_only = false;
                    } else {
                        return; // a host may only set cookies for itself or a parent
                    }
                }
                "path" if v.starts_with('/') => c.path = v.to_string(),
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                "samesite" => c.same_site = Some(v.to_string()),
                "max-age" => max_age = v.parse().ok(),
                "expires" => c.expires = parse_http_date(v),
                _ => {}
            }
        }
        if let Some(m) = max_age {
            c.expires = Some(now + m);
        }
        let mut jar = self.cookies.lock().unwrap();
        jar.retain(|o| !o.same_key(&c));
        if !c.expired(now) {
            jar.push(c);
        }
    }

    /// `Cookie` request header value for a request, if any cookie applies.
    pub fn header_for(&self, host: &str, path: &str, https: bool) -> Option<String> {
        let now = now_unix();
        let jar = self.cookies.lock().unwrap();
        let mut m: Vec<&Cookie> = jar
            .iter()
            .filter(|c| !c.expired(now) && c.matches(host, path, https))
            .collect();
        if m.is_empty() {
            return None;
        }
        m.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
        Some(
            m.iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    pub fn list(&self) -> Vec<Cookie> {
        let now = now_unix();
        self.cookies
            .lock()
            .unwrap()
            .iter()
            .filter(|c| !c.expired(now))
            .cloned()
            .collect()
    }

    /// Insert or replace (used by the Cookies panel editor).
    pub fn set(&self, c: Cookie) {
        let mut jar = self.cookies.lock().unwrap();
        match jar.iter().position(|o| o.same_key(&c)) {
            Some(i) => jar[i] = c,
            None => jar.push(c),
        }
    }

    pub fn remove(&self, domain: &str, path: &str, name: &str) {
        self.cookies
            .lock()
            .unwrap()
            .retain(|c| !(c.domain == domain && c.path == path && c.name == name));
    }

    pub fn clear(&self) {
        self.cookies.lock().unwrap().clear();
    }

    /// Persistent cookies only (`persist_cookies`, SPEC §9.3).
    pub fn to_json(&self) -> String {
        let keep: Vec<Cookie> = self
            .list()
            .into_iter()
            .filter(|c| c.expires.is_some())
            .collect();
        serde_json::to_string_pretty(&keep).unwrap_or_else(|_| "[]".into())
    }

    pub fn load_json(&self, json: &str) {
        if let Ok(v) = serde_json::from_str::<Vec<Cookie>>(json) {
            *self.cookies.lock().unwrap() = v;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    pub token_type: String,
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` never expires as far as we know.
    pub expires_at: Option<i64>,
    pub scope: Option<String>,
}

impl Token {
    /// Expired, or within `skew` seconds of expiring.
    pub fn is_stale(&self, skew: i64) -> bool {
        self.expires_at.is_some_and(|e| e - skew <= now_unix())
    }
}

#[derive(Debug, Default)]
pub struct TokenCache {
    tokens: Mutex<HashMap<String, Token>>,
}

impl TokenCache {
    pub fn get(&self, key: &str) -> Option<Token> {
        self.tokens.lock().unwrap().get(key).cloned()
    }

    pub fn put(&self, key: impl Into<String>, t: Token) {
        self.tokens.lock().unwrap().insert(key.into(), t);
    }

    pub fn remove(&self, key: &str) {
        self.tokens.lock().unwrap().remove(key);
    }

    pub fn clear(&self) {
        self.tokens.lock().unwrap().clear();
    }
}

/// Shared handle for one environment; cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct EnvState {
    pub name: String,
    pub cookies: Arc<CookieJar>,
    pub tokens: Arc<TokenCache>,
}

impl EnvState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

/// What the Auth tab shows about an OAuth2 token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    pub token_type: String,
    pub expires_at: Option<i64>,
    pub has_refresh: bool,
}

impl From<&Token> for TokenInfo {
    fn from(t: &Token) -> Self {
        Self {
            token_type: t.token_type.clone(),
            expires_at: t.expires_at,
            has_refresh: t.refresh_token.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_dates() {
        assert_eq!(
            parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(parse_http_date("Thu, 01-Jan-1970 00:00:10 GMT"), Some(10));
        assert_eq!(parse_http_date("garbage"), None);
    }

    #[test]
    fn domain_path_secure_matching() {
        let j = CookieJar::default();
        j.store_at("api.example.com", "/v1/x", "a=1; Path=/v1", 0);
        j.store_at("api.example.com", "/", "b=2; Domain=example.com; Secure", 0);
        j.store_at("api.example.com", "/", "c=3", 0);
        assert_eq!(
            j.header_for("api.example.com", "/v1/y", true).as_deref(),
            Some("a=1; b=2; c=3")
        );
        // plain http: no Secure cookie; other path: no /v1 cookie
        assert_eq!(
            j.header_for("api.example.com", "/z", false).as_deref(),
            Some("c=3")
        );
        // sibling host gets only the Domain cookie (and it is Secure)
        assert_eq!(
            j.header_for("www.example.com", "/", true).as_deref(),
            Some("b=2")
        );
        assert_eq!(j.header_for("evil.com", "/", true), None);
    }

    #[test]
    fn rejects_foreign_domain_and_handles_expiry() {
        let j = CookieJar::default();
        j.store_at("a.com", "/", "x=1; Domain=b.com", 0);
        assert!(j.list().is_empty());
        j.store_at("a.com", "/", "x=1; Max-Age=3600", now_unix());
        assert_eq!(j.list().len(), 1);
        j.store_at("a.com", "/", "x=1; Max-Age=0", now_unix());
        assert!(j.list().is_empty());
        j.store_at(
            "a.com",
            "/",
            "y=1; Expires=Thu, 01 Jan 1970 00:00:01 GMT",
            now_unix(),
        );
        assert!(j.list().is_empty());
    }

    #[test]
    fn replace_edit_remove_and_persist() {
        let j = CookieJar::default();
        j.store("h.io", "/", "s=session");
        j.store("h.io", "/", "p=persist; Max-Age=3600");
        j.store("h.io", "/", "s=changed");
        assert_eq!(
            j.list().iter().find(|c| c.name == "s").unwrap().value,
            "changed"
        );
        let k = CookieJar::default();
        k.load_json(&j.to_json());
        assert_eq!(k.list().len(), 1); // session cookie not persisted
        k.remove("h.io", "/", "p");
        assert!(k.list().is_empty());
    }

    #[test]
    fn token_staleness() {
        let t = Token {
            access_token: "a".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: Some(now_unix() + 20),
            scope: None,
        };
        assert!(!t.is_stale(10));
        assert!(t.is_stale(30));
    }
}
