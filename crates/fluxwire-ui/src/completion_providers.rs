//! The completion providers of the code editor (M9.06): buffer words, `{{` variables, the
//! `fw`/`pm` script model, region-aware `.flux.md` completions and Markdown prose helpers.

use crate::completion::{
    CompletionItem, CompletionProvider, Kind, Trigger, is_ident_char, word_start,
};
use crate::markdown_edit::{MdTrigger, heading_anchors, trigger_at};
use fluxwire_script::{CompletionKind, ScriptCompletionCtx};
use std::collections::BTreeSet;
use std::ops::Range;

// ---- words --------------------------------------------------------------------------------

/// Identifiers already in the buffer (Zed's buffer-word completion). Ranked last.
pub struct WordsProvider;

impl CompletionProvider for WordsProvider {
    fn completions(&self, text: &str, offset: usize, _: Trigger) -> Vec<CompletionItem> {
        if text.len() > 200_000 {
            return vec![];
        }
        let ws = word_start(text, offset);
        let mut seen = BTreeSet::new();
        let mut cur = String::new();
        let mut out = Vec::new();
        let mut flush = |w: &mut String, at_end: usize, out: &mut Vec<CompletionItem>| {
            let is_partial = at_end == offset && !w.is_empty();
            if w.chars().count() >= 3 && !is_partial && seen.insert(w.clone()) {
                out.push(CompletionItem::new(w.clone(), Kind::Word, ws..offset).detail("word"));
            }
            w.clear();
        };
        for (i, c) in text.char_indices() {
            if is_ident_char(c) {
                cur.push(c);
            } else {
                flush(&mut cur, i, &mut out);
            }
        }
        flush(&mut cur, text.len(), &mut out);
        out
    }
}

// ---- variables ----------------------------------------------------------------------------

/// Names that resolve in the active scope, offered after `{{`.
pub struct VariablesProvider {
    /// `(name, source)` in the scope's order.
    pub vars: Vec<(String, String)>,
}

impl CompletionProvider for VariablesProvider {
    fn completions(&self, text: &str, offset: usize, _: Trigger) -> Vec<CompletionItem> {
        let ls = text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let before = &text[ls..offset];
        let Some(i) = before.rfind("{{") else {
            return vec![];
        };
        let rest = &before[i + 2..];
        if rest.contains("}}") {
            return vec![];
        }
        let start = ls + i + 2 + (rest.len() - rest.trim_start().len());
        let close = text[offset..].starts_with("}}");
        self.vars
            .iter()
            .map(|(n, src)| {
                CompletionItem::new(n.clone(), Kind::Variable, start..offset)
                    .detail(src.clone())
                    .insert(if close { n.clone() } else { format!("{n}}}}}") })
            })
            .collect()
    }
    fn is_trigger_char(&self, c: char) -> bool {
        c == '{'
    }
}

// ---- scripts ------------------------------------------------------------------------------

pub struct ScriptProvider {
    pub role: fluxwire_script::Role,
    pub variables: Vec<String>,
}

fn kind_of(k: CompletionKind) -> Kind {
    match k {
        CompletionKind::Method => Kind::Method,
        CompletionKind::Property => Kind::Property,
        CompletionKind::Function => Kind::Function,
        CompletionKind::Namespace => Kind::Namespace,
        CompletionKind::Variable => Kind::Variable,
        CompletionKind::Keyword => Kind::Keyword,
        CompletionKind::Snippet => Kind::Snippet,
        CompletionKind::Value => Kind::Value,
    }
}

impl ScriptProvider {
    /// Completions for a script that starts at `base` inside a larger document.
    fn run(&self, src: &str, offset: usize, base: usize) -> Vec<CompletionItem> {
        let ctx = ScriptCompletionCtx {
            variables: &self.variables,
            role: self.role,
        };
        fluxwire_script::script_completions(src, offset, &ctx)
            .into_iter()
            .map(|i| {
                CompletionItem::new(
                    i.label,
                    kind_of(i.kind),
                    i.replace.start + base..i.replace.end + base,
                )
                .detail(i.detail)
                .doc(i.documentation)
                .insert(i.insert_text)
            })
            .collect()
    }
}

