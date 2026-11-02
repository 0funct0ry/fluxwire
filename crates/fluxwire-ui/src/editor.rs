//! Read-only highlighted code view and token colours. Every editable code surface is a
//! `CodeEditor` (`code_editor.rs`, ADR-016); this module keeps the static view used by the
//! response and history viewers.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, Bounds, Div, Hsla, IntoElement, ParentElement, Pixels, Point, Stateful, Styled,
    canvas, div, prelude::*, px,
};

use crate::{
    assets::BUFFER_FONT,
    highlight::{Spans, Tok, highlight, line_runs},
    theme::Theme,
};

pub use crate::highlight::Lang;
use crate::highlight::Lang as L;

pub const LINE_HEIGHT: f32 = 20.0;
pub const GUTTER_WIDTH: f32 = 44.0;
pub const TEXT_LEFT: f32 = GUTTER_WIDTH + 4.0;
/// Advance of one column of the bundled monospace font at 13 px.
pub const CHAR_WIDTH: f32 = 7.8;

pub fn line_start(s: &str, i: usize) -> usize {
    s[..i].rfind('\n').map_or(0, |p| p + 1)
}

pub fn line_end(s: &str, i: usize) -> usize {
    s[i..].find('\n').map_or(s.len(), |p| i + p)
}

/// (line, column in chars) of a byte offset.
pub fn line_col(s: &str, i: usize) -> (usize, usize) {
    let i = i.min(s.len());
    let ls = line_start(s, i);
    (s[..i].matches('\n').count(), s[ls..i].chars().count())
}

/// Byte offset of (line, column in chars), clamped to the line.
pub fn offset_of(s: &str, line: usize, col: usize) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match s[start..].find('\n') {
            Some(p) => start += p + 1,
            None => return s.len(),
        }
    }
    let end = line_end(s, start);
    s[start..end]
        .char_indices()
        .nth(col)
        .map_or(end, |(p, _)| start + p)
}

pub(crate) fn tok_color(t: &Theme, tok: Tok) -> Hsla {
    match tok {
        Tok::Plain => t.text,
        Tok::Property => t.syntax_property,
        Tok::String => t.syntax_string,
        Tok::Number => t.syntax_number,
        Tok::Boolean => t.syntax_boolean,
        Tok::Punct => t.syntax_punctuation,
        Tok::Keyword => t.syntax_keyword,
        Tok::Function => t.syntax_function,
        Tok::Type => t.syntax_type,
        Tok::Comment => t.syntax_comment,
        Tok::Heading => t.syntax_function,
        Tok::Variable => t.accent,
    }
}

fn caret(t: &Theme) -> Div {
    div()
        .w(px(1.5))
        .h(px(16.))
        .mx(px(-0.75))
        .flex_none()
        .bg(t.accent)
}

fn line_element(
    t: &Theme,
    number: usize,
    runs: Vec<(String, Tok)>,
    caret_col: Option<usize>,
    sel: Option<(usize, usize)>,
    highlight_line: bool,
) -> Div {
    let mut children: Vec<AnyElement> = Vec::new();
    let mut col = 0usize;
    let mut placed = false;
    for (s, tok) in runs {
        let n = s.chars().count();
        let mut cuts: Vec<usize> = caret_col
            .into_iter()
            .chain(sel.into_iter().flat_map(|(a, b)| [a, b]))
            .filter(|c| *c > col && *c < col + n)
            .map(|c| c - col)
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut bounds = vec![0];
        bounds.extend(cuts);
        bounds.push(n);
        for w in bounds.windows(2) {
            let (a, b) = (w[0], w[1]);
            if !placed && caret_col == Some(col + a) {
                children.push(caret(t).into_any_element());
                placed = true;
            }
            let piece: String = s.chars().skip(a).take(b - a).collect();
            if piece.is_empty() {
                continue;
            }
            let selected = sel.is_some_and(|(x, y)| col + a >= x && col + a < y);
            children.push(
                div()
                    .text_color(tok_color(t, tok))
                    .when(selected, |d| d.bg(t.accent_bg()))
                    .child(piece)
                    .into_any_element(),
            );
        }
        col += n;
    }
    if caret_col.is_some() && !placed {
        children.push(caret(t).into_any_element());
    }
    div()
        .flex()
        .flex_row()
        .h(px(LINE_HEIGHT))
        .whitespace_nowrap()
        .when(highlight_line, |d| d.bg(t.element_hover))
        .child(
            div()
                .w(px(GUTTER_WIDTH))
                .pr(px(14.))
                .flex_none()
                .text_right()
                .text_color(t.gutter)
                .child(number.to_string()),
        )
        .children(children)
}

/// Line-numbered, highlighted view of `text`. With `cursor`, draws a caret (and the `selection`
/// range, when non-empty) and records the content element's bounds into `bounds_cell` so clicks
/// can be mapped back to offsets.
fn editor_content(
    text: &str,
    spans: &Spans,
    cursor: Option<usize>,
    selection: (usize, usize),
    bounds_cell: Option<Rc<Cell<Bounds<Pixels>>>>,
    t: &Theme,
) -> Div {
    let runs = line_runs(text, spans);
    let caret_pos = cursor.map(|c| line_col(text, c));
    let sel = cursor
        .filter(|_| selection.0 < selection.1)
        .map(|_| (line_col(text, selection.0), line_col(text, selection.1)));
    let mut el = div()
        .relative()
        .flex()
        .flex_col()
        .py_2()
        .min_h(px(60.))
        .font_family(BUFFER_FONT)
        .text_size(px(13.))
        .children(runs.into_iter().enumerate().map(|(i, r)| {
            let c = caret_pos.filter(|(l, _)| *l == i).map(|(_, c)| c);
            let line_len = r.iter().map(|(s, _)| s.chars().count()).sum::<usize>();
            let s = sel.and_then(|((l0, c0), (l1, c1))| {
                (i >= l0 && i <= l1).then_some({
                    (
                        if i == l0 { c0 } else { 0 },
                        if i == l1 { c1 } else { line_len },
                    )
                })
            });
            line_element(t, i + 1, r, c, s, false)
        }));
    if let Some(cell) = bounds_cell {
        el = el.child(
            canvas(
                move |b, _, _| {
                    // Content origin sits below the container's vertical padding.
                    cell.set(Bounds {
                        origin: Point {
                            x: b.origin.x,
                            y: b.origin.y + px(8.),
                        },
                        size: b.size,
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        );
    }
    el
}

/// Read-only code surface.
pub fn code_view(text: &str, lang: L, t: &Theme) -> AnyElement {
    let spans = highlight(text, lang);
    editor_content(text, &spans, None, (0, 0), None, t).into_any_element()
}

pub fn scroll_container(id: &'static str) -> Stateful<Div> {
    div().id(id).flex_1().min_h_0().overflow_y_scroll()
}
