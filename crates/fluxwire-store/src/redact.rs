use serde_json::Value;

/// Bodies are stored up to 1 MiB (SPEC §13).
pub const BODY_CAP: usize = 1 << 20;

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
];

/// Replaces secrets with `‹secret:name›` before anything is written (SPEC §8.3).
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// `(variable name, value)` pairs of every secret in scope.
    pub secrets: Vec<(String, String)>,
    /// Extra header names to always redact (e.g. an api-key header).
    pub headers: Vec<String>,
}

impl Redactor {
    pub fn with_secrets(secrets: Vec<(String, String)>) -> Self {
        Self {
            secrets,
            headers: Vec::new(),
        }
    }

    pub fn redact_str(&self, s: &str) -> String {
        let mut out = s.to_string();
        // longest value first so a secret containing another is replaced whole
        let mut sec: Vec<_> = self.secrets.iter().filter(|(_, v)| !v.is_empty()).collect();
        sec.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        for (name, value) in sec {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), &format!("‹secret:{name}›"));
            }
        }
        out
    }

    fn sensitive(&self, name: &str) -> bool {
        SENSITIVE_HEADERS
            .iter()
            .any(|h| name.eq_ignore_ascii_case(h))
            || self.headers.iter().any(|h| name.eq_ignore_ascii_case(h))
    }

    /// Redact in place: secret values everywhere, sensitive header values by name.
    pub fn redact_value(&self, v: &mut Value) {
        match v {
            Value::String(s) => *s = self.redact_str(s),
            Value::Array(a) => a.iter_mut().for_each(|x| self.redact_value(x)),
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if k == "headers" {
                        self.redact_headers(x);
                    }
                    self.redact_value(x);
                }
            }
            _ => {}
        }
    }

    fn redact_headers(&self, v: &mut Value) {
        let Some(list) = v.as_array_mut() else { return };
        for h in list {
            if let Some([name, value]) = h.as_array_mut().map(Vec::as_mut_slice)
                && name.as_str().is_some_and(|n| self.sensitive(n))
            {
                *value = Value::String("‹redacted›".into());
            }
        }
    }
}

/// Cap every `body` object holding `text` or `b64` at [`BODY_CAP`], flagging `truncated`.
/// Returns whether anything was cut.
pub(crate) fn cap_bodies(v: &mut Value) -> bool {
    let mut cut = false;
    match v {
        Value::Array(a) => {
            for x in a {
                cut |= cap_bodies(x);
            }
        }
        Value::Object(m) => {
            if let Some(Value::Object(b)) = m.get_mut("body") {
                for (key, align) in [("text", 1usize), ("b64", 4)] {
                    if let Some(Value::String(s)) = b.get_mut(key)
                        && s.len() > BODY_CAP
                    {
                        let mut n = BODY_CAP - BODY_CAP % align;
                        while !s.is_char_boundary(n) {
                            n -= 1;
                        }
                        s.truncate(n);
                        b.insert("truncated".into(), Value::Bool(true));
                        cut = true;
                    }
                }
            }
            for x in m.values_mut() {
                cut |= cap_bodies(x);
            }
        }
        _ => {}
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_values_and_headers() {
        let r = Redactor::with_secrets(vec![("token".into(), "s3cr3t-value".into())]);
        let mut v = json!({
            "request": {
                "url": "https://x/?k=s3cr3t-value",
                "headers": [["Authorization", "Bearer abc"], ["Accept", "*/*"], ["Cookie", "a=b"]],
                "body": {"text": "{\"t\":\"s3cr3t-value\"}"}
            },
            "response": {"headers": [["set-cookie", "sid=1"], ["X-Other", "s3cr3t-value"]]}
        });
        r.redact_value(&mut v);
        let s = v.to_string();
        assert!(!s.contains("s3cr3t-value") && !s.contains("Bearer abc") && !s.contains("sid=1"));
        assert!(s.contains("‹secret:token›") && s.contains("*/*"));
    }

    #[test]
    fn caps_bodies() {
        let mut v = json!({"response": {"body": {"text": "a".repeat(BODY_CAP + 10)}}});
        assert!(cap_bodies(&mut v));
        assert_eq!(
            v["response"]["body"]["text"].as_str().unwrap().len(),
            BODY_CAP
        );
        assert_eq!(v["response"]["body"]["truncated"], true);
        let mut small = json!({"body": {"text": "ok"}});
        assert!(!cap_bodies(&mut small));
    }
}
