//! Fuzzy index over the workspace (SPEC §17.5): requests, folders, environments, docs headings,
//! recent history entries, recent workspaces and palette actions, ranked with nucleo.

mod fixture;

use std::collections::HashMap;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub use fixture::generate_requests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Request,
    Folder,
    Environment,
    Doc,
    History,
    Workspace,
    Action,
}

/// One searchable thing. `id` is stable and unique within the index (e.g. `req:auth/login.flux.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub detail: String,
    /// Extra text matched but not shown (URL, method, path).
    pub extra: String,
    pub tags: Vec<String>,
}

impl Item {
    pub fn new(id: impl Into<String>, kind: Kind, label: impl Into<String>) -> Self {
        Item {
            id: id.into(),
            kind,
            label: label.into(),
            detail: String::new(),
            extra: String::new(),
            tags: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn extra(mut self, extra: impl Into<String>) -> Self {
        self.extra = extra.into();
        self
    }

    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// No prefix: everything except palette actions.
    All,
    Actions,
    Environments,
    Tags,
    Folders,
    History,
}

/// Splits a leading filter prefix (`>` `@` `#` `/` `!`) off the query.
pub fn parse_query(raw: &str) -> (Filter, &str) {
    let mut chars = raw.chars();
    let filter = match chars.next() {
        Some('>') => Filter::Actions,
        Some('@') => Filter::Environments,
        Some('#') => Filter::Tags,
        Some('/') => Filter::Folders,
        Some('!') => Filter::History,
        _ => return (Filter::All, raw),
    };
    (filter, chars.as_str().trim_start())
}

impl Filter {
    fn accepts(self, item: &Item) -> bool {
        match self {
            Filter::All => item.kind != Kind::Action,
            Filter::Actions => item.kind == Kind::Action,
            Filter::Environments => item.kind == Kind::Environment,
            Filter::Tags => !item.tags.is_empty(),
            Filter::Folders => item.kind == Kind::Folder,
            Filter::History => item.kind == Kind::History,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Hit {
    /// Position in `Index::items()` at query time.
    pub pos: usize,
    pub score: u32,
    /// Char indices in `label` to highlight.
    pub label_indices: Vec<u32>,
}

pub struct Index {
    items: Vec<Item>,
    haystacks: Vec<String>,
    by_id: HashMap<String, usize>,
    /// id → recency rank (0 = most recent).
    recency: HashMap<String, usize>,
    matcher: Matcher,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            items: Vec::new(),
            haystacks: Vec::new(),
            by_id: HashMap::new(),
            recency: HashMap::new(),
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }
}

fn haystack(item: &Item) -> String {
    let mut h = String::with_capacity(item.label.len() + item.detail.len() + item.extra.len() + 2);
    h.push_str(&item.label);
    h.push(' ');
    h.push_str(&item.detail);
    if !item.extra.is_empty() {
        h.push(' ');
        h.push_str(&item.extra);
    }
    h
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn item(&self, pos: usize) -> Option<&Item> {
        self.items.get(pos)
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.by_id.get(id).map(|&p| &self.items[p])
    }

    /// Makes the index equal to `want`, touching only items that were added, removed or changed
    /// (the incremental update path for file-watcher changes). Returns how many items changed.
    pub fn sync(&mut self, want: Vec<Item>) -> usize {
        let keep: std::collections::HashSet<&str> = want.iter().map(|i| i.id.as_str()).collect();
        let stale: Vec<String> = self
            .items
            .iter()
            .filter(|i| !keep.contains(i.id.as_str()))
            .map(|i| i.id.clone())
            .collect();
        let mut changed = stale.len();
        for id in stale {
            self.remove(&id);
        }
        for item in want {
            if self.get(&item.id) != Some(&item) {
                changed += 1;
                self.upsert(item);
            }
        }
        changed
    }

    pub fn upsert(&mut self, item: Item) {
        let hay = haystack(&item);
        match self.by_id.get(&item.id) {
            Some(&pos) => {
                self.items[pos] = item;
                self.haystacks[pos] = hay;
            }
            None => {
                self.by_id.insert(item.id.clone(), self.items.len());
                self.items.push(item);
                self.haystacks.push(hay);
            }
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let Some(pos) = self.by_id.remove(id) else {
            return false;
        };
        self.items.swap_remove(pos);
        self.haystacks.swap_remove(pos);
        if let Some(moved) = self.items.get(pos) {
            self.by_id.insert(moved.id.clone(), pos);
        }
        true
    }

    /// Replaces every item of `kind` with `items` (used for envs, history, recent workspaces).
    pub fn replace_kind(&mut self, kind: Kind, items: impl IntoIterator<Item = Item>) {
        let stale: Vec<String> = self
            .items
            .iter()
            .filter(|i| i.kind == kind)
            .map(|i| i.id.clone())
            .collect();
        for id in stale {
            self.remove(&id);
        }
        for item in items {
            self.upsert(item);
        }
    }

    /// Most recently used first; ids not present are ignored at query time.
    pub fn set_recency(&mut self, ids: impl IntoIterator<Item = String>) {
        self.recency = ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    }

    fn boost(&self, id: &str) -> u32 {
        match self.recency.get(id) {
            Some(&rank) => 64u32.saturating_sub(rank as u32 * 8),
            None => 0,
        }
    }

    /// Ranked hits for a raw query (prefix included). Empty needle lists recent items first.
    pub fn query(&mut self, raw: &str, limit: usize) -> Vec<Hit> {
        let (filter, needle) = parse_query(raw);
        let mut hits: Vec<Hit> = Vec::new();
        let needle = needle.trim();
        if needle.is_empty() {
            for (pos, item) in self.items.iter().enumerate() {
                if filter.accepts(item) {
                    let boost = self.boost(&item.id);
                    hits.push(Hit {
                        pos,
                        score: boost,
                        label_indices: Vec::new(),
                    });
                }
            }
            // Stable: recent first, then insertion order.
            hits.sort_by(|a, b| b.score.cmp(&a.score));
            hits.truncate(limit);
            return hits;
        }

        let pattern = Pattern::new(
            needle,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut buf = Vec::new();
        for pos in 0..self.items.len() {
            let item = &self.items[pos];
            if !filter.accepts(item) {
                continue;
            }
            let hay = if filter == Filter::Tags {
                // Match against tags only; `#` means "tag filter".
                item.tags.join(" ")
            } else {
                self.haystacks[pos].clone()
            };
            let Some(score) = pattern.score(Utf32Str::new(&hay, &mut buf), &mut self.matcher)
            else {
                continue;
            };
            let boost = self.boost(&self.items[pos].id);
            hits.push(Hit {
                pos,
                score: score + boost,
                label_indices: Vec::new(),
            });
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.pos.cmp(&b.pos)));
        hits.truncate(limit);
        if filter != Filter::Tags {
            for hit in &mut hits {
                let mut indices = Vec::new();
                let label = &self.items[hit.pos].label;
                pattern.indices(
                    Utf32Str::new(label, &mut buf),
                    &mut self.matcher,
                    &mut indices,
                );
                indices.sort_unstable();
                indices.dedup();
                hit.label_indices = indices;
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let mut ix = Index::new();
        ix.upsert(
            Item::new(
                "req:payments/create.flux.md",
                Kind::Request,
                "Create payment",
            )
            .detail("payments/create.flux.md")
            .extra("POST {{baseUrl}}/v1/payments")
            .tags(vec!["billing".into()]),
        );
        ix.upsert(
            Item::new("req:auth/login.flux.md", Kind::Request, "Login")
                .detail("auth/login.flux.md"),
        );
        ix.upsert(Item::new("folder:payments", Kind::Folder, "payments"));
        ix.upsert(Item::new("env:staging", Kind::Environment, "staging"));
        ix.upsert(Item::new(
            "hist:1",
            Kind::History,
            "POST Create payment 201",
        ));
        ix.upsert(Item::new("act:save", Kind::Action, "file: save"));
        ix
    }

    fn ids(ix: &mut Index, q: &str) -> Vec<String> {
        ix.query(q, 20)
            .into_iter()
            .map(|h| ix.items[h.pos].id.clone())
            .collect()
    }

    #[test]
    fn prefixes_filter_kinds() {
        let mut ix = sample();
        assert_eq!(ids(&mut ix, "@stag"), ["env:staging"]);
        assert_eq!(ids(&mut ix, "/pay"), ["folder:payments"]);
        assert_eq!(ids(&mut ix, "!create"), ["hist:1"]);
        assert_eq!(ids(&mut ix, ">save"), ["act:save"]);
        assert_eq!(ids(&mut ix, "#bill"), ["req:payments/create.flux.md"]);
        assert!(!ids(&mut ix, "save").contains(&"act:save".to_string()));
    }

    #[test]
    fn matches_url_and_ranks_name_first() {
        let mut ix = sample();
        assert!(ids(&mut ix, "v1/payments").contains(&"req:payments/create.flux.md".to_string()));
        assert_eq!(ids(&mut ix, "login")[0], "req:auth/login.flux.md");
    }

    #[test]
    fn highlights_label_indices() {
        let mut ix = sample();
        let hits = ix.query("login", 5);
        assert_eq!(hits[0].label_indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn recency_boosts_and_orders_empty_query() {
        let mut ix = sample();
        ix.set_recency(["env:staging".to_string()]);
        assert_eq!(ids(&mut ix, "")[0], "env:staging");
    }

    #[test]
    fn upsert_remove_replace() {
        let mut ix = sample();
        ix.remove("req:auth/login.flux.md");
        assert!(ids(&mut ix, "login").is_empty());
        ix.replace_kind(
            Kind::Environment,
            [Item::new("env:prod", Kind::Environment, "prod")],
        );
        assert_eq!(ids(&mut ix, "@"), ["env:prod"]);
        ix.upsert(Item::new("env:prod", Kind::Environment, "production"));
        assert_eq!(ids(&mut ix, "@prod"), ["env:prod"]);
    }

    #[test]
    fn sync_applies_only_the_delta() {
        let mut ix = sample();
        let mut want = ix.items().to_vec();
        assert_eq!(ix.sync(want.clone()), 0);
        want.retain(|i| i.id != "env:staging");
        want.push(Item::new("env:qa", Kind::Environment, "qa"));
        assert_eq!(ix.sync(want), 2);
        assert_eq!(ids(&mut ix, "@"), ["env:qa"]);
    }

    #[test]
    fn keystroke_budget_5000_requests() {
        let mut ix = Index::new();
        for it in generate_requests(5000) {
            ix.upsert(it);
        }
        let start = std::time::Instant::now();
        for q in ["c", "cr", "crea", "create pay", "zzz"] {
            ix.query(q, 50);
        }
        let per = start.elapsed() / 5;
        // Generous bound for debug builds; the criterion bench checks the real budget.
        assert!(per.as_millis() < 250, "{per:?}");
    }
}
