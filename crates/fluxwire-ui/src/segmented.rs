//! A segmented control: one of N labelled options (New request type, initial body type).
//! `step` is the pure selection logic; `segmented` renders the control.

use crate::dev_id::{DevClick, Role};
use gpui::{
    App, InteractiveElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px,
};

use crate::{theme::Theme, ui::row};

/// The next index when moving `dir` (-1 or 1) from `selected`, clamped to `0..len`.
pub fn step(selected: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (selected as i64 + dir as i64).clamp(0, len as i64 - 1) as usize
}

/// Render `items` with `selected` highlighted; `on_select` gets the clicked index.
pub fn segmented(
    t: &Theme,
    id: &'static str,
    items: Vec<String>,
    selected: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> gpui::Div {
    let last = items.len().saturating_sub(1);
    let mut bar = row()
        .rounded(px(4.))
        .border_1()
        .border_color(t.border)
        .overflow_hidden();
    for (i, label) in items.into_iter().enumerate() {
        let on = i == selected;
        let f = on_select.clone();
        bar = bar.child(
            div()
                .id((id, i))
                .px_3()
                .h(px(26.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(12.))
                .when_else(
                    on,
                    |d| d.bg(t.accent_bg()).text_color(t.accent),
                    |d| d.text_color(t.text_muted).hover(|s| s.bg(t.element_hover)),
                )
                .when(i != last, |d| d.border_r_1().border_color(t.border))
                .child(SharedString::from(label))
                .dev_click(
                    "segmented.segmented",
                    Role::Button,
                    "segmented.segmented",
                    true,
                    move |_, w, cx| f(i, w, cx),
                ),
        );
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_clamps_at_both_ends() {
        assert_eq!(step(0, 3, -1), 0);
        assert_eq!(step(0, 3, 1), 1);
        assert_eq!(step(2, 3, 1), 2);
        assert_eq!(step(1, 3, -1), 0);
        assert_eq!(step(0, 0, 1), 0);
    }
}
