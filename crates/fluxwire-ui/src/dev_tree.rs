//! The semantic UI tree (`dev-mcp`, ADR-018): a pure model of the tagged elements of one frame.
//! Views tag elements through [`crate::dev_id`]; the per-frame records land in a [`Frame`], and
//! this module turns them into a nested [`Node`] tree with filtering, search and click targets.
//! No GPUI types here, so everything is tested without a window.

use serde::{Deserialize, Serialize};

/// What an element is, in the vocabulary an agent can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Button,
    Tab,
    Input,
    List,
    Row,
    Modal,
    Text,
    Dock,
    Toolbar,
    StatusBar,
    Menu,
    Editor,
    Checkbox,
    Panel,
}

/// A rectangle in window coordinates (device-independent pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0. || self.h <= 0.
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect::new(x0, y0, (x1 - x0).max(0.), (y1 - y0).max(0.))
    }

    pub fn contains(&self, (px, py): (f32, f32)) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Optional facts about an element that the audit (M9.7) needs and bounds alone cannot give.
/// Every field defaults to "unknown or no"; views fill in what they know through
/// `dev_tag_meta`/`dev_click_meta`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// The text is shown truncated (ellipsis) or cut off by its container.
    #[serde(default, skip_serializing_if = "is_false")]
    pub overflow: bool,
    /// A tooltip shows the full text or purpose on hover.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tooltip: bool,
    /// Reachable with the keyboard (has a focus handle and tab stop); `None` when the view has
    /// not said, which the audit does not report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focusable: Option<bool>,
    /// Draws a visible focus indicator while focused; `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_visible: Option<bool>,
    /// Theme token names of the text and the surface behind it, e.g. `text_muted` on `surface`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One tagged element as recorded during a frame's prepaint.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// The semantic key; made unique per frame by [`Frame::push`].
    pub id: String,
    pub role: Role,
    pub label: String,
    pub enabled: bool,
    pub bounds: Rect,
    /// The ancestor clip in effect (the window when nothing clips).
    pub clip: Rect,
    pub focused: bool,
    /// Index of the parent record in the same frame.
    pub parent: Option<usize>,
    pub meta: Meta,
}

/// The records of one frame, in prepaint (paint) order, which is also the z-order.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub records: Vec<Record>,
}

impl Frame {
    /// Adds a record and returns its index. A repeated key gets `#2`, `#3`… by order of
    /// occurrence, so ids depend on the view's content, never on pointers or loop counters.
    pub fn push(&mut self, mut r: Record) -> usize {
        let base = r.id.clone();
        let mut n = 1;
        while self.records.iter().any(|o| o.id == r.id) {
            n += 1;
            r.id = format!("{base}#{n}");
        }
        self.records.push(r);
        self.records.len() - 1
    }

    pub fn tree(&self) -> Vec<Node> {
        let mut children: Vec<Vec<usize>> = vec![vec![]; self.records.len()];
        let mut roots = vec![];
        for (i, r) in self.records.iter().enumerate() {
            match r.parent {
                Some(p) if p < i => children[p].push(i),
                _ => roots.push(i),
            }
        }
        roots.into_iter().map(|i| self.node(i, &children)).collect()
    }

    fn node(&self, i: usize, children: &[Vec<usize>]) -> Node {
        let r = &self.records[i];
        let visible_area = r.bounds.intersect(&r.clip);
        Node {
            id: r.id.clone(),
            role: r.role,
            label: r.label.clone(),
            enabled: r.enabled,
            bounds: r.bounds,
            visible: !visible_area.is_empty(),
            clipped: !r.bounds.is_empty() && visible_area != r.bounds,
            focused: r.focused,
            meta: r.meta.clone(),
            z: i as u32,
            children: children[i]
                .iter()
                .map(|&c| self.node(c, children))
                .collect(),
        }
    }