impl CompletionProvider for ScriptProvider {
    fn completions(&self, text: &str, offset: usize, _: Trigger) -> Vec<CompletionItem> {
        self.run(text, offset, 0)
    }
    fn is_trigger_char(&self, c: char) -> bool {
        c == '.' || c == '"' || c == '\''
    }
}

// ---- .flux.md regions ---------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Region {
    FrontMatter {
        start: usize,
        end: usize,
    },
    /// On an opening fence line: the info string typed so far.
    FenceInfo {
        partial: Range<usize>,
        typed: String,
    },
    /// Inside a fence: its info string and where its content starts.
    Fence {
        info: String,
        content: usize,
    },
    Prose,
}

pub fn region_at(text: &str, offset: usize) -> Region {
    let mut pos = 0;
    let mut fence: Option<(String, usize)> = None;
    let mut fm_state = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        1
    } else {
        0
    };
    for line in text.split_inclusive('\n') {
        let start = pos;
        let end = pos + line.len();
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let line_end = start + trimmed.len();
        if fm_state == 1 && start > 0 && trimmed == "---" {
            fm_state = 2;
            if offset <= line_end && offset >= start {
                return Region::FrontMatter {
                    start: 0,
                    end: start,
                };
            }
        } else if fm_state == 1 {
            if offset >= start && offset <= line_end.max(start) && start > 0 && fm_closes(text) {
                return Region::FrontMatter {
                    start: 0,
                    end: fm_end(text),
                };
            }
        } else if fm_state == 0 || fm_state == 2 {
            if let Some((info, content)) = &fence {
                if trimmed.trim_start() == "```" && trimmed.len() - trimmed.trim_start().len() <= 3
                {
                    if offset >= start && offset <= line_end {
                        return Region::Prose;
                    }
                    fence = None;
                } else if offset >= start && offset <= end.max(line_end) {
                    return Region::Fence {
                        info: info.clone(),
                        content: *content,
                    };
                }
            } else if let Some(r) = trimmed.strip_prefix("```")
                && !r.starts_with('`')
            {
                if offset >= start + 3 && offset <= line_end {
                    let typed = text[start + 3..offset].to_string();
                    return Region::FenceInfo {
                        partial: start + 3..offset,
                        typed,
                    };
                }
                fence = Some((r.trim().to_string(), end));
            }
        }
        if offset < end || (offset == end && end == text.len() && !line.ends_with('\n')) {
            // Caret on this line and no region claimed it.
            if fm_state == 1 {
                return Region::FrontMatter {
                    start: 0,
                    end: fm_end(text),
                };
            }
            return match &fence {
                Some((info, content)) if offset >= *content => Region::Fence {
                    info: info.clone(),
                    content: *content,
                },
                _ => Region::Prose,
            };
        }
        pos = end;
    }
    match fence {
        Some((info, content)) if offset >= content => Region::Fence { info, content },
        _ => Region::Prose,
    }
}

fn fm_closes(text: &str) -> bool {
    text[3..].lines().any(|l| l.trim_end() == "---")
}

fn fm_end(text: &str) -> usize {
    let mut pos = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if n > 0 && line.trim_end() == "---" {
            return pos;
        }
        pos += line.len();
    }
    text.len()
}

/// Languages × roles offered after an opening fence, with whether they are supported.
const FENCES: &[(&str, &str, bool)] = &[
    ("http", "The request", true),
    ("ws", "A WebSocket request", true),
    ("http expect", "Expected response", true),
    ("http form", "Form-encoded body", true),
    ("http multipart", "Multipart body", true),
    (
        "http defaults",
        "Headers inherited by every request below",
        true,
    ),
    ("ts pre", "Script before the send", true),
    ("ts post", "Script after the response", true),
    ("ts test", "Tests", true),
    ("js pre", "Script before the send", true),
    ("js post", "Script after the response", true),
    ("js test", "Tests", true),
    ("json message", "Saved message", true),
    ("text message", "Saved message", true),
    ("xml message", "Saved message", true),
    ("yaml config", "Nested configuration", true),
    ("json example", "Documentation-only sample", true),
    ("http example", "Documentation-only sample", true),
    ("graphql query", "GraphQL operation", true),
    ("json variables", "GraphQL variables", true),
    ("grpc", "Reserved", false),
    ("sse", "Reserved", false),
    ("mcp", "Reserved", false),
];

