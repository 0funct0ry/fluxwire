//! Tab model: preview/permanent semantics, pinning, dirty state (SPEC §17.2). Pure data, no GPUI.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    Http { method: String },
    WebSocket,
    FolderRun,
    Docs,
    Environment,
    History,
    Sync,
    Keybindings,
}

impl TabKind {
    /// Leading glyph text (SPEC §17.2). HTTP tabs show the method itself.
    pub fn glyph(&self) -> String {
        match self {
            TabKind::Http { method } => method.clone(),
            TabKind::WebSocket => "⇄".into(),
            TabKind::FolderRun => "▶".into(),
            TabKind::Docs => "¶".into(),
            TabKind::Environment => "⚙".into(),
            TabKind::History => "⟲".into(),
            TabKind::Sync => "⇅".into(),
            TabKind::Keybindings => "⌨".into(),
        }
    }

    /// Token name passed to `Theme::method_color`.
    pub fn color_key(&self) -> &str {
        match self {
            TabKind::Http { method } => method,
            TabKind::WebSocket => "WS",
            TabKind::FolderRun => "RUN",
            _ => "",
        }
    }

    pub fn group_order(&self) -> u8 {
        match self {
            TabKind::Http { .. } => 0,
            TabKind::WebSocket => 1,
            TabKind::FolderRun => 2,
            TabKind::Docs => 3,
            TabKind::Environment => 4,
            TabKind::History => 5,
            TabKind::Sync => 6,
            TabKind::Keybindings => 7,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: u64,
    /// Identity of the thing shown; opening the same key activates the existing tab.
    pub key: String,
    pub title: String,
    pub kind: TabKind,
    pub preview: bool,
    pub pinned: bool,
    pub dirty: bool,
}

/// Closed tabs remembered for reopen.
const CLOSED_LIMIT: usize = 20;

#[derive(Default, Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
    closed: Vec<Tab>,
    /// Tab ids, most recently activated first (Ctrl+Tab order).
    mru: Vec<u64>,
    pub group_by_kind: bool,
}

impl Tabs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active
    }

    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn get(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// Activate `id` and move it to the front of the recency list.
    fn set_active(&mut self, id: u64) {
        self.active = Some(id);
        self.mru.retain(|m| *m != id);
        self.mru.insert(0, id);
    }

    /// Tabs by recency, most recent first; tabs never activated follow in tab order.
    pub fn mru_order(&self) -> Vec<u64> {
        let mut out: Vec<u64> = self
            .mru
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some())
            .collect();
        for t in &self.tabs {
            if !out.contains(&t.id) {
                out.push(t.id);
            }
        }
        out
    }

    /// Show `id` without changing the recency order (Ctrl+Tab while the modifier is held).
    pub fn peek(&mut self, id: u64) {
        if self.get(id).is_some() {
            self.active = Some(id);
        }
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn insert_new(&mut self, key: &str, title: &str, kind: TabKind, preview: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            key: key.into(),
            title: title.into(),
            kind,
            preview,
            pinned: false,
            dirty: false,
        });
        self.set_active(id);
        id
    }

    /// Single-click: reuse the existing tab for `key`, else replace the current preview tab, else
    /// append a new preview tab.
    pub fn open_preview(&mut self, key: &str, title: &str, kind: TabKind) -> u64 {
        if let Some(id) = self.tabs.iter().find(|t| t.key == key).map(|t| t.id) {
            self.set_active(id);
            return id;
        }
        if let Some(pos) = self.tabs.iter().position(|t| t.preview && !t.pinned) {
            let id = self.next_id;
            self.next_id += 1;
            self.tabs[pos] = Tab {
                id,
                key: key.into(),
                title: title.into(),
                kind,
                preview: true,
                pinned: false,
                dirty: false,
            };
            self.set_active(id);
            return id;
        }
        self.insert_new(key, title, kind, true)
    }

    /// Double-click or edit: the tab becomes permanent.
    pub fn open_permanent(&mut self, key: &str, title: &str, kind: TabKind) -> u64 {
        if let Some(id) = self.tabs.iter().find(|t| t.key == key).map(|t| t.id) {
            if let Some(t) = self.get_mut(id) {
                t.preview = false;
            }
            self.set_active(id);
            return id;
        }
        self.insert_new(key, title, kind, false)
    }

    pub fn id_for_key(&self, key: &str) -> Option<u64> {
        self.tabs.iter().find(|t| t.key == key).map(|t| t.id)
    }

    /// Refresh the title and kind of the tab showing `key` (after a rename or external edit).
    pub fn update(&mut self, key: &str, title: &str, kind: TabKind) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.key == key) {
            t.title = title.into();
            t.kind = kind;
        }
    }

    /// Point the tab for `old` at `new` (rename or move on disk).
    pub fn rekey(&mut self, old: &str, new: &str) {
        for t in self.tabs.iter_mut().chain(self.closed.iter_mut()) {
            if t.key == old {
                t.key = new.into();
            } else if let Some(rest) = t.key.strip_prefix(&format!("{old}/")) {
                t.key = format!("{new}/{rest}");
            }
        }
    }

    /// Close every tab whose key is `key` or lies below the folder `key`.
    pub fn close_key(&mut self, key: &str) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.key == key || t.key.starts_with(&format!("{key}/")))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close(id);
        }
    }

    pub fn make_permanent(&mut self, id: u64) {
        if let Some(t) = self.get_mut(id) {
            t.preview = false;
        }
    }

    pub fn activate(&mut self, id: u64) {
        if self.get(id).is_some() {
            self.set_active(id);
        }
    }

    pub fn set_dirty(&mut self, id: u64, dirty: bool) {
        if let Some(t) = self.get_mut(id) {
            t.dirty = dirty;
            if dirty {
                t.preview = false;
            }
        }
    }

    /// Toggles the pin; pinned tabs sort to the front and are never replaced as previews.
    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(t) = self.get_mut(id) {
            t.pinned = !t.pinned;
            if t.pinned {
                t.preview = false;
            }
        }
        self.tabs.sort_by_key(|t| !t.pinned); // stable: keeps relative order
    }

    pub fn close(&mut self, id: u64) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let tab = self.tabs.remove(pos);
        self.mru.retain(|m| *m != id);
        self.closed.push(tab);
        if self.closed.len() > CLOSED_LIMIT {
            self.closed.remove(0);
        }
        if self.active == Some(id) {
            let next = pos.min(self.tabs.len().saturating_sub(1));
            self.active = self.tabs.get(next).map(|t| t.id);
        }
    }

    pub fn close_others(&mut self, keep: u64) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.id != keep && !t.pinned)
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close(id);
        }
    }

    /// Close every unpinned tab after `id` in tab order.
    pub fn close_to_right(&mut self, id: u64) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let ids: Vec<u64> = self.tabs[pos + 1..]
            .iter()
            .filter(|t| !t.pinned)
            .map(|t| t.id)
            .collect();
        for i in ids {
            self.close(i);
        }
    }

    /// Close every unpinned tab without unsaved edits.
    pub fn close_clean(&mut self) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| !t.dirty && !t.pinned)
            .map(|t| t.id)
            .collect();
        for i in ids {
            self.close(i);
        }
    }

    /// Close every unpinned tab.
    pub fn close_all(&mut self) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| !t.pinned)
            .map(|t| t.id)
            .collect();
        for i in ids {
            self.close(i);
        }
    }

    /// Reopens the most recently closed tab, keeping its pin.
    pub fn reopen_closed(&mut self) -> Option<u64> {
        let t = self.closed.pop()?;
        let id = self.open_permanent(&t.key, &t.title, t.kind);
        if t.pinned {
            self.toggle_pin(id);
        }
        Some(id)
    }

    /// Removes and returns the most recently closed tab without reopening it.
    pub fn pop_closed(&mut self) -> Option<Tab> {
        self.closed.pop()
    }

    pub fn closed_tabs(&self) -> &[Tab] {
        &self.closed
    }

    /// Restores the closed-tab stack from a saved session (oldest first).
    pub fn set_closed(&mut self, closed: Vec<(String, String, TabKind, bool)>) {
        self.closed = closed
            .into_iter()
            .map(|(key, title, kind, pinned)| {
                let id = self.next_id;
                self.next_id += 1;
                Tab {
                    id,
                    key,
                    title,
                    kind,
                    preview: false,
                    pinned,
                    dirty: false,
                }
            })
            .collect();
    }

    pub fn cycle(&mut self, delta: isize) {
        let n = self.tabs.len() as isize;
        if n == 0 {
            return;
        }
        let cur = self
            .active
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(n) as usize;
        self.active = Some(self.tabs[next].id);
    }

    /// Display order: as stored, or grouped by kind (stable) when `group_by_kind` is set.
    /// Move tab `id` to sit right after tab `after` (open "beside").
    pub fn place_after(&mut self, id: u64, after: u64) {
        let Some(from) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let at = self
            .tabs
            .iter()
            .position(|t| t.id == after)
            .map_or(self.tabs.len(), |i| i + 1);
        self.tabs.insert(at, tab);
    }

    pub fn display_order(&self) -> Vec<&Tab> {
        let mut v: Vec<&Tab> = self.tabs.iter().collect();
        if self.group_by_kind {
            v.sort_by_key(|t| (!t.pinned, t.kind.group_order()));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http() -> TabKind {
        TabKind::Http {
            method: "GET".into(),
        }
    }

    #[test]
    fn preview_is_replaced_by_next_preview() {
        let mut t = Tabs::new();
        t.open_preview("a", "A", http());
        t.open_preview("b", "B", http());
        assert_eq!(t.all().len(), 1);
        assert_eq!(t.active().unwrap().key, "b");
    }

    #[test]
    fn permanent_tab_survives_new_previews() {
        let mut t = Tabs::new();
        t.open_preview("a", "A", http());
        t.open_permanent("a", "A", http());
        t.open_preview("b", "B", http());
        assert_eq!(t.all().len(), 2);
        assert!(!t.all()[0].preview);
        assert!(t.all()[1].preview);
    }

    #[test]
    fn reopening_a_key_activates_it() {
        let mut t = Tabs::new();
        let a = t.open_permanent("a", "A", http());
        t.open_permanent("b", "B", http());
        t.open_preview("a", "A", http());
        assert_eq!(t.active_id(), Some(a));
        assert_eq!(t.all().len(), 2);
    }

    #[test]
    fn pinning_moves_to_front_and_blocks_replacement() {
        let mut t = Tabs::new();
        t.open_permanent("a", "A", http());
        let b = t.open_permanent("b", "B", http());
        t.toggle_pin(b);
        assert_eq!(t.all()[0].key, "b");
        t.open_preview("c", "C", http());
        t.open_preview("d", "D", http());
        let keys: Vec<_> = t.all().iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "d"]);
    }

    #[test]
    fn closing_active_activates_neighbour_and_can_reopen() {
        let mut t = Tabs::new();
        let a = t.open_permanent("a", "A", http());
        let b = t.open_permanent("b", "B", http());
        t.close(b);
        assert_eq!(t.active_id(), Some(a));
        t.close(a);
        assert_eq!(t.active_id(), None);
        t.reopen_closed();
        assert_eq!(t.active().unwrap().key, "a");
    }

    #[test]
    fn editing_makes_preview_permanent() {
        let mut t = Tabs::new();
        let a = t.open_preview("a", "A", http());
        t.set_dirty(a, true);
        assert!(!t.get(a).unwrap().preview);
    }

    #[test]
    fn grouping_orders_by_kind() {
        let mut t = Tabs::new();
        t.open_permanent("ws", "S", TabKind::WebSocket);
        t.open_permanent("a", "A", http());
        t.group_by_kind = true;
        let keys: Vec<_> = t.display_order().iter().map(|x| x.key.clone()).collect();
        assert_eq!(keys, ["a", "ws"]);
    }

    #[test]
    fn mru_order_follows_activation_and_peek_does_not_reorder() {
        let mut t = Tabs::new();
        let a = t.open_permanent("a", "a", TabKind::Docs);
        let b = t.open_permanent("b", "b", TabKind::Docs);
        let c = t.open_permanent("c", "c", TabKind::Docs);
        assert_eq!(t.mru_order(), [c, b, a]);
        t.activate(a);
        assert_eq!(t.mru_order(), [a, c, b]);
        t.peek(b);
        assert_eq!(t.active_id(), Some(b));
        assert_eq!(t.mru_order(), [a, c, b]);
    }

    #[test]
    fn close_variants_respect_pins_and_dirty() {
        let mut t = Tabs::new();
        let a = t.open_permanent("a", "a", TabKind::Docs);
        let b = t.open_permanent("b", "b", TabKind::Docs);
        let c = t.open_permanent("c", "c", TabKind::Docs);
        let d = t.open_permanent("d", "d", TabKind::Docs);
        t.toggle_pin(d); // sorts to front: d a b c
        t.set_dirty(c, true);
        t.close_to_right(a);
        assert_eq!(t.all().iter().map(|x| x.id).collect::<Vec<_>>(), [d, a]);
        let e = t.open_permanent("e", "e", TabKind::Docs);
        t.set_dirty(e, true);
        t.close_clean();
        assert_eq!(t.all().iter().map(|x| x.id).collect::<Vec<_>>(), [d, e]);
        t.close_all();
        assert_eq!(t.all().iter().map(|x| x.id).collect::<Vec<_>>(), [d]);
        let _ = b;
    }

    #[test]
    fn closed_stack_is_bounded_and_reopen_keeps_pin() {
        let mut t = Tabs::new();
        for i in 0..30 {
            let id = t.open_permanent(&format!("k{i}"), "t", TabKind::Docs);
            if i == 29 {
                t.toggle_pin(id);
            }
            t.close(id);
        }
        assert_eq!(t.closed_tabs().len(), 20);
        let id = t.reopen_closed().unwrap();
        assert!(t.get(id).unwrap().pinned);
    }
}