    /// The point a click on `id` lands on: the centre of its visible area, or why there is none.
    pub fn click_point(&self, id: &str) -> Result<(f32, f32), String> {
        let (i, r) = self
            .records
            .iter()
            .enumerate()
            .find(|(_, r)| r.id == id)
            .ok_or_else(|| format!("no element with id {id:?}"))?;
        if !r.enabled {
            return Err(format!("element {id:?} is disabled"));
        }
        let area = r.bounds.intersect(&r.clip);
        if area.is_empty() {
            return Err(format!("element {id:?} is not visible"));
        }
        let p = area.center();
        // Anything painted later whose visible area covers the point takes the click.
        if let Some(over) = self.records[i + 1..].iter().find(|o| {
            o.parent != Some(i)
                && !self.is_descendant(o, i)
                && o.bounds.intersect(&o.clip).contains(p)
                && o.role == Role::Modal
        }) {
            return Err(format!("element {id:?} is covered by {:?}", over.id));
        }
        Ok(p)
    }

    fn is_descendant(&self, r: &Record, ancestor: usize) -> bool {
        let mut p = r.parent;
        while let Some(i) = p {
            if i == ancestor {
                return true;
            }
            p = self.records[i].parent;
        }
        false
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub role: Role,
    pub label: String,
    pub enabled: bool,
    pub bounds: Rect,
    pub visible: bool,
    /// Partly hidden by an ancestor's clip.
    pub clipped: bool,
    pub focused: bool,
    #[serde(default)]
    pub meta: Meta,
    /// Paint order; higher is in front.
    pub z: u32,
    pub children: Vec<Node>,
}

/// Which parts of the tree `ui.tree` returns.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TreeFilter {
    /// Only the subtree rooted at this id.
    pub subtree: Option<String>,
    pub role: Option<Role>,
    pub visible_only: bool,
}

impl Node {
    fn find_id(&self, id: &str) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find_id(id))
    }

    fn pruned(&self, f: &TreeFilter) -> Option<Node> {
        if f.visible_only && !self.visible {
            return None;
        }
        let kids: Vec<Node> = self.children.iter().filter_map(|c| c.pruned(f)).collect();
        let me = f.role.is_none_or(|r| r == self.role);
        if !me && kids.is_empty() {
            return None;
        }
        Some(Node {
            children: kids,
            ..self.clone()
        })
    }
}

/// Applies `f` to a whole tree.
pub fn filter(roots: &[Node], f: &TreeFilter) -> Result<Vec<Node>, String> {
    let scope: Vec<&Node> = match &f.subtree {
        Some(id) => vec![
            roots
                .iter()
                .find_map(|n| n.find_id(id))
                .ok_or_else(|| format!("no element with id {id:?}"))?,
        ],
        None => roots.iter().collect(),
    };
    Ok(scope.into_iter().filter_map(|n| n.pruned(f)).collect())
}

/// Flat match for `ui.find`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub id: String,
    pub role: Role,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub bounds: Rect,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FindQuery {
    pub label: Option<String>,
    pub role: Option<Role>,
    /// Matches the label or the id.
    pub text: Option<String>,
}

