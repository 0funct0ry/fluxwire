//! The History tab's query model (SPEC §13.1): filters, chips, navigator items, row grouping and
//! column layout. Pure data with no GPUI, so every rule is unit-tested.

use std::collections::HashSet;

use fluxwire_store::{DAY_MS, Filter, Sort, SortCol, Summary};

pub const PAGE: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeRange {
    Today,
    Last24h,
    Last7d,
}

impl TimeRange {
    pub const ALL: [TimeRange; 3] = [TimeRange::Today, TimeRange::Last24h, TimeRange::Last7d];

    pub fn label(self) -> &'static str {
        match self {
            TimeRange::Today => "Today",
            TimeRange::Last24h => "24 h",
            TimeRange::Last7d => "7 d",
        }
    }

    fn since(self, now: i64, day_start: i64) -> i64 {
        match self {
            TimeRange::Today => day_start,
            TimeRange::Last24h => now - DAY_MS,
            TimeRange::Last7d => now - 7 * DAY_MS,
        }
    }
}

/// Navigator timeline buckets; they match `Store::counts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeline {
    Today,
    Yesterday,
    Week,
    Older,
}

impl Timeline {
    pub const ALL: [Timeline; 4] = [
        Timeline::Today,
        Timeline::Yesterday,
        Timeline::Week,
        Timeline::Older,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Timeline::Today => "Today",
            Timeline::Yesterday => "Yesterday",
            Timeline::Week => "This week",
            Timeline::Older => "Older",
        }
    }

    /// `[since, until)` in ms.
    pub fn bounds(self, day_start: i64) -> (Option<i64>, Option<i64>) {
        let yday = day_start - DAY_MS;
        let week = day_start - 6 * DAY_MS;
        match self {
            Timeline::Today => (Some(day_start), None),
            Timeline::Yesterday => (Some(yday), Some(day_start)),
            Timeline::Week => (Some(week), Some(yday)),
            Timeline::Older => (None, Some(week)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedView {
    Failures,
    Slow,
    Unsaved,
    Pinned,
    Runs,
}

impl SavedView {
    pub const ALL: [SavedView; 5] = [
        SavedView::Failures,
        SavedView::Slow,
        SavedView::Unsaved,
        SavedView::Pinned,
        SavedView::Runs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SavedView::Failures => "Failures",
            SavedView::Slow => "Slow",
            SavedView::Unsaved => "Sent unsaved",
            SavedView::Pinned => "Pinned",
            SavedView::Runs => "Folder runs",
        }
    }
}

/// What a navigator item selects; the active one is highlighted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavItem {
    All,
    Timeline(Timeline),
    View(SavedView),
    Collection(String),
    Request { collection: String, path: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    Method,
    Class,
    Env,
    Protocol,
    Collection,
    Unsaved,
    Pinned,
    Failed,
    Slow,
    Runs,
    Run,
    Request,
    Range,
    Timeline,
}

/// Everything the user has narrowed the table to. Filters combine with AND.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub text: String,
    pub method: Option<String>,
    pub class: Option<u8>,
    pub env: Option<String>,
    pub protocol: Option<String>,
    pub collection: Option<String>,
    pub request_path: Option<String>,
    pub run: Option<String>,
    pub unsaved: bool,
    pub pinned: bool,
    pub failed: bool,
    pub slow: bool,
    pub runs_only: bool,
    pub range: Option<TimeRange>,
    pub timeline: Option<Timeline>,
    pub nav: Option<NavItem>,
    pub sort: Sort,
}

impl Query {
    /// Whether any filter is set (sorting does not count).
    pub fn has_filters(&self) -> bool {
        let mut q = self.clone();
        q.sort = Sort::default();
        q.nav = None;
        q != Query::default()
    }

    /// Drop every filter but keep the sort.
    pub fn clear(&mut self) {
        *self = Query {
            sort: self.sort,
            ..Query::default()
        };
    }

    pub fn apply_nav(&mut self, item: NavItem) {
        self.clear();
        match &item {
            NavItem::All => {}
            NavItem::Timeline(t) => self.timeline = Some(*t),
            NavItem::View(v) => match v {
                SavedView::Failures => self.failed = true,
                SavedView::Slow => self.slow = true,
                SavedView::Unsaved => self.unsaved = true,
                SavedView::Pinned => self.pinned = true,
                SavedView::Runs => self.runs_only = true,
            },
            NavItem::Collection(c) => self.collection = Some(c.clone()),
            NavItem::Request { collection, path } => {
                self.collection = Some(collection.clone());
                self.request_path = Some(path.clone());
            }
        }
        self.nav = Some(item);
    }

    /// The store filter. `now` and `day_start` are ms; `before` pages newest-first order,
    /// `offset` pages any other order.
    pub fn to_filter(
        &self,
        now: i64,
        day_start: i64,
        before: Option<i64>,
        offset: usize,
    ) -> Filter {
        let (mut since, mut until) = self.timeline.map_or((None, None), |t| t.bounds(day_start));
        if let Some(r) = self.range {
            let s = r.since(now, day_start);
            since = Some(since.map_or(s, |x| x.max(s)));
        }
        let newest_first = self.sort.col == SortCol::Time && !self.sort.asc;
        if until.is_some_and(|u| u <= since.unwrap_or(i64::MIN)) {
            until = since;
        }
        Filter {
            method: self.method.clone(),
            status_class: self.class,
            text: Some(self.text.clone()).filter(|s| !s.is_empty()),
            request_path: self.request_path.clone(),
            run_id: self.run.clone(),
            env: self.env.clone(),
            protocol: self.protocol.clone(),
            collection: self.collection.clone(),
            pinned: self.pinned,
            dirty: self.unsaved,
            failed: self.failed,
            slow: self.slow,
            runs_only: self.runs_only,
            since_ms: since,
            until_ms: until,
            sort: self.sort,
            before_id: if newest_first { before } else { None },
            offset: if newest_first { None } else { Some(offset) },
            limit: Some(PAGE),
            ..Filter::default()
        }
    }

    /// Removable chips for the toolbar, in a stable order.
    pub fn chips(&self) -> Vec<(ChipKind, String)> {
        let mut c = Vec::new();
        let mut push = |k, s: String| c.push((k, s));
        if let Some(m) = &self.method {
            push(ChipKind::Method, m.clone());
        }
        if let Some(k) = self.class {
            push(ChipKind::Class, class_label(k).into());
        }
        if let Some(e) = &self.env {
            push(ChipKind::Env, format!("env {e}"));
        }
        if let Some(p) = &self.protocol {
            push(ChipKind::Protocol, p.clone());
        }
        if let Some(x) = &self.collection {
            push(ChipKind::Collection, x.clone());
        }
        if let Some(p) = &self.request_path {
            push(ChipKind::Request, p.clone());
        }
        if self.unsaved {
            push(ChipKind::Unsaved, "sent unsaved".into());
        }
        if self.pinned {
            push(ChipKind::Pinned, "pinned".into());
        }
        if self.failed {
            push(ChipKind::Failed, "failures".into());
        }
        if self.slow {
            push(ChipKind::Slow, "slow".into());
        }
        if self.runs_only {
            push(ChipKind::Runs, "folder runs".into());
        }
        if let Some(r) = &self.run {
            let short = r.rsplit('-').next().unwrap_or(r);
            push(ChipKind::Run, format!("run {short}"));
        }
        if let Some(r) = self.range {
            push(ChipKind::Range, r.label().into());
        }
        if let Some(t) = self.timeline {
            push(ChipKind::Timeline, t.label().into());
        }
        c
    }

    pub fn remove(&mut self, k: ChipKind) {
        match k {
            ChipKind::Method => self.method = None,
            ChipKind::Class => self.class = None,
            ChipKind::Env => self.env = None,
            ChipKind::Protocol => self.protocol = None,
            ChipKind::Collection => self.collection = None,
            ChipKind::Request => self.request_path = None,
            ChipKind::Unsaved => self.unsaved = false,
            ChipKind::Pinned => self.pinned = false,
            ChipKind::Failed => self.failed = false,
            ChipKind::Slow => self.slow = false,
            ChipKind::Runs => self.runs_only = false,
            ChipKind::Run => self.run = None,
            ChipKind::Range => self.range = None,
            ChipKind::Timeline => self.timeline = None,
        }
        self.nav = None;
    }
}

pub fn class_label(c: u8) -> &'static str {
    match c {
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "err",
    }
}

