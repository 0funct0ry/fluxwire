//! Completion model for scripts (SPEC §17.3, M9.06). Pure: never runs or transpiles user code.
//!
//! The `fw` symbol table is derived from the embedded `fw.d.ts` (the file is generated and
//! regular, so a line scanner reads it; a test pins every namespace member), so it cannot
//! drift from the typings. The `pm` table is checked against `pm.js` by a test.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use crate::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Method,
    Property,
    Function,
    Namespace,
    Variable,
    Keyword,
    Snippet,
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    pub detail: String,
    pub documentation: String,
    /// May contain `$0` and `${1:placeholder}`.
    pub insert_text: String,
    /// Bytes of the source replaced on accept (the partial word).
    pub replace: Range<usize>,
}

pub struct ScriptCompletionCtx<'a> {
    pub variables: &'a [String],
    pub role: Role,
}

#[derive(Debug, Clone)]
enum Ty {
    Named(String),
    Inline(Vec<Member>),
    Opaque,
}

#[derive(Debug, Clone)]
struct Member {
    name: String,
    kind: ItemKind,
    sig: String,
    doc: String,
    ty: Ty,
}

struct Table {
    root: Vec<Member>,
    types: HashMap<String, (Vec<Member>, Option<String>)>,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| parse_dts(crate::fw_dts()))
}

fn parse_dts(src: &str) -> Table {
    // Frames: (name, members, is_interface, extends)
    struct Frame {
        name: String,
        doc: String,
        members: Vec<Member>,
        interface: Option<(String, Option<String>)>,
    }
    let mut root: Vec<Member> = Vec::new();
    let mut types = HashMap::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut doc = String::new();
    let mut in_ns = false;
    for raw in src.lines() {
        let l = raw.trim();
        if l.starts_with("//") || l.is_empty() {
            continue;
        }
        if let Some(d) = l.strip_prefix("/**") {
            doc = d.trim_end_matches("*/").trim().to_string();
            continue;
        }
        if l.starts_with("declare namespace") {
            in_ns = true;
            continue;
        }
        if !in_ns {
            continue;
        }
        let take_doc = std::mem::take(&mut doc);
        if l == "}" || l == "};" {
            let Some(f) = stack.pop() else { continue };
            match f.interface {
                Some((n, ext)) => {
                    types.insert(n, (f.members, ext));
                }
                None => {
                    let m = Member {
                        kind: ItemKind::Namespace,
                        sig: String::new(),
                        name: f.name,
                        doc: f.doc,
                        ty: Ty::Inline(f.members),
                    };
                    match stack.last_mut() {
                        Some(p) => p.members.push(m),
                        None => root.push(m),
                    }
                }
            }
            continue;
        }
        let l = l.strip_prefix("const ").unwrap_or(l);
        if let Some(rest) = l.strip_prefix("interface ") {
            let rest = rest.trim_end_matches('{').trim();
            let (n, ext) = match rest.split_once(" extends ") {
                Some((n, e)) => (n.trim().to_string(), Some(e.trim().to_string())),
                None => (rest.to_string(), None),
            };
            stack.push(Frame {
                name: n.clone(),
                doc: take_doc,
                members: vec![],
                interface: Some((n, ext)),
            });
            continue;
        }
        let l = l
            .strip_prefix("function ")
            .map(|s| (true, s))
            .unwrap_or((false, l));
        let (is_fn, l) = l;
        let l = l.strip_prefix("readonly ").unwrap_or(l);
        if l.ends_with('{') && !l.contains('(') {
            let name = l.trim_end_matches('{').trim().trim_end_matches(':').trim();
            stack.push(Frame {
                name: name.to_string(),
                doc: take_doc,
                members: vec![],
                interface: None,
            });
            continue;
        }
        let name_end = l.find(['(', ':', '?']).unwrap_or(l.len());
        let name = l[..name_end].trim().to_string();
        if name.is_empty() {
            continue;
        }
        let sig = l.trim_end_matches(';').to_string();
        let m = if l[name_end..].starts_with('(') {
            let ret = sig.rsplit_once("): ").map(|(_, r)| r.trim());
            let ty = match ret {
                Some(r)
                    if r.chars().next().is_some_and(|c| c.is_uppercase())
                        && r.chars().all(|c| c.is_alphanumeric()) =>
                {
                    Ty::Named(r.to_string())
                }
                _ => Ty::Opaque,
            };
            Member {
                name,
                kind: if is_fn {
                    ItemKind::Function
                } else {
                    ItemKind::Method
                },
                sig,
                doc: take_doc,
                ty,
            }
        } else {
            let t = sig.split_once(':').map(|(_, t)| t.trim()).unwrap_or("");
            let ty = if t.chars().next().is_some_and(|c| c.is_uppercase())
                && t.chars().all(|c| c.is_alphanumeric())
            {
                Ty::Named(t.to_string())
            } else {
                Ty::Opaque
            };
            Member {
                name,
                kind: ItemKind::Property,
                sig,
                doc: take_doc,
                ty,
            }
        };
        match stack.last_mut() {
            Some(p) => p.members.push(m),
            None => root.push(m),
        }
    }
    Table { root, types }
}

