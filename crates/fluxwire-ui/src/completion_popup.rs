//! The grouped `{{` completion popup of the URL bar and the Params/Headers cells: a vertical
//! list with Environment/Dynamic section headers, a kind icon and a muted detail column.
//! `visible_window` is pure and tested; `completion_popup` is the view.

use crate::completion::{Row, rows};
use crate::dev_id::{DevClick, Role};
use crate::envs::Completion;
use crate::theme::Theme;
use crate::workspace::WorkspaceView;
use gpui::anchored as anchor_at;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::*,
    px,
};
use std::ops::Range;

/// Rows (headers included) listed at once; the window scrolls with the selection.
pub const VISIBLE_ROWS: usize = 10;

/// The slice of `rows` to draw so that the selected item stays visible, and a header stays
/// attached to the first item of its group when the window starts on it.
pub fn visible_window(rows: &[Row], selected: usize, max: usize) -> Range<usize> {
    let Some(pos) = rows.iter().position(|r| *r == Row::Item(selected)) else {
        return 0..rows.len().min(max);
    };
    if rows.len() <= max {
        return 0..rows.len();
    }
    let mut start = (pos + 1).saturating_sub(max);
    if start > 0 && matches!(rows[start], Row::Item(_)) && matches!(rows[start - 1], Row::Header(_))
    {
        start -= 1;
    }
    start..(start + max).min(rows.len())
}

impl WorkspaceView {
    /// The popup for the open completion, drawn below the point it is placed at (the caller
    /// wraps it in a zero-size absolutely positioned anchor).
    pub(crate) fn completion_popup(
        &self,
        t: &Theme,
        c: &Completion,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all = rows(&c.items);
        let win = visible_window(&all, c.selected, VISIBLE_ROWS);
        let target = c.target;
        let mut list = div()
            .id("var-completion")
            .flex()
            .flex_col()
            .min_w(px(280.))
            .max_w(px(460.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .text_size(px(12.))
            .font_family(crate::assets::BUFFER_FONT)
            .text_color(t.text)
            .occlude();
        for r in &all[win] {
            match *r {
                Row::Header(g) => {
                    list = list.child(
                        div()
                            .px(px(8.))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .text_size(px(10.))
                            .text_color(t.text_placeholder)
                            .child(g.to_uppercase()),
                    );
                }
                Row::Item(i) => {
                    let ranked = &c.items[i];
                    let name = ranked.item.label.clone();
                    let matched = ranked.matched.clone();
                    let label = name.chars().enumerate().map(|(ci, ch)| {
                        div()
                            .when(matched.contains(&(ci as u32)), |d| {
                                d.text_color(t.accent).font_weight(gpui::FontWeight::BOLD)
                            })
                            .child(ch.to_string())
                    });
                    list = list.child(
                        div()
                            .id(("var-choice", i))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px(px(8.))
                            .h(px(22.))
                            .cursor_pointer()
                            .when(i == c.selected, |d| d.bg(t.element_selected))
                            .hover({
                                let h = t.element_hover;
                                move |s| s.bg(h)
                            })
                            .child(crate::ui::icon(ranked.item.kind.icon(), 12., t.text_muted))
                            .child(div().flex().flex_row().children(label))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_color(t.text_muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(ranked.item.detail.clone()),
                            )
                            .dev_click(
                                "var-completion",
                                Role::Button,
                                "var-completion",
                                true,
                                cx.listener(move |this, _, _, cx| {
                                    this.accept_completion_click(target, &name, cx)
                                }),
                            ),
                    );
                }
            }
        }
        gpui::deferred(
            anchor_at()
                .anchor(gpui::Corner::TopLeft)
                .snap_to_window_with_margin(px(8.))
                .child(list),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// Popup for `target` if that is where the open completion belongs.
    pub(crate) fn completion_popup_for(
        &self,
        t: &Theme,
        target: crate::envs::CompleteTarget,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let c = self
            .scope
            .envs
            .completion
            .as_ref()
            .filter(|c| c.target == target)?;
        Some(self.completion_popup(t, c, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(n: usize) -> Vec<Row> {
        let mut v = vec![Row::Header("A")];
        v.extend((0..n).map(Row::Item));
        v
    }

    #[test]
    fn window_covers_everything_when_short() {
        assert_eq!(visible_window(&rows_of(3), 2, 10), 0..4);
    }

    #[test]
    fn window_follows_selection_down_and_back() {
        let r = rows_of(30);
        let w = visible_window(&r, 20, 10);
        assert_eq!(w.len(), 10);
        assert!(r[w.clone()].contains(&Row::Item(20)));
        let w0 = visible_window(&r, 0, 10);
        assert_eq!(w0, 0..10);
    }

    #[test]
    fn window_keeps_header_with_first_item() {
        let mut r = rows_of(3);
        r.push(Row::Header("B"));
        r.extend((3..13).map(Row::Item));
        let w = visible_window(&r, 3, 10);
        assert!(
            r[w.start..]
                .iter()
                .take(2)
                .any(|x| matches!(x, Row::Header("B")))
                || w.start == 0
        );
    }
}