/// A row of the virtualized table.
#[derive(Clone, Debug, PartialEq)]
pub enum HistRow {
    Day(String),
    Entry(usize),
    /// A folder run collapsed into one row.
    Run {
        id: String,
        count: usize,
        failed: usize,
        expanded: bool,
        first: usize,
    },
    More,
}

/// Day headers, folder-run groups (by `run_id`), and a trailing "load more" row.
pub fn build_rows(
    entries: &[Summary],
    expanded: &HashSet<String>,
    all_loaded: bool,
    day: impl Fn(i64) -> String,
) -> Vec<HistRow> {
    let mut rows = Vec::with_capacity(entries.len() + 8);
    let mut seen: HashSet<&str> = HashSet::new();
    let mut last = String::new();
    for (i, e) in entries.iter().enumerate() {
        let d = day(e.ts_ms);
        let run = e.run_id.as_deref();
        let first_of_run = run.is_some_and(|r| seen.insert(r));
        if run.is_some() && !first_of_run && !expanded.contains(run.unwrap_or_default()) {
            continue;
        }
        if d != last {
            rows.push(HistRow::Day(d.clone()));
            last = d;
        }
        if let (true, Some(r)) = (first_of_run, run) {
            let group = entries.iter().filter(|x| x.run_id.as_deref() == Some(r));
            let failed = group
                .clone()
                .filter(|x| x.status.is_none_or(|s| s >= 400))
                .count();
            rows.push(HistRow::Run {
                id: r.to_string(),
                count: group.count(),
                failed,
                expanded: expanded.contains(r),
                first: i,
            });
            if expanded.contains(r) {
                rows.push(HistRow::Entry(i));
            }
        } else {
            rows.push(HistRow::Entry(i));
        }
    }
    if !all_loaded {
        rows.push(HistRow::More);
    }
    rows
}