/// Members of a named type including inherited ones.
fn members_of_named(name: &str) -> Vec<Member> {
    let t = table();
    let mut out = Vec::new();
    let mut cur = Some(name.to_string());
    while let Some(n) = cur {
        match t.types.get(&n) {
            Some((m, ext)) => {
                out.extend(m.iter().cloned());
                cur = ext.clone();
            }
            None => break,
        }
    }
    out
}

fn members_of(ty: &Ty) -> Vec<Member> {
    match ty {
        Ty::Named(n) => members_of_named(n),
        Ty::Inline(m) => m.clone(),
        Ty::Opaque => vec![],
    }
}

const PM_TOP: &[(&str, &str)] = &[
    ("environment", "Environment variables"),
    ("variables", "Variables for this run"),
    ("collectionVariables", "Collection variables"),
    ("globals", "Global variables"),
    ("info", "Information about the running request"),
    ("test", "Declares a named test"),
    ("expect", "Starts a Chai-style assertion"),
    ("sendRequest", "Sends a request"),
    ("request", "The request (pre scripts)"),
    ("response", "The response (post scripts and tests)"),
];

fn var_scope() -> Vec<Member> {
    ["get", "set", "unset", "has", "toObject"]
        .iter()
        .map(|n| Member {
            name: n.to_string(),
            kind: ItemKind::Method,
            sig: format!("{n}(…)"),
            doc: String::new(),
            ty: Ty::Opaque,
        })
        .collect()
}

fn pm_members(path: &[&str]) -> Vec<Member> {
    match path {
        [] => PM_TOP
            .iter()
            .map(|(n, d)| Member {
                name: n.to_string(),
                kind: if matches!(*n, "test" | "expect" | "sendRequest") {
                    ItemKind::Method
                } else {
                    ItemKind::Namespace
                },
                sig: n.to_string(),
                doc: d.to_string(),
                ty: Ty::Opaque,
            })
            .collect(),
        ["environment" | "variables" | "collectionVariables" | "globals"] => var_scope(),
        _ => vec![],
    }
}

/// Names of `pm` members, for the drift test.
#[cfg(test)]
pub(crate) fn pm_member_names() -> Vec<&'static str> {
    PM_TOP.iter().map(|(n, _)| *n).collect()
}

/// Top-level `fw` members (names), for tests.
#[cfg(test)]
pub(crate) fn fw_member_names() -> Vec<String> {
    table().root.iter().map(|m| m.name.clone()).collect()
}

// ---- context analysis --------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Seg {
    Name(String),
    Call,
}

/// Lexical state at `offset`: inside a comment or string?
#[derive(Debug, PartialEq, Eq)]
enum Lex {
    Code,
    Comment,
    Str(char),
}

