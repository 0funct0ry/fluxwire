//! Request authentication: none, basic, bearer, apikey, digest (SPEC §9.4).
//! `inherit` and `oauth2` arrive with M4.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest as _, Md5};
use sha2::{Sha256, Sha512_256};

#[derive(Debug, Clone, PartialEq)]
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
    Digest {
        username: String,
        password: String,
    },
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> &'a str {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .unwrap_or("")
}

impl Auth {
    pub fn from_front(keyword: Option<&str>, fields: &[(String, String)]) -> Result<Auth, String> {
        if fields.is_empty() {
            return match keyword.map(str::trim) {
                None | Some("") | Some("none") | Some("inherit") => Ok(Auth::None),
                Some(other) => Err(format!(
                    "Unknown auth `{other}`. Use none, inherit, or a map with `type`."
                )),
            };
        }
        let ty = field(fields, "type");
        let need = |name: &str| -> Result<String, String> {
            let v = field(fields, name);
            if v.is_empty() {
                Err(format!("auth type `{ty}` needs `{name}`."))
            } else {
                Ok(v.to_string())
            }
        };
        match ty {
            "none" => Ok(Auth::None),
            "basic" => Ok(Auth::Basic {
                username: need("username")?,
                password: field(fields, "password").to_string(),
            }),
            "bearer" => Ok(Auth::Bearer {
                token: need("token")?,
            }),
            "apikey" => Ok(Auth::ApiKey {
                key: need("key")?,
                value: field(fields, "value").to_string(),
                in_query: field(fields, "in") == "query",
            }),
            "digest" => Ok(Auth::Digest {
                username: need("username")?,
                password: field(fields, "password").to_string(),
            }),
            "oauth2" => Err("OAuth2 arrives in M4.".into()),
            "" => Err("auth map needs a `type`.".into()),
            other => Err(format!(
                "Unknown auth type `{other}`. Use none, basic, bearer, apikey or digest."
            )),
        }
    }

    /// Header (name, value) to send up front, if any.
    pub fn header(&self) -> Option<(String, String)> {
        match self {
            Auth::Basic { username, password } => Some((
                "Authorization".into(),
                format!(
                    "Basic {}",
                    STANDARD.encode(format!("{username}:{password}"))
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

    pub fn secrets(&self) -> Vec<&str> {
        match self {
            Auth::None => vec![],
            Auth::Basic { password, .. } | Auth::Digest { password, .. } => vec![password],
            Auth::Bearer { token } => vec![token],
            Auth::ApiKey { value, .. } => vec![value],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: String,
    pub qop_auth: bool,
}

fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq]
            .trim()
            .trim_start_matches(',')
            .trim()
            .to_ascii_lowercase();
        let after = rest[eq + 1..].trim_start();
        let (val, tail) = if let Some(q) = after.strip_prefix('"') {
            match q.find('"') {
                Some(e) => (q[..e].to_string(), &q[e + 1..]),
                None => (q.to_string(), ""),
            }
        } else {
            match after.find(',') {
                Some(e) => (after[..e].trim().to_string(), &after[e..]),
                None => (after.trim().to_string(), ""),
            }
        };
        out.push((key, val));
        rest = tail.trim_start().trim_start_matches(',').trim_start();
    }
    out
}

pub fn parse_challenge(header: &str) -> Option<Challenge> {
    let rest = header
        .trim()
        .strip_prefix("Digest")
        .or_else(|| header.trim().strip_prefix("digest"))?;
    let p = parse_params(rest);
    let get = |k: &str| p.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    Some(Challenge {
        realm: get("realm").unwrap_or_default(),
        nonce: get("nonce")?,
        opaque: get("opaque"),
        algorithm: get("algorithm").unwrap_or_else(|| "MD5".into()),
        qop_auth: get("qop").is_some_and(|q| q.split(',').any(|x| x.trim() == "auth")),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(alg: &str, s: &str) -> String {
    match alg.to_ascii_uppercase().trim_end_matches("-SESS") {
        "SHA-256" => hex(&Sha256::digest(s.as_bytes())),
        "SHA-512-256" => hex(&Sha512_256::digest(s.as_bytes())),
        _ => hex(&Md5::digest(s.as_bytes())),
    }
}

/// Compute the `Authorization: Digest ...` value (RFC 7616, qop=auth).
pub fn digest_header(
    c: &Challenge,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    cnonce: &str,
) -> String {
    let alg = c.algorithm.as_str();
    let mut ha1 = hash(alg, &format!("{username}:{}:{password}", c.realm));
    if alg.to_ascii_uppercase().ends_with("-SESS") {
        ha1 = hash(alg, &format!("{ha1}:{}:{cnonce}", c.nonce));
    }
    let ha2 = hash(alg, &format!("{method}:{uri}"));
    let nc = "00000001";
    let response = if c.qop_auth {
        hash(alg, &format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", c.nonce))
    } else {
        hash(alg, &format!("{ha1}:{}:{ha2}", c.nonce))
    };
    let mut h = format!(
        "Digest username=\"{username}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", algorithm={alg}, response=\"{response}\"",
        c.realm, c.nonce
    );
    if c.qop_auth {
        h.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(o) = &c.opaque {
        h.push_str(&format!(", opaque=\"{o}\""));
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_and_bearer_headers() {
        let a = Auth::from_front(
            None,
            &[
                ("type".into(), "basic".into()),
                ("username".into(), "u".into()),
                ("password".into(), "p".into()),
            ],
        )
        .unwrap();
        assert_eq!(a.header().unwrap().1, "Basic dTpw");
        let b = Auth::from_front(
            None,
            &[
                ("type".into(), "bearer".into()),
                ("token".into(), "t".into()),
            ],
        )
        .unwrap();
        assert_eq!(b.header().unwrap().1, "Bearer t");
    }

    #[test]
    fn apikey_in_query_has_no_header() {
        let a = Auth::from_front(
            None,
            &[
                ("type".into(), "apikey".into()),
                ("key".into(), "k".into()),
                ("value".into(), "v".into()),
                ("in".into(), "query".into()),
            ],
        )
        .unwrap();
        assert!(a.header().is_none());
    }

    #[test]
    fn missing_fields_and_unknown_types_error() {
        assert!(Auth::from_front(None, &[("type".into(), "bearer".into())]).is_err());
        assert!(Auth::from_front(Some("weird"), &[]).is_err());
        assert_eq!(Auth::from_front(Some("inherit"), &[]).unwrap(), Auth::None);
    }

    // RFC 2617 section 3.5 example.
    #[test]
    fn digest_rfc2617_vector() {
        let c = parse_challenge(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        )
        .unwrap();
        let h = digest_header(
            &c,
            "Mufasa",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            "0a4f113b",
        );
        assert!(
            h.contains("response=\"6629fae49393a05397450978507c4ef1\""),
            "{h}"
        );
    }
}