pub const COLUMNS: [(&str, SortCol); 7] = [
    ("Time", SortCol::Time),
    ("Method", SortCol::Method),
    ("Name", SortCol::Name),
    ("URL", SortCol::Url),
    ("Status", SortCol::Status),
    ("Duration", SortCol::Duration),
    ("Env", SortCol::Env),
];
pub const MIN_COL: f32 = 44.0;
pub const DEFAULT_WIDTHS: [f32; 7] = [86.0, 64.0, 170.0, 280.0, 64.0, 80.0, 90.0];

/// Click on a header: same column flips direction, a new one starts descending
/// (ascending for text columns).
pub fn next_sort(cur: Sort, col: SortCol) -> Sort {
    if cur.col == col {
        Sort { col, asc: !cur.asc }
    } else {
        let text = matches!(
            col,
            SortCol::Method | SortCol::Name | SortCol::Url | SortCol::Env
        );
        Sort { col, asc: text }
    }
}

pub fn clamp_width(w: f32) -> f32 {
    w.max(MIN_COL)
}

pub fn sort_name(c: SortCol) -> &'static str {
    COLUMNS
        .iter()
        .find(|(_, k)| *k == c)
        .map_or("Time", |(n, _)| n)
}

pub fn sort_from_name(n: &str) -> SortCol {
    COLUMNS
        .iter()
        .find(|(name, _)| *name == n)
        .map_or(SortCol::Time, |(_, k)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: i64, run: Option<&str>, status: Option<i64>, ts: i64) -> Summary {
        Summary {
            id,
            ts_ms: ts,
            collection: "c".into(),
            request_path: "a".into(),
            request_name: "a".into(),
            protocol: "http".into(),
            method: "GET".into(),
            url: "u".into(),
            status,
            total_ms: 1.0,
            env: None,
            dirty: false,
            summary: String::new(),
            run_id: run.map(String::from),
            pinned: false,
            note: None,
        }
    }

    #[test]
    fn nav_items_set_exactly_their_filter() {
        let mut q = Query {
            text: "x".into(),
            method: Some("GET".into()),
            ..Default::default()
        };
        q.apply_nav(NavItem::View(SavedView::Failures));
        assert!(q.failed && q.text.is_empty() && q.method.is_none());
        assert_eq!(q.nav, Some(NavItem::View(SavedView::Failures)));
        q.apply_nav(NavItem::Request {
            collection: "c".into(),
            path: "p".into(),
        });
        assert!(!q.failed);
        let f = q.to_filter(0, 0, None, 0);
        assert_eq!(
            (f.collection.as_deref(), f.request_path.as_deref()),
            (Some("c"), Some("p"))
        );
    }

    #[test]
    fn removing_a_chip_drops_the_nav_highlight() {
        let mut q = Query::default();
        q.apply_nav(NavItem::View(SavedView::Pinned));
        assert_eq!(q.chips(), vec![(ChipKind::Pinned, "pinned".into())]);
        q.remove(ChipKind::Pinned);
        assert!(q.nav.is_none() && !q.has_filters());
    }

    #[test]
    fn timeline_and_range_intersect() {
        let day = 10 * DAY_MS;
        let mut q = Query::default();
        q.apply_nav(NavItem::Timeline(Timeline::Week));
        q.range = Some(TimeRange::Last24h);
        let f = q.to_filter(day + 100, day, None, 0);
        assert_eq!(f.since_ms, Some(day + 100 - DAY_MS));
        // the intersection is empty; it stays an empty range rather than inverting
        assert_eq!(f.until_ms, f.since_ms);
    }

    #[test]
    fn paging_uses_before_id_only_for_newest_first() {
        let mut q = Query::default();
        let f = q.to_filter(0, 0, Some(5), 40);
        assert_eq!((f.before_id, f.offset), (Some(5), None));
        q.sort = next_sort(q.sort, SortCol::Name);
        let f = q.to_filter(0, 0, Some(5), 40);
        assert_eq!((f.before_id, f.offset), (None, Some(40)));
    }

    #[test]
    fn header_clicks_toggle_and_remember_direction() {
        let s = Sort::default();
        assert!(!s.asc && s.col == SortCol::Time);
        let s = next_sort(s, SortCol::Time);
        assert!(s.asc);
        let s = next_sort(s, SortCol::Duration);
        assert!(!s.asc && s.col == SortCol::Duration);
        assert!(next_sort(s, SortCol::Name).asc);
        assert_eq!(sort_from_name(sort_name(SortCol::Env)), SortCol::Env);
    }

    #[test]
    fn runs_collapse_into_one_row_and_expand() {
        let entries = vec![
            e(5, Some("r1"), Some(200), 5),
            e(4, Some("r1"), Some(500), 4),
            e(3, None, Some(200), 3),
            e(2, Some("r1"), None, 2),
        ];
        let day = |_| "Today".to_string();
        let rows = build_rows(&entries, &HashSet::new(), true, day);
        assert_eq!(
            rows,
            vec![
                HistRow::Day("Today".into()),
                HistRow::Run {
                    id: "r1".into(),
                    count: 3,
                    failed: 2,
                    expanded: false,
                    first: 0
                },
                HistRow::Entry(2),
            ]
        );
        let open: HashSet<String> = ["r1".to_string()].into();
        let rows = build_rows(&entries, &open, false, day);
        let kinds: Vec<_> = rows
            .iter()
            .map(|r| match r {
                HistRow::Day(_) => 'd',
                HistRow::Entry(_) => 'e',
                HistRow::Run { .. } => 'r',
                HistRow::More => 'm',
            })
            .collect();
        assert_eq!(kinds, ['d', 'r', 'e', 'e', 'e', 'e', 'm']);
    }

    #[test]
    fn day_headers_follow_the_entries() {
        let entries = vec![e(2, None, Some(200), 2000), e(1, None, Some(200), 1)];
        let rows = build_rows(&entries, &HashSet::new(), true, |t| {
            format!("d{}", t / 1000)
        });
        assert_eq!(
            rows.iter().filter(|r| matches!(r, HistRow::Day(_))).count(),
            2
        );
    }
}
