//! Completion model for the code editor (M9.06): items, providers, fuzzy ranking and snippet
//! expansion. Pure (no GPUI), so every provider is unit-tested.
//!
//! Providers are synchronous: they only read the text and small tables, so there is no task
//! to cancel. The editor still drops a result whose revision or caret no longer matches.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::ops::Range;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Method,
    Property,
    Function,
    Namespace,
    Variable,
    Keyword,
    Snippet,
    Value,
    Word,
}

impl Kind {
    /// Lucide icon (from the bundled set) for the menu row.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Method | Kind::Function => "code",
            Kind::Property => "square",
            Kind::Namespace => "layers",
            Kind::Variable => "link",
            Kind::Keyword => "pin",
            Kind::Snippet => "file-plus",
            Kind::Value => "check",
            Kind::Word => "search",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Kind,
    /// Signature or source, shown muted beside the label.
    pub detail: String,
    pub documentation: String,
    /// May contain one `$0` caret stop and one `${1:placeholder}` selected on insert.
    pub insert_text: String,
    /// Bytes of the document replaced on accept.
    pub replace: Range<usize>,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>, kind: Kind, replace: Range<usize>) -> Self {
        let label = label.into();
        Self {
            insert_text: label.clone(),
            label,
            kind,
            detail: String::new(),
            documentation: String::new(),
            replace,
        }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }
    pub fn doc(mut self, d: impl Into<String>) -> Self {
        self.documentation = d.into();
        self
    }
    pub fn insert(mut self, t: impl Into<String>) -> Self {
        self.insert_text = t.into();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// A trigger character was typed.
    Char(char),
    /// An identifier character was typed.
    Typing,
    /// `editor: show completions`.
    Manual,
}

pub trait CompletionProvider {
    /// Candidates at `offset` (a char boundary). Not filtered by the partial word.
    fn completions(&self, text: &str, offset: usize, trigger: Trigger) -> Vec<CompletionItem>;
    /// Characters that open the menu without a preceding identifier character.
    fn is_trigger_char(&self, _c: char) -> bool {
        false
    }
}

/// Providers merged in order; an earlier provider ranks above a later one, and a label
/// already offered is not repeated.
pub struct Composite(pub Vec<Rc<dyn CompletionProvider>>);

impl CompletionProvider for Composite {
    fn completions(&self, text: &str, offset: usize, trigger: Trigger) -> Vec<CompletionItem> {
        let mut out: Vec<CompletionItem> = Vec::new();
        for p in &self.0 {
            for it in p.completions(text, offset, trigger) {
                if !out.iter().any(|o| o.label == it.label) {
                    out.push(it);
                }
            }
        }
        out
    }
    fn is_trigger_char(&self, c: char) -> bool {
        self.0.iter().any(|p| p.is_trigger_char(c))
    }
}

/// An item with the indices of its label that matched the typed text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ranked {
    pub item: CompletionItem,
    pub matched: Vec<u32>,
}

/// Filters `items` by `query` with the fuzzy matcher. An empty query keeps provider order;
/// otherwise best score first, ties by provider order.
pub fn rank(items: Vec<CompletionItem>, query: &str) -> Vec<Ranked> {
    if query.is_empty() {
        return items
            .into_iter()
            .map(|item| Ranked {
                item,
                matched: vec![],
            })
            .collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pat = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize, Ranked)> = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        let hay = Utf32Str::new(&item.label, &mut buf);
        let mut idx = Vec::new();
        if let Some(score) = pat.indices(hay, &mut matcher, &mut idx) {
            idx.sort_unstable();
            idx.dedup();
            scored.push((score, i, Ranked { item, matched: idx }));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|s| s.2).collect()
}

/// Result of expanding an item's insert text: the text and where the caret (or selection)
/// goes, relative to the start of the inserted text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    pub selection: Range<usize>,
}