fn lex_state(src: &str, offset: usize) -> Lex {
    let b = src.as_bytes();
    let mut i = 0;
    let mut st = Lex::Code;
    while i < offset {
        match st {
            Lex::Code => match b[i] {
                b'/' if b.get(i + 1) == Some(&b'/') => {
                    // line comment: ends at newline
                    let end = src[i..].find('\n').map(|n| i + n).unwrap_or(src.len());
                    if end >= offset {
                        return Lex::Comment;
                    }
                    i = end;
                }
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    let end = src[i + 2..]
                        .find("*/")
                        .map(|n| i + 4 + n)
                        .unwrap_or(src.len());
                    if end > offset {
                        return Lex::Comment;
                    }
                    i = end - 1;
                }
                q @ (b'"' | b'\'' | b'`') => st = Lex::Str(q as char),
                _ => {}
            },
            Lex::Str(q) => {
                if b[i] == b'\\' {
                    i += 1;
                } else if b[i] == q as u8 || (b[i] == b'\n' && q != '`') {
                    st = Lex::Code;
                }
            }
            Lex::Comment => {}
        }
        i += 1;
    }
    st
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Start of the identifier being typed that ends at `offset`.
fn word_start(src: &str, offset: usize) -> usize {
    let mut s = offset;
    for (i, c) in src[..offset].char_indices().rev() {
        if is_ident(c) {
            s = i;
        } else {
            break;
        }
    }
    s
}

