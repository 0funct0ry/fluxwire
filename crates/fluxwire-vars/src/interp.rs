//! `{{name}}` scanning and substitution (SPEC §8.1).

use std::ops::Range;

use crate::{Source, VarScope, dynamic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarRefKind {
    Plain,
    Dynamic,
    ProcessEnv,
    /// `{{scheme:reference}}`; the scheme registry is empty in v1.0.
    Scheme,
}

/// One `{{…}}` occurrence; `range` covers the braces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarRef {
    pub range: Range<usize>,
    pub name: String,
    pub kind: VarRefKind,
}

fn kind_of(name: &str) -> VarRefKind {
    if name.starts_with('$') {
        VarRefKind::Dynamic
    } else if name.starts_with("process.env.") {
        VarRefKind::ProcessEnv
    } else if let Some((scheme, _)) = name.split_once(':') {
        if scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !scheme.is_empty()
        {
            VarRefKind::Scheme
        } else {
            VarRefKind::Plain
        }
    } else {
        VarRefKind::Plain
    }
}

/// Find every unescaped `{{…}}` in `text`.
pub fn refs(text: &str) -> Vec<VarRef> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find("{{").map(|i| i + at) {
        if i > 0 && text.as_bytes()[i - 1] == b'\\' {
            at = i + 2;
            continue;
        }
        let Some(j) = text[i + 2..].find("}}").map(|j| j + i + 2) else {
            break;
        };
        let name = text[i + 2..j].trim().to_string();
        out.push(VarRef {
            range: i..j + 2,
            kind: kind_of(&name),
            name,
        });
        at = j + 2;
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Interp {
    pub text: String,
    /// Names that did not resolve; their `{{…}}` is left in `text`.
    pub missing: Vec<String>,
    /// `{{scheme:ref}}` references: never sent literally, always an error.
    pub schemes: Vec<String>,
    /// Sources of the variables that were substituted.
    pub used: Vec<(String, Source, bool)>,
}

impl Interp {
    pub fn merge(&mut self, other: Interp) {
        self.missing.extend(other.missing);
        self.schemes.extend(other.schemes);
        self.used.extend(other.used);
    }
}

/// Substitute variables from `scope`. `\{{x}}` yields a literal `{{x}}`.
pub fn interpolate(text: &str, scope: &VarScope) -> Interp {
    let mut r = Interp::default();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut at = 0;
    while let Some(i) = text[at..].find("{{").map(|i| i + at) {
        if i > 0 && text.as_bytes()[i - 1] == b'\\' {
            out.push_str(&text[last..i - 1]);
            out.push_str("{{");
            last = i + 2;
            at = i + 2;
            continue;
        }
        let Some(j) = text[i + 2..].find("}}").map(|j| j + i + 2) else {
            break;
        };
        out.push_str(&text[last..i]);
        let name = text[i + 2..j].trim();
        let raw = &text[i..j + 2];
        match kind_of(name) {
            VarRefKind::Dynamic => match dynamic::resolve(name) {
                Some(v) => out.push_str(&v),
                None => {
                    r.missing.push(name.to_string());
                    out.push_str(raw);
                }
            },
            VarRefKind::ProcessEnv => {
                let key = &name["process.env.".len()..];
                match scope
                    .allow_process_env
                    .then(|| std::env::var(key).ok())
                    .flatten()
                {
                    Some(v) => out.push_str(&v),
                    None => {
                        r.missing.push(name.to_string());
                        out.push_str(raw);
                    }
                }
            }
            VarRefKind::Scheme => {
                r.schemes.push(name.to_string());
                out.push_str(raw);
            }
            VarRefKind::Plain => match scope.var(name) {
                Some(v) => {
                    out.push_str(&v.value);
                    r.used.push((name.to_string(), v.source.clone(), v.secret));
                }
                None => {
                    r.missing.push(name.to_string());
                    out.push_str(raw);
                }
            },
        }
        last = j + 2;
        at = j + 2;
    }
    out.push_str(&text[last..]);
    r.text = out;
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, Var};

    fn scope() -> VarScope {
        let mut s = VarScope::default();
        s.set(
            Layer::Env,
            "baseUrl",
            Var {
                value: "http://x".into(),
                secret: false,
                source: Source::new("staging.env", Some(2)),
            },
        );
        s
    }

    #[test]
    fn table() {
        let cases = [
            ("{{baseUrl}}/a", "http://x/a", 0),
            ("{{ baseUrl }}", "http://x", 0),
            ("\\{{baseUrl}}", "{{baseUrl}}", 0),
            ("{{nope}}", "{{nope}}", 1),
            ("a{{unterminated", "a{{unterminated", 0),
            ("{{$base64(ab)}}", "YWI=", 0),
            ("{{$bogus}}", "{{$bogus}}", 1),
            ("{{process.env.HOME}}", "{{process.env.HOME}}", 1),
        ];
        for (input, want, missing) in cases {
            let r = interpolate(input, &scope());
            assert_eq!(r.text, want, "{input}");
            assert_eq!(r.missing.len(), missing, "{input}");
        }
    }

    #[test]
    fn schemes_are_reported_never_substituted() {
        let r = interpolate("{{vault:secret/x#k}}", &scope());
        assert_eq!(r.text, "{{vault:secret/x#k}}");
        assert_eq!(r.schemes, vec!["vault:secret/x#k"]);
        assert!(r.missing.is_empty());
    }

    #[test]
    fn process_env_is_gated_by_the_setting() {
        let mut s = scope();
        assert_eq!(interpolate("{{process.env.PATH}}", &s).missing.len(), 1);
        // PATH exists on every platform we run on.
        s.allow_process_env = true;
        let r = interpolate("{{process.env.PATH}}", &s);
        assert!(r.missing.is_empty());
        assert!(!r.text.is_empty());
    }

    #[test]
    fn refs_have_ranges_and_kinds() {
        let v = refs("a {{x}} \\{{y}} {{$uuid}} {{aws:z}}");
        let got: Vec<_> = v.iter().map(|r| (&*r.name, r.kind.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("x", VarRefKind::Plain),
                ("$uuid", VarRefKind::Dynamic),
                ("aws:z", VarRefKind::Scheme)
            ]
        );
        assert_eq!(&"a {{x}}"[v[0].range.clone()], "{{x}}");
    }

    #[test]
    fn provenance_recorded() {
        let r = interpolate("{{baseUrl}}", &scope());
        assert_eq!(r.used[0].1.describe(), "staging.env, line 2");
    }
}