const PLAIN_LANGS: &[&str] = &[
    "json", "yaml", "xml", "sh", "ts", "js", "text", "http", "graphql",
];

fn fence_items(partial: Range<usize>, with_roles: bool) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    if with_roles {
        for (info, doc, ok) in FENCES {
            out.push(
                CompletionItem::new(*info, Kind::Keyword, partial.clone())
                    .detail(if *ok {
                        "fence".to_string()
                    } else {
                        "not supported in this version".to_string()
                    })
                    .doc(*doc),
            );
        }
    }
    for l in PLAIN_LANGS {
        if !out.iter().any(|i: &CompletionItem| i.label == *l) {
            out.push(CompletionItem::new(*l, Kind::Keyword, partial.clone()).detail("language"));
        }
    }
    out
}

fn schema_keys() -> Vec<(String, String, String)> {
    let v: serde_json::Value =
        serde_json::from_str(&fluxwire_format::front_matter_schema_json()).unwrap_or_default();
    let Some(props) = v.get("properties").and_then(|p| p.as_object()) else {
        return vec![];
    };
    props
        .iter()
        .map(|(k, p)| {
            let desc = p
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let ty = match p.get("type") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| x.as_str())
                    .find(|s| *s != "null")
                    .unwrap_or("any")
                    .to_string(),
                _ => "object".to_string(),
            };
            (k.clone(), ty, desc)
        })
        .collect()
}

fn enum_values(key: &str) -> &'static [&'static str] {
    match key {
        "kind" => &["request", "folder", "collection"],
        "auth" => &["inherit", "none"],
        "type" => &["basic", "bearer", "apikey", "digest", "oauth2"],
        _ => &[],
    }
}

/// Region-aware completions for `.flux.md` text.
pub struct FluxMdProvider {
    pub variables: Vec<String>,
}

impl CompletionProvider for FluxMdProvider {
    fn completions(&self, text: &str, offset: usize, trigger: Trigger) -> Vec<CompletionItem> {
        match region_at(text, offset) {
            Region::FenceInfo { partial, .. } => fence_items(partial, true),
            Region::FrontMatter { end, .. } => {
                let ls = text[..offset].rfind('\n').map_or(0, |i| i + 1);
                let before = &text[ls..offset];
                if let Some((key, val)) = before.split_once(':') {
                    let k = key.trim();
                    let vs = ls + key.len() + 1 + (val.len() - val.trim_start().len());
                    return enum_values(k)
                        .iter()
                        .map(|v| {
                            CompletionItem::new(*v, Kind::Value, vs..offset)
                                .detail(format!("{k} value"))
                        })
                        .collect();
                }
                if before.starts_with(' ') || before.starts_with('\t') || before.starts_with('#') {
                    return vec![];
                }
                let present: Vec<&str> = text[..end]
                    .lines()
                    .filter(|l| !l.starts_with([' ', '\t', '#', '-']))
                    .filter_map(|l| l.split_once(':').map(|(k, _)| k.trim()))
                    .collect();
                schema_keys()
                    .into_iter()
                    .filter(|(k, _, _)| !present.contains(&k.as_str()))
                    .map(|(k, ty, d)| {
                        CompletionItem::new(k.clone(), Kind::Property, ls..offset)
                            .detail(ty)
                            .doc(d)
                            .insert(format!("{k}: "))
                    })
                    .collect()
            }
            Region::Fence { info, content } => {
                let mut it = info.split_whitespace();
                let (lang, role) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
                let rel = offset - content;
                let body_end = text[content..]
                    .find("\n```")
                    .map_or(text.len() - content, |i| i + 1);
                let body = &text[content..content + body_end];
                match (lang, role) {
                    ("ts" | "js", "pre" | "post" | "test") => {
                        let role = match role {
                            "pre" => fluxwire_script::Role::Pre,
                            "post" => fluxwire_script::Role::Post,
                            _ => fluxwire_script::Role::Test,
                        };
                        let p = ScriptProvider {
                            role,
                            variables: self.variables.clone(),
                        };
                        p.run(body, rel.min(body.len()), content)
                    }
                    ("http", "expect") => {
                        let ls = text[..offset].rfind('\n').map_or(0, |i| i + 1);
                        let first = ls == content;
                        let r = ls..offset;
                        if first {
                            ["HTTP 200", "HTTP 201", "HTTP 2xx", "HTTP 4xx", "HTTP *"]
                                .iter()
                                .map(|s| {
                                    CompletionItem::new(*s, Kind::Value, r.clone())
                                        .detail("status line")
                                })
                                .collect()
                        } else {
                            vec![
                                CompletionItem::new(
                                    "{{*}}",
                                    Kind::Snippet,
                                    word_start(text, offset)..offset,
                                )
                                .detail("matches any value"),
                                CompletionItem::new(
                                    "~//",
                                    Kind::Snippet,
                                    word_start(text, offset)..offset,
                                )
                                .detail("matches a regular expression")
                                .insert("~/$0/"),
                            ]
                        }
                    }
                    _ => vec![],
                }
            }
            Region::Prose => {
                let _ = trigger;
                MarkdownProvider { links: vec![] }.completions(text, offset, trigger)
            }
        }
    }
    fn is_trigger_char(&self, c: char) -> bool {
        matches!(c, '{' | '.' | '"' | '\'' | '`' | '(' | '#')
    }
}