pub fn find(roots: &[Node], q: &FindQuery) -> Vec<Found> {
    fn walk(n: &Node, q: &FindQuery, out: &mut Vec<Found>) {
        let has = |hay: &str, needle: &str| hay.to_lowercase().contains(&needle.to_lowercase());
        let ok = q.label.as_ref().is_none_or(|l| has(&n.label, l))
            && q.role.is_none_or(|r| r == n.role)
            && q.text
                .as_ref()
                .is_none_or(|t| has(&n.label, t) || has(&n.id, t));
        if ok {
            out.push(Found {
                id: n.id.clone(),
                role: n.role,
                label: n.label.clone(),
                enabled: n.enabled,
                visible: n.visible,
                bounds: n.bounds,
            });
        }
        for c in &n.children {
            walk(c, q, out);
        }
    }
    let mut out = vec![];
    for n in roots {
        walk(n, q, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, role: Role, b: Rect, parent: Option<usize>) -> Record {
        Record {
            id: id.into(),
            role,
            label: id.into(),
            enabled: true,
            bounds: b,
            clip: Rect::new(0., 0., 1000., 800.),
            focused: false,
            parent,
            meta: Meta::default(),
        }
    }

    fn sample() -> Frame {
        let mut f = Frame::default();
        let dock = f.push(rec(
            "dock.left",
            Role::Dock,
            Rect::new(0., 0., 200., 800.),
            None,
        ));
        f.push(rec(
            "tab:a",
            Role::Tab,
            Rect::new(0., 0., 100., 30.),
            Some(dock),
        ));
        f.push(rec(
            "tab:a",
            Role::Tab,
            Rect::new(100., 0., 100., 30.),
            Some(dock),
        ));
        f.push(rec(
            "send",
            Role::Button,
            Rect::new(900., 10., 60., 20.),
            None,
        ));
        f
    }

    #[test]
    fn duplicate_keys_get_occurrence_suffixes() {
        let ids: Vec<_> = sample().records.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids, ["dock.left", "tab:a", "tab:a#2", "send"]);
    }

    #[test]
    fn tree_nests_and_orders_z() {
        let t = sample().tree();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].children.len(), 2);
        assert!(t[1].z > t[0].children[1].z);
    }

    #[test]
    fn tree_is_deterministic() {
        assert_eq!(sample().tree(), sample().tree());
    }

    #[test]
    fn clipping_and_visibility() {
        let mut f = Frame::default();
        let mut r = rec("half", Role::Row, Rect::new(0., 90., 50., 20.), None);
        r.clip = Rect::new(0., 0., 100., 100.);
        f.push(r);
        let mut r = rec("gone", Role::Row, Rect::new(0., 120., 50., 20.), None);
        r.clip = Rect::new(0., 0., 100., 100.);
        f.push(r);
        let t = f.tree();
        assert!(t[0].visible && t[0].clipped);
        assert!(!t[1].visible);
        assert_eq!(f.click_point("half"), Ok((25., 95.)));
        assert!(f.click_point("gone").unwrap_err().contains("not visible"));
    }

    #[test]
    fn filters() {
        let t = sample().tree();
        let sub = filter(
            &t,
            &TreeFilter {
                subtree: Some("dock.left".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(sub[0].children.len(), 2);
        let tabs = filter(
            &t,
            &TreeFilter {
                role: Some(Role::Tab),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].children.len(), 2);
        assert!(
            filter(
                &t,
                &TreeFilter {
                    subtree: Some("nope".into()),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn find_by_label_role_and_text() {
        let t = sample().tree();
        let q = FindQuery {
            label: Some("SEND".into()),
            ..Default::default()
        };
        assert_eq!(find(&t, &q)[0].id, "send");
        let q = FindQuery {
            role: Some(Role::Tab),
            ..Default::default()
        };
        assert_eq!(find(&t, &q).len(), 2);
        let q = FindQuery {
            text: Some("dock".into()),
            ..Default::default()
        };
        assert_eq!(find(&t, &q).len(), 1);
    }

    #[test]
    fn disabled_and_covered_elements_refuse_clicks() {
        let mut f = Frame::default();
        let mut d = rec("off", Role::Button, Rect::new(0., 0., 10., 10.), None);
        d.enabled = false;
        f.push(d);
        f.push(rec(
            "under",
            Role::Button,
            Rect::new(20., 20., 10., 10.),
            None,
        ));
        f.push(rec("dlg", Role::Modal, Rect::new(0., 0., 100., 100.), None));
        assert!(f.click_point("off").unwrap_err().contains("disabled"));
        assert!(f.click_point("under").unwrap_err().contains("covered"));
        assert!(f.click_point("dlg").is_ok());
        assert!(f.click_point("zzz").is_err());
    }
}
