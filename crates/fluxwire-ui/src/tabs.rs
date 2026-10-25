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

    fn group_order(&self) -> u8 {
        match self {
            TabKind::Http { .. } => 0,
            TabKind::WebSocket => 1,
            TabKind::FolderRun => 2,
            TabKind::Docs => 3,
            TabKind::Environment => 4,
            TabKind::History => 5,
            TabKind::Sync => 6,
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

#[derive(Default, Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: Option<u64>,
    next_id: u64,
    closed: Vec<Tab>,
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
        self.active = Some(id);
        id
    }

    /// Single-click: reuse the existing tab for `key`, else replace the current preview tab, else
    /// append a new preview tab.
    pub fn open_preview(&mut self, key: &str, title: &str, kind: TabKind) -> u64 {
        if let Some(id) = self.tabs.iter().find(|t| t.key == key).map(|t| t.id) {
            self.active = Some(id);
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
            self.active = Some(id);
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
            self.active = Some(id);
            return id;
        }
        self.insert_new(key, title, kind, false)
    }

    pub fn make_permanent(&mut self, id: u64) {
        if let Some(t) = self.get_mut(id) {
            t.preview = false;
        }
    }

    pub fn activate(&mut self, id: u64) {
        if self.get(id).is_some() {
            self.active = Some(id);
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
        self.closed.push(tab);
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

    pub fn reopen_closed(&mut self) -> Option<u64> {
        let t = self.closed.pop()?;
        Some(self.open_permanent(&t.key, &t.title, t.kind))
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
}
