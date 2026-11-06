//! Titles of the script cards in the Scripts tab (SPEC §17.3). Pure logic, no GPUI.

/// Longest default title taken from a comment, in characters.
pub const MAX_TITLE_CHARS: usize = 48;

/// The first line of a script when it is a `//` or `/* */` comment, trimmed and truncated.
pub fn comment_title(content: &str) -> Option<String> {
    let line = content.lines().find(|l| !l.trim().is_empty())?.trim();
    let body = if let Some(rest) = line.strip_prefix("//") {
        rest
    } else if let Some(rest) = line.strip_prefix("/*") {
        rest.trim_end()
            .trim_end_matches("*/")
            .trim_start_matches('*')
    } else {
        return None;
    };
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let mut out: String = body.chars().take(MAX_TITLE_CHARS).collect();
    if body.chars().count() > MAX_TITLE_CHARS {
        out.push('…');
    }
    Some(out)
}

/// What a card shows: the block's `title`, else its leading comment, else `Pre script 1`.
/// The flag is true when the text is a fallback rather than a title the user wrote.
pub fn card_title(title: Option<&str>, content: &str, post: bool, order: usize) -> (String, bool) {
    if let Some(t) = title {
        return (t.to_string(), false);
    }
    if let Some(c) = comment_title(content) {
        return (c, true);
    }
    let kind = if post { "Post" } else { "Pre" };
    (format!("{kind} script {order}"), true)
}

/// Height of a card's header row, in pixels (for the resize maths).
pub const HEADER_PX: f32 = 34.0;
/// Height of the gap or drag handle between two cards.
pub const HANDLE_PX: f32 = 8.0;
/// Smallest height of an expanded card.
pub const MIN_CARD_PX: f32 = 96.0;

/// How the cards of one request's Pre or Post scripts share the pane: a relative height weight
/// per card (equal by default) and which cards are collapsed. Kept in memory only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardLayout {
    pub weights: Vec<f32>,
    pub collapsed: Vec<bool>,
}

impl CardLayout {
    /// Make the layout describe `n` cards: new cards are expanded with weight 1, extra ones go.
    pub fn fit(&mut self, n: usize) {
        self.weights.resize(n, 1.0);
        self.collapsed.resize(n, false);
    }

    pub fn remove(&mut self, pos: usize) {
        if pos < self.weights.len() {
            self.weights.remove(pos);
            self.collapsed.remove(pos);
        }
    }

    pub fn toggle(&mut self, pos: usize) {
        if let Some(c) = self.collapsed.get_mut(pos) {
            *c = !*c;
        }
    }

    pub fn is_collapsed(&self, pos: usize) -> bool {
        self.collapsed.get(pos).copied().unwrap_or(false)
    }

    /// The next expanded card after `pos`, which the handle below `pos` resizes against.
    pub fn next_expanded(&self, pos: usize) -> Option<usize> {
        (pos + 1..self.weights.len()).find(|&j| !self.collapsed[j])
    }

    /// Pixels per unit of weight for a pane `height` px tall.
    pub fn px_per_weight(&self, height: f32) -> f32 {
        let n = self.weights.len();
        let collapsed = self.collapsed.iter().filter(|c| **c).count();
        let sum: f32 = (0..n)
            .filter(|&i| !self.collapsed[i])
            .map(|i| self.weights[i])
            .sum();
        let fixed = collapsed as f32 * HEADER_PX + n.saturating_sub(1) as f32 * HANDLE_PX;
        ((height - fixed).max(1.0)) / sum.max(f32::EPSILON)
    }
}

/// New weights of the two cards around a handle after it moved by `dy` px, keeping both at
/// least `MIN_CARD_PX` tall and their combined weight unchanged.
pub fn drag_weights(wi: f32, wj: f32, dy: f32, px_per_weight: f32) -> (f32, f32) {
    let total = wi + wj;
    let min_w = (MIN_CARD_PX / px_per_weight).min(total / 2.0);
    let ni = (wi + dy / px_per_weight).clamp(min_w, total - min_w);
    (ni, total - ni)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_titles() {
        assert_eq!(
            comment_title("// Store auth\nx()"),
            Some("Store auth".into())
        );
        assert_eq!(
            comment_title("\n  /* Sign it */\nx()"),
            Some("Sign it".into())
        );
        assert_eq!(comment_title("/** Doc */"), Some("Doc".into()));
        assert_eq!(comment_title("x(); // later"), None);
        assert_eq!(comment_title("//\nx()"), None);
        assert_eq!(comment_title(""), None);
        let long = format!("// {}", "a".repeat(80));
        let t = comment_title(&long).unwrap();
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn title_wins_then_comment_then_numbered() {
        assert_eq!(card_title(Some("T"), "// c", false, 1), ("T".into(), false));
        assert_eq!(card_title(None, "// c", false, 1), ("c".into(), true));
        assert_eq!(
            card_title(None, "x()", true, 2),
            ("Post script 2".into(), true)
        );
    }

    #[test]
    fn layout_fits_toggles_and_removes() {
        let mut l = CardLayout::default();
        l.fit(3);
        assert_eq!(l.weights, [1.0, 1.0, 1.0]);
        l.toggle(1);
        assert!(l.is_collapsed(1) && !l.is_collapsed(0));
        assert_eq!(l.next_expanded(0), Some(2));
        l.remove(1);
        assert_eq!(l.weights.len(), 2);
        assert!(!l.is_collapsed(1));
        l.fit(3);
        assert_eq!(l.weights.len(), 3);
    }

    #[test]
    fn px_per_weight_discounts_headers_and_handles() {
        let mut l = CardLayout::default();
        l.fit(2);
        assert_eq!(l.px_per_weight(508.0), 250.0); // 508 - 8 handle, two weights
        l.toggle(1);
        assert_eq!(l.px_per_weight(508.0), 508.0 - HANDLE_PX - HEADER_PX);
    }

    #[test]
    fn drag_moves_height_between_neighbours_within_limits() {
        let (a, b) = drag_weights(1.0, 1.0, 50.0, 250.0);
        assert!((a - 1.2).abs() < 1e-5 && (b - 0.8).abs() < 1e-5);
        // cannot shrink a card below the minimum height
        let (a, b) = drag_weights(1.0, 1.0, 1000.0, 250.0);
        assert!((b * 250.0 - MIN_CARD_PX).abs() < 0.01);
        assert!((a + b - 2.0).abs() < 1e-6);
        let (a, _) = drag_weights(1.0, 1.0, -1000.0, 250.0);
        assert!((a * 250.0 - MIN_CARD_PX).abs() < 0.01);
    }
}