/// Expands one `$0` caret stop and one `${n:placeholder}` (selected). With both, the
/// placeholder is selected; with neither, the caret goes to the end.
pub fn expand(insert: &str) -> Expanded {
    let mut text = String::new();
    let mut caret: Option<usize> = None;
    let mut sel: Option<Range<usize>> = None;
    let mut rest = insert;
    while let Some(i) = rest.find('$') {
        text.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(t) = tail.strip_prefix("$0") {
            caret.get_or_insert(text.len());
            rest = t;
        } else if let Some(t) = tail.strip_prefix("${")
            && let Some(end) = t.find('}')
        {
            let body = &t[..end];
            let ph = body.split_once(':').map_or("", |(_, p)| p);
            let start = text.len();
            text.push_str(ph);
            sel.get_or_insert(start..text.len());
            rest = &t[end + 1..];
        } else {
            text.push('$');
            rest = &tail[1..];
        }
    }
    text.push_str(rest);
    let selection = sel
        .or(caret.map(|c| c..c))
        .unwrap_or(text.len()..text.len());
    Expanded { text, selection }
}

pub fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Start of the identifier ending at `offset`.
pub fn word_start(text: &str, offset: usize) -> usize {
    let mut s = offset;
    for (i, c) in text[..offset].char_indices().rev() {
        if is_ident_char(c) {
            s = i;
        } else {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(l: &str) -> CompletionItem {
        CompletionItem::new(l, Kind::Word, 0..0)
    }

    #[test]
    fn rank_filters_and_orders_by_score_then_provider_order() {
        let r = rank(
            vec![
                item("status"),
                item("statusText"),
                item("sleep"),
                item("xyz"),
            ],
            "stat",
        );
        let l: Vec<_> = r.iter().map(|r| r.item.label.as_str()).collect();
        assert_eq!(l, ["status", "statusText"]);
        assert_eq!(r[0].matched, [0, 1, 2, 3]);
    }

    #[test]
    fn rank_is_fuzzy_and_empty_keeps_order() {
        let r = rank(vec![item("toBeGreaterThan"), item("toBe")], "tbgt");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.label, "toBeGreaterThan");
        let all = rank(vec![item("b"), item("a")], "");
        assert_eq!(all[0].item.label, "b");
    }

    #[test]
    fn expand_caret_and_placeholder() {
        assert_eq!(
            expand("fw.expect(${1:value}).$0"),
            Expanded {
                text: "fw.expect(value).".into(),
                selection: 10..15
            }
        );
        assert_eq!(
            expand("get($0)"),
            Expanded {
                text: "get()".into(),
                selection: 4..4
            }
        );
        assert_eq!(
            expand("plain"),
            Expanded {
                text: "plain".into(),
                selection: 5..5
            }
        );
        assert_eq!(expand("cost $5").text, "cost $5");
        let t = expand("fw.test(\"${1:name}\", () => {\n  $0\n});");
        assert_eq!(t.text, "fw.test(\"name\", () => {\n  \n});");
        assert_eq!(&t.text[t.selection.clone()], "name");
    }

    #[test]
    fn composite_merges_in_order_without_duplicates() {
        struct P(&'static [&'static str]);
        impl CompletionProvider for P {
            fn completions(&self, _: &str, _: usize, _: Trigger) -> Vec<CompletionItem> {
                self.0.iter().map(|l| item(l)).collect()
            }
            fn is_trigger_char(&self, c: char) -> bool {
                c == '.' && self.0.len() == 2
            }
        }
        let c = Composite(vec![Rc::new(P(&["a", "b"])), Rc::new(P(&["b", "c"]))]);
        let l: Vec<_> = c
            .completions("", 0, Trigger::Manual)
            .into_iter()
            .map(|i| i.label)
            .collect();
        assert_eq!(l, ["a", "b", "c"]);
        assert!(c.is_trigger_char('.') && !c.is_trigger_char('x'));
    }

    #[test]
    fn word_start_finds_identifier() {
        assert_eq!(word_start("a.bcd", 5), 2);
        assert_eq!(word_start("a.", 2), 2);
    }
}