// ---- markdown -----------------------------------------------------------------------------

/// Relative links and heading anchors in prose (fence languages are the `.flux.md` provider's).
pub struct MarkdownProvider {
    /// Collection-relative paths of `.flux.md` and `.md` files.
    pub links: Vec<String>,
}

impl CompletionProvider for MarkdownProvider {
    fn completions(&self, text: &str, offset: usize, _: Trigger) -> Vec<CompletionItem> {
        match trigger_at(text, offset) {
            Some(MdTrigger::FenceInfo { partial }) => fence_items(partial, false),
            Some(MdTrigger::LinkPath { partial }) => self
                .links
                .iter()
                .map(|l| {
                    CompletionItem::new(l.clone(), Kind::Value, partial.clone()).detail("file")
                })
                .collect(),
            Some(MdTrigger::Anchor { partial }) => heading_anchors(text)
                .into_iter()
                .map(|a| CompletionItem::new(a, Kind::Value, partial.clone()).detail("heading"))
                .collect(),
            None => vec![],
        }
    }
    fn is_trigger_char(&self, c: char) -> bool {
        matches!(c, '(' | '#' | '`')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[CompletionItem]) -> Vec<&str> {
        v.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn words_skip_the_partial_word_and_short_or_duplicate_ones() {
        let t = "alpha beta alpha ab alp";
        let v = WordsProvider.completions(t, t.len(), Trigger::Typing);
        assert_eq!(labels(&v), ["alpha", "beta"]);
        assert_eq!(v[0].replace, 20..23);
    }

    #[test]
    fn variables_after_open_braces_insert_the_closer_once() {
        let p = VariablesProvider {
            vars: vec![("token".into(), "staging.env".into())],
        };
        let t = "Bearer {{to";
        let v = p.completions(t, t.len(), Trigger::Char('{'));
        assert_eq!(v[0].label, "token");
        assert_eq!(v[0].detail, "staging.env");
        assert_eq!(v[0].insert_text, "token}}");
        assert_eq!(v[0].replace, 9..11);
        let t2 = "{{to}}";
        assert_eq!(
            p.completions(t2, 4, Trigger::Typing)[0].insert_text,
            "token"
        );
        assert!(p.completions("no braces", 4, Trigger::Manual).is_empty());
        assert!(p.completions("{{a}} b", 7, Trigger::Manual).is_empty());
    }

    #[test]
    fn script_provider_maps_items_and_offsets() {
        let p = ScriptProvider {
            role: fluxwire_script::Role::Post,
            variables: vec!["token".into()],
        };
        let v = p.completions("fw.res", 6, Trigger::Typing);
        assert!(labels(&v).contains(&"response"));
        assert_eq!(v[0].replace, 3..6);
        let v = p.completions("fw.vars.get(\"", 13, Trigger::Char('"'));
        assert_eq!(labels(&v), ["token"]);
    }

    const DOC: &str = "---\nname: A\nseq: 1\n---\n\nIntro\n\n```ts pre\nfw.\n```\n\n```ht\n```\n\n```http expect\nHTTP 200\nX: 1\n```\n";

    #[test]
    fn regions() {
        let at = |needle: &str, add: usize| DOC.find(needle).unwrap() + add;
        assert!(matches!(
            region_at(DOC, at("name", 2)),
            Region::FrontMatter { .. }
        ));
        assert_eq!(region_at(DOC, at("Intro", 2)), Region::Prose);
        assert!(
            matches!(region_at(DOC, at("fw.", 3)), Region::Fence { ref info, .. } if info == "ts pre")
        );
        assert!(matches!(
            region_at(DOC, at("```ht", 5)),
            Region::FenceInfo { .. }
        ));
        assert!(matches!(
            region_at(DOC, at("```ts", 5)),
            Region::FenceInfo { .. }
        ));
        assert_eq!(region_at("plain", 3), Region::Prose);
    }

    #[test]
    fn front_matter_keys_exclude_present_ones_and_carry_docs() {
        let p = FluxMdProvider { variables: vec![] };
        let t = "---\nname: A\n\n---\n";
        let off = t.find("\n\n").unwrap() + 1;
        let v = p.completions(t, off, Trigger::Manual);
        let l = labels(&v);
        assert!(l.contains(&"tags") && l.contains(&"seq") && !l.contains(&"name"));
        let tags = v.iter().find(|i| i.label == "tags").unwrap();
        assert!(!tags.documentation.is_empty() && !tags.detail.is_empty());
        assert_eq!(tags.insert_text, "tags: ");
    }

    #[test]
    fn front_matter_enum_values() {
        let p = FluxMdProvider { variables: vec![] };
        let t = "---\nkind: \n---\n";
        let off = t.find("kind: ").unwrap() + 6;
        let l = labels(&p.completions(t, off, Trigger::Manual)).join(",");
        assert_eq!(l, "request,folder,collection");
    }

    #[test]
    fn fence_info_lists_roles_and_marks_reserved() {
        let p = FluxMdProvider { variables: vec![] };
        let off = DOC.find("```ht").unwrap() + 5;
        let v = p.completions(DOC, off, Trigger::Manual);
        assert!(labels(&v).contains(&"ts pre") && labels(&v).contains(&"http expect"));
        let g = v.iter().find(|i| i.label == "grpc").unwrap();
        assert_eq!(g.detail, "not supported in this version");
        let q = v.iter().find(|i| i.label == "graphql query").unwrap();
        assert_eq!(q.detail, "fence");
    }

    #[test]
    fn script_completions_inside_a_ts_pre_fence() {
        let p = FluxMdProvider { variables: vec![] };
        let off = DOC.find("fw.").unwrap() + 3;
        let v = p.completions(DOC, off, Trigger::Char('.'));
        assert!(labels(&v).contains(&"vars"));
        assert_eq!(v[0].replace.start, off);
    }

    #[test]
    fn expect_fence_offers_status_lines_and_matchers() {
        let p = FluxMdProvider { variables: vec![] };
        let s = DOC.find("HTTP 200").unwrap();
        let v = p.completions(DOC, s + 4, Trigger::Manual);
        assert!(labels(&v).contains(&"HTTP 2xx"));
        let h = DOC.find("X: 1").unwrap() + 3;
        assert!(labels(&p.completions(DOC, h, Trigger::Manual)).contains(&"{{*}}"));
    }

    #[test]
    fn markdown_links_and_anchors() {
        let p = MarkdownProvider {
            links: vec!["auth/login.flux.md".into()],
        };
        let t = "# Hello\n\nsee [x](au";
        assert_eq!(
            labels(&p.completions(t, t.len(), Trigger::Char('('))),
            ["auth/login.flux.md"]
        );
        let t = "# Hello World\n\n[x](#he";
        assert_eq!(
            labels(&p.completions(t, t.len(), Trigger::Char('#'))),
            ["hello-world"]
        );
        let t = "```js";
        assert!(labels(&p.completions(t, t.len(), Trigger::Typing)).contains(&"json"));
    }
}