/// The member chain before `end` (exclusive), e.g. `fw.response.headers` → names; None when
/// the text before is not a plain chain.
fn chain_before(src: &str, end: usize) -> Option<Vec<Seg>> {
    let mut segs = Vec::new();
    let mut i = end;
    loop {
        let t = src[..i].trim_end();
        let t2 = t.strip_suffix('.')?;
        let t2 = t2.strip_suffix('?').unwrap_or(t2);
        i = t2.len();
        // consume calls and one identifier
        loop {
            let t = src[..i].trim_end();
            if t.ends_with(')') {
                let mut depth = 0;
                let mut j = t.len();
                for (k, c) in t.char_indices().rev() {
                    match c {
                        ')' => depth += 1,
                        '(' => {
                            depth -= 1;
                            if depth == 0 {
                                j = k;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if depth != 0 {
                    return None;
                }
                segs.push(Seg::Call);
                i = j;
                continue;
            }
            let ws = word_start(src, t.len());
            if ws == t.len() {
                return None;
            }
            segs.push(Seg::Name(src[ws..t.len()].to_string()));
            i = ws;
            break;
        }
        let before = src[..i].trim_end();
        if before.ends_with('.') {
            continue;
        }
        segs.reverse();
        return Some(segs);
    }
}

fn resolve(segs: &[Seg]) -> Vec<Member> {
    let Some(Seg::Name(root)) = segs.first() else {
        return vec![];
    };
    match root.as_str() {
        "fw" => {
            let mut cur = Ty::Inline(table().root.clone());
            let mut called = false;
            let mut last: Option<Member> = None;
            for s in &segs[1..] {
                match s {
                    Seg::Name(n) => {
                        let Some(m) = members_of(&cur).into_iter().find(|m| &m.name == n) else {
                            return vec![];
                        };
                        called = false;
                        cur = match m.kind {
                            ItemKind::Method | ItemKind::Function => Ty::Opaque,
                            _ => m.ty.clone(),
                        };
                        last = Some(m);
                    }
                    Seg::Call => {
                        let Some(m) = last.take() else { return vec![] };
                        if called || !matches!(m.kind, ItemKind::Method | ItemKind::Function) {
                            return vec![];
                        }
                        called = true;
                        cur = m.ty;
                    }
                }
            }
            members_of(&cur)
        }
        "pm" => {
            let names: Vec<&str> = segs[1..]
                .iter()
                .filter_map(|s| match s {
                    Seg::Name(n) => Some(n.as_str()),
                    Seg::Call => None,
                })
                .collect();
            pm_members(&names)
        }
        _ => vec![],
    }
}

const VAR_FNS: &[&str] = &[
    "fw.vars.get",
    "fw.vars.set",
    "fw.vars.setSecret",
    "fw.vars.has",
    "fw.vars.unset",
    "fw.env.get",
    "fw.env.set",
    "fw.env.persist",
    "fw.globals.get",
    "pm.variables.get",
    "pm.variables.set",
    "pm.variables.has",
    "pm.variables.unset",
    "pm.environment.get",
    "pm.environment.set",
    "pm.environment.has",
    "pm.environment.unset",
    "pm.collectionVariables.get",
    "pm.collectionVariables.set",
    "pm.globals.get",
    "pm.globals.set",
];

/// If the caret is in the first string argument of a variable function, the string start.
fn var_string_arg(src: &str, offset: usize) -> Option<usize> {
    let ws = src[..offset]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_ident(*c) && *c != '-' && *c != '.')
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let before = &src[..ws];
    let q = before.chars().last()?;
    if !matches!(q, '"' | '\'' | '`') {
        return None;
    }
    let b = before[..before.len() - 1]
        .trim_end()
        .strip_suffix('(')?
        .trim_end();
    let start = b
        .char_indices()
        .rev()
        .find(|(_, c)| !is_ident(*c) && *c != '.')
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    VAR_FNS.contains(&&b[start..]).then_some(ws)
}

fn snippets(role: Role) -> Vec<(&'static str, &'static str, &'static str)> {
    let _ = role;
    vec![
        (
            "test",
            "fw.test(\"${1:name}\", () => {\n  $0\n});",
            "Declare a test",
        ),
        ("expect", "fw.expect(${1:value}).$0", "Start an assertion"),
        (
            "status",
            "fw.expect(fw.response.status).toBe(${1:200});",
            "Assert the status code",
        ),
        (
            "json",
            "const body = fw.response.json();$0",
            "Parse the JSON response",
        ),
    ]
}

const KEYWORDS: &[&str] = &[
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "await",
    "async",
    "try",
    "catch",
    "throw",
    "new",
    "typeof",
    "true",
    "false",
    "null",
    "undefined",
];
const GLOBALS: &[&str] = &["fw", "pm", "console", "JSON", "Math", "Date", "Promise"];

fn declared_locals(src: &str, before: usize) -> Vec<String> {
    let text = &src[..before];
    let mut out = Vec::new();
    for kw in ["const ", "let ", "var ", "function "] {
        let mut from = 0;
        while let Some(i) = text[from..].find(kw) {
            let s = from + i + kw.len();
            let e = s + text[s..]
                .find(|c: char| !is_ident(c))
                .unwrap_or(text.len() - s);
            let ok_start = from + i == 0 || !is_ident(text[..from + i].chars().last().unwrap());
            if e > s && ok_start && !out.contains(&text[s..e].to_string()) {
                out.push(text[s..e].to_string());
            }
            from = s;
        }
    }
    out
}

fn item_from(m: &Member, replace: Range<usize>, post_only: bool) -> Item {
    let insert = match m.kind {
        ItemKind::Method | ItemKind::Function => format!("{}($0)", m.name),
        _ => m.name.clone(),
    };
    Item {
        label: m.name.clone(),
        kind: m.kind,
        detail: if post_only {
            "post scripts and tests only".to_string()
        } else {
            m.sig.clone()
        },
        documentation: m.doc.clone(),
        insert_text: insert,
        replace,
    }
}

/// Candidates for the caret at `offset` (a char boundary). Unfiltered by the partial word;
/// the UI filters and ranks with the shared fuzzy matcher. Order is the ranking order.
pub fn script_completions(src: &str, offset: usize, ctx: &ScriptCompletionCtx) -> Vec<Item> {
    if offset > src.len() || !src.is_char_boundary(offset) {
        return vec![];
    }
    // Variable names inside a string argument.
    if let Some(start) = var_string_arg(src, offset) {
        return ctx
            .variables
            .iter()
            .map(|v| Item {
                label: v.clone(),
                kind: ItemKind::Variable,
                detail: "variable".into(),
                documentation: String::new(),
                insert_text: v.clone(),
                replace: start..offset,
            })
            .collect();
    }
    if lex_state(src, offset) != Lex::Code {
        return vec![];
    }
    let ws = word_start(src, offset);
    let replace = ws..offset;
    // Type annotation position: `: Name` — no fw items.
    let before = src[..ws].trim_end();
    if before.ends_with(':') && !before.ends_with("::") {
        let line = before.rsplit('\n').next().unwrap_or("");
        if line.contains("let ")
            || line.contains("const ")
            || line.contains("(") && !line.contains('{')
        {
            return vec![];
        }
    }
    if let Some(segs) = chain_before(src, ws) {
        let post_only_roles = ctx.role == Role::Pre;
        return resolve(&segs)
            .iter()
            .map(|m| {
                let post_only = post_only_roles
                    && matches!(segs.as_slice(), [Seg::Name(r), ..] if r == "fw" || r == "pm")
                    && segs.len() == 1
                    && m.name == "response";
                item_from(m, replace.clone(), post_only)
            })
            .map(|mut i| {
                // keep ranking: post-only last
                if i.detail == "post scripts and tests only" {
                    i.documentation = format!("{}\n(unavailable in pre scripts)", i.documentation);
                }
                i
            })
            .collect::<Vec<_>>()
            .into_iter()
            .fold((Vec::new(), Vec::new()), |(mut a, mut b), i| {
                if i.detail == "post scripts and tests only" {
                    b.push(i)
                } else {
                    a.push(i)
                }
                (a, b)
            })
            .pipe_concat();
    }
    if before.ends_with('.') {
        return vec![];
    }
    // Top level.
    let mut out = Vec::new();
    let snips = snippets(ctx.role);
    let push_snips = |out: &mut Vec<Item>| {
        for (label, body, doc) in &snips {
            out.push(Item {
                label: label.to_string(),
                kind: ItemKind::Snippet,
                detail: "snippet".into(),
                documentation: doc.to_string(),
                insert_text: body.to_string(),
                replace: replace.clone(),
            });
        }
    };
    if ctx.role == Role::Test {
        push_snips(&mut out);
    }
    for g in GLOBALS {
        out.push(Item {
            label: g.to_string(),
            kind: ItemKind::Namespace,
            detail: String::new(),
            documentation: String::new(),
            insert_text: g.to_string(),
            replace: replace.clone(),
        });
    }
    for l in declared_locals(src, ws) {
        out.push(Item {
            label: l.clone(),
            kind: ItemKind::Variable,
            detail: "local".into(),
            documentation: String::new(),
            insert_text: l,
            replace: replace.clone(),
        });
    }
    for k in KEYWORDS {
        out.push(Item {
            label: k.to_string(),
            kind: ItemKind::Keyword,
            detail: String::new(),
            documentation: String::new(),
            insert_text: k.to_string(),
            replace: replace.clone(),
        });
    }
    if ctx.role != Role::Test {
        push_snips(&mut out);
    }
    out
}

trait PipeConcat {
    fn pipe_concat(self) -> Vec<Item>;
}
impl PipeConcat for (Vec<Item>, Vec<Item>) {
    fn pipe_concat(mut self) -> Vec<Item> {
        self.0.append(&mut self.1);
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, role: Role) -> Vec<Item> {
        let (s, _) = src
            .split_once('|')
            .map(|(a, b)| (format!("{a}{b}"), a.len()))
            .unwrap();
        let off = src.find('|').unwrap();
        let vars = vec!["token".to_string(), "baseUrl".to_string()];
        script_completions(
            &s,
            off,
            &ScriptCompletionCtx {
                variables: &vars,
                role,
            },
        )
    }
    fn labels(v: &[Item]) -> Vec<&str> {
        v.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn fw_namespace_lists_members_with_signatures_and_docs() {
        let v = at("fw.|", Role::Post);
        let l = labels(&v);
        for n in [
            "request",
            "response",
            "vars",
            "env",
            "globals",
            "test",
            "expect",
            "sendRequest",
            "sleep",
            "skip",
            "crypto",
            "jsonpath",
        ] {
            assert!(l.contains(&n), "missing {n}");
        }
        let t = v.iter().find(|i| i.label == "test").unwrap();
        assert!(t.detail.contains("name: string") && !t.documentation.is_empty());
        assert_eq!(t.kind, ItemKind::Function);
    }

    #[test]
    fn every_dts_namespace_member_is_known() {
        let names = fw_member_names();
        for line in crate::fw_dts().lines() {
            let l = line.trim();
            if line.starts_with("  ") && !line.starts_with("   ") {
                for p in ["const ", "function "] {
                    if let Some(r) = l.strip_prefix(p) {
                        let n: String = r.chars().take_while(|c| is_ident(*c)).collect();
                        assert!(names.contains(&n), "{n} missing");
                    }
                }
            }
        }
    }

    #[test]
    fn headers_chain() {
        let l = labels(&at("fw.response.headers.|", Role::Post)).join(",");
        assert_eq!(l, "get,has,all");
        let r = labels(&at("fw.request.headers.|", Role::Pre)).join(",");
        assert!(r.contains("set") && r.contains("remove") && r.contains("get"));
    }

    #[test]
    fn expect_chain_and_not() {
        let v = at("fw.expect(1).|", Role::Test);
        assert!(labels(&v).contains(&"toBe") && labels(&v).contains(&"not"));
        let n = at("fw.expect(1).not.|", Role::Test);
        assert!(labels(&n).contains(&"toEqual"));
    }

    #[test]
    fn json_result_is_any() {
        assert!(at("fw.response.json().|", Role::Post).is_empty());
    }

    #[test]
    fn partial_word_is_replaced() {
        let v = at("fw.res|", Role::Post);
        assert_eq!(v[0].replace, 3..6);
    }

    #[test]
    fn response_ranked_last_in_pre() {
        let v = at("fw.|", Role::Pre);
        let last = v.last().unwrap();
        assert_eq!(last.label, "response");
        assert_eq!(last.detail, "post scripts and tests only");
        let post = at("fw.|", Role::Post);
        assert_ne!(post.last().unwrap().detail, "post scripts and tests only");
    }

    #[test]
    fn variable_names_in_string_args() {
        for s in [
            "fw.vars.get(\"|",
            "fw.env.set('|",
            "pm.environment.get(\"to|",
            "fw.globals.get(\"|",
        ] {
            let v = at(s, Role::Pre);
            assert_eq!(labels(&v), ["token", "baseUrl"], "{s}");
            assert_eq!(v[0].kind, ItemKind::Variable);
        }
        assert!(at("console.log(\"|", Role::Pre).is_empty());
    }

    #[test]
    fn top_level_has_globals_keywords_locals_and_snippets() {
        let v = at("const abc = 1;\n|", Role::Post);
        let l = labels(&v);
        for n in [
            "fw", "pm", "console", "JSON", "Math", "Date", "Promise", "await", "const", "abc",
            "test",
        ] {
            assert!(l.contains(&n), "missing {n}");
        }
    }

    #[test]
    fn test_role_ranks_snippets_first() {
        let v = at("|", Role::Test);
        assert_eq!(v[0].kind, ItemKind::Snippet);
        let s = v.iter().find(|i| i.label == "test").unwrap();
        assert_eq!(s.insert_text, "fw.test(\"${1:name}\", () => {\n  $0\n});");
    }

    #[test]
    fn no_members_in_comments_or_strings() {
        assert!(at("// fw.|", Role::Post).is_empty());
        assert!(at("/* fw.| */", Role::Post).is_empty());
        assert!(at("const s = \"fw.|\";", Role::Post).is_empty());
        assert!(!at("// done\nfw.|", Role::Post).is_empty());
    }

    #[test]
    fn no_fw_items_in_type_positions() {
        assert!(at("let x: |", Role::Post).is_empty());
    }

    #[test]
    fn unfinished_expression() {
        assert!(!at("const r = await fw.sendRequest(\"a\"); fw.v|", Role::Pre).is_empty());
        assert!(labels(&at("if (fw.vars.|", Role::Pre)).contains(&"get"));
    }

    #[test]
    fn pm_members() {
        let l = labels(&at("pm.|", Role::Post)).join(",");
        assert!(l.contains("environment") && l.contains("test"));
        assert!(labels(&at("pm.variables.|", Role::Post)).contains(&"get"));
    }

    #[test]
    fn pm_table_matches_pm_js() {
        let js = include_str!("pm.js");
        let start = js.find("const pm = strict({").unwrap();
        let end = js[start..].find("}, \"pm\");").unwrap() + start;
        let mut keys: Vec<String> = js[start..end]
            .lines()
            .skip(1)
            .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
            .filter_map(|l| {
                let t = l.trim();
                let n: String = t.chars().take_while(|c| is_ident(*c)).collect();
                (!n.is_empty() && (t[n.len()..].starts_with(':') || t[n.len()..].starts_with('(')))
                    .then_some(n)
            })
            .collect();
        keys.push("request".into());
        keys.push("response".into());
        keys.sort();
        let mut table: Vec<String> = pm_member_names().iter().map(|s| s.to_string()).collect();
        table.sort();
        assert_eq!(keys, table);
    }
}
