//! Paints a `CodeEditor`: only the rows in view are shaped (M9.06). Monospace: a column is
//! `char_w` wide, so clicks and the caret are computed from the `DisplayMap`, not from shaped
//! text. Colours come from the active `Theme`.

use crate::code_editor::CodeEditor;
use crate::editor::tok_color;
use crate::highlight::{Spans, Tok};
use crate::theme::Theme;
use fluxwire_format::{SoftWrap, Whitespace};
use gpui::{
    App, Bounds, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseMoveEvent, PaintQuad, Pixels,
    ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window, fill, point, px, relative,
    size,
};
use std::ops::Range;

pub struct CodeEditorElement {
    pub editor: Entity<CodeEditor>,
}

impl IntoElement for CodeEditorElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// A style run: `(len, token, marked, read-only, unresolved variable)`.
pub(crate) type Run = (usize, Tok, bool, bool, bool);

/// Row text with tabs expanded to spaces and style runs `(len, token, marked, read-only,
/// unresolved variable)` over it. `vars` are `{{name}}` ranges and whether each resolves.
pub(crate) fn expand_row(
    src: &str,
    row: Range<usize>,
    spans: &Spans,
    tab: usize,
    marked: Option<&Range<usize>>,
    read_only: &[Range<usize>],
    vars: &[(usize, usize, bool)],
) -> (String, Vec<Run>) {
    let mut out = String::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    let mut si = spans.partition_point(|(_, e, _)| *e <= row.start);
    for (o, c) in src[row.clone()].char_indices() {
        let o = row.start + o;
        while si < spans.len() && spans[si].1 <= o {
            si += 1;
        }
        let var = vars.iter().find(|(s, e, _)| *s <= o && o < *e);
        let tok = if var.is_some() {
            Tok::Variable
        } else {
            spans
                .get(si)
                .filter(|(s, e, _)| *s <= o && o < *e)
                .map_or(Tok::Plain, |s| s.2)
        };
        let bad = var.is_some_and(|v| !v.2);
        let m = marked.is_some_and(|m| m.contains(&o));
        let ro = read_only.iter().any(|r| r.contains(&o));
        let w = if c == '\t' { tab - col % tab } else { 1 };
        let piece: String = if c == '\t' {
            " ".repeat(w)
        } else {
            c.to_string()
        };
        col += w;
        out.push_str(&piece);
        match runs.last_mut() {
            Some(l) if l.1 == tok && l.2 == m && l.3 == ro && l.4 == bad => l.0 += piece.len(),
            _ => runs.push((piece.len(), tok, m, ro, bad)),
        }
    }
    (out, runs)
}

/// `{{name}}` references in a row as `(start, end, resolves)` byte ranges of the document.
pub(crate) fn var_ranges(
    row: &str,
    base: usize,
    check: &Option<crate::code_editor::VarCheck>,
) -> Vec<(usize, usize, bool)> {
    let Some(check) = check else { return vec![] };
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = row[from..].find("{{") {
        let s = from + i;
        let Some(e) = row[s..].find("}}") else { break };
        let end = s + e + 2;
        out.push((base + s, base + end, check(row[s + 2..s + e].trim())));
        from = end;
    }
    out
}

/// One cell per display column: `·` for a space and `→` for a tab (then blanks to the next
/// stop) inside `shown`, blank elsewhere. `None` when nothing in the row is marked.
pub(crate) fn invisibles_row(
    src: &str,
    row: Range<usize>,
    tab: usize,
    shown: Option<Range<usize>>,
) -> Option<String> {
    let mut out = String::new();
    let mut col = 0;
    let mut any = false;
    for (o, c) in src[row.clone()].char_indices() {
        let o = row.start + o;
        let on = shown.as_ref().is_none_or(|r| r.contains(&o));
        let w = if c == '\t' { tab - col % tab } else { 1 };
        col += w;
        match c {
            ' ' if on => {
                any = true;
                out.push('·');
            }
            '\t' if on => {
                any = true;
                out.push('→');
                out.extend(std::iter::repeat_n(' ', w - 1));
            }
            _ => out.extend(std::iter::repeat_n(' ', w)),
        }
    }
    any.then_some(out)
}

/// Columns of the indent guides of a row whose indentation is `width` cells wide.
pub(crate) fn guide_columns(width: usize, tab: usize) -> impl Iterator<Item = usize> {
    (1..).map(move |n| n * tab).take_while(move |c| *c < width)
}

/// Leading whitespace width (cells) of a row; blank rows take `inherit`.
pub(crate) fn indent_width(row: &str, tab: usize, inherit: usize) -> usize {
    if row.trim().is_empty() {
        return inherit;
    }
    let mut col = 0;
    for c in row.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += tab - col % tab,
            _ => break,
        }
    }
    col
}

struct RowPaint {
    y: Pixels,
    line: Option<ShapedLine>,
    invisibles: Option<ShapedLine>,
    chip: Option<(ShapedLine, usize)>,
    number: Option<ShapedLine>,
}

pub struct Prepaint {
    rows: Vec<RowPaint>,
    quads: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    text_origin_x: Pixels,
    gutter_bg: Vec<PaintQuad>,
}

impl Element for CodeEditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let t = cx.global::<Theme>().clone();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let font = style.font();
        let line_h = window.line_height();
        let probe_run = TextRun {
            len: 1,
            font: font.clone(),
            color: t.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_w = window
            .text_system()
            .shape_line("0".into(), font_size, &[probe_run], None)
            .width;
        let focused = self.editor.read(cx).focus.is_focused(window);

        let (rows, quads, caret, gutter_w, gutter_bg) = self.editor.update(cx, |e, _| {
            let digits = e.display().line_count().to_string().len().max(2);
            let gutter_w = if e.cfg.line_numbers {
                char_w * (digits as f32 + 2.)
            } else {
                px(8.)
            };
            e.line_h = line_h;
            e.char_w = char_w;
            e.gutter_w = gutter_w;
            e.bounds = Some(bounds);
            e.wrap_cols = (e.cfg.soft_wrap == SoftWrap::EditorWidth).then(|| {
                (f32::from(bounds.size.width - gutter_w - px(8.)) / f32::from(char_w))
                    .floor()
                    .max(4.) as usize
            });
            let visible = (f32::from(bounds.size.height) / f32::from(line_h)).ceil() as usize + 1;
            e.reveal_caret(visible.saturating_sub(1));
            let total = e.display().row_count();
            e.top_row = e.top_row.clamp(0., (total.saturating_sub(1)) as f32);
            let first = e.top_row.floor() as usize;
            let frac = e.top_row - first as f32;
            let last = (first + visible + 1).min(total);
            let text = e.buf.text().to_string();
            let tab = e.cfg.tab_size;
            let (sel_a, sel_b) = e.buf.selection();
            let head = e.buf.head();
            let head_row = e.display().row_of(head);
            let marked = e.marked.clone();
            let read_only = e.protected_ranges().to_vec();
            let var_check = e.var_check();
            let chips = e.chips().to_vec();
            let empty = text.is_empty();
            let dm_rows: Vec<_> = e.display().rows()[first..last].to_vec();
            let view_bytes = dm_rows.first().map_or(0, |r| r.range.start)
                ..dm_rows.last().map_or(0, |r| r.range.end);
            let spans = e.spans_in(view_bytes);
            let head_col = e.display().col_of(head);
            let origin_x = bounds.left() + gutter_w - e.scroll_x;

            let sel_bg = if focused {
                t.accent_bg()
            } else {
                Hsla {
                    a: t.accent_bg().a * 0.5,
                    ..t.accent
                }
            };
            let mut quads = Vec::new();
            let mut painted = Vec::new();
            let pair = e.bracket_pair();
            let ws_mode = e.cfg.show_whitespace;
            let guides_on = e.cfg.indent_guides;
            let mut prev_indent = 0;
            for (i, r) in dm_rows.iter().enumerate() {
                let ri = first + i;
                let y = bounds.top() + line_h * (i as f32 - frac);
                // current-line highlight
                if ri == head_row && sel_a == sel_b {
                    quads.push(fill(
                        Bounds::new(
                            point(bounds.left() + gutter_w, y),
                            size(bounds.size.width - gutter_w, line_h),
                        ),
                        t.editor_active_line,
                    ));
                }
                // selection
                if sel_a != sel_b {
                    let (lo, hi) = (sel_a.min(sel_b), sel_a.max(sel_b));
                    if lo < r.range.end + 1 && hi > r.range.start {
                        let s = lo.max(r.range.start);
                        let en = hi.min(r.range.end);
                        let sc = e.display().col_of(s);
                        let mut ec = e.display().col_of(en);
                        if hi > r.range.end {
                            ec += 1; // the line break
                        }
                        if ec > sc {
                            quads.push(fill(
                                Bounds::new(
                                    point(origin_x + char_w * sc as f32, y),
                                    size(char_w * (ec - sc) as f32, line_h),
                                ),
                                sel_bg,
                            ));
                        }
                    }
                }
                // indent guides
                if guides_on {
                    let w = indent_width(&text[r.range.clone()], tab, prev_indent);
                    prev_indent = w;
                    for c in guide_columns(w, tab) {
                        quads.push(fill(
                            Bounds::new(
                                point(origin_x + char_w * c as f32, y),
                                size(px(1.), line_h),
                            ),
                            t.editor_indent_guide,
                        ));
                    }
                }
                // matching bracket boxes
                if let Some((a, b)) = pair {
                    for off in [a, b] {
                        if off >= r.range.start && off < r.range.end {
                            let col = e.display().col_of(off);
                            quads.push(fill(
                                Bounds::new(
                                    point(origin_x + char_w * col as f32, y),
                                    size(char_w, line_h),
                                ),
                                t.editor_bracket_match,
                            ));
                        }
                    }
                }
                let row_vars = var_ranges(&text[r.range.clone()], r.range.start, &var_check);
                let (disp, runs) = expand_row(
                    &text,
                    r.range.clone(),
                    &spans,
                    tab,
                    marked.as_ref(),
                    &read_only,
                    &row_vars,
                );
                let line = (!disp.is_empty()).then(|| {
                    let text_runs: Vec<TextRun> = runs
                        .iter()
                        .map(|(len, tok, m, ro, bad)| TextRun {
                            len: *len,
                            font: font.clone(),
                            color: if *ro {
                                t.text_muted
                            } else if *bad {
                                t.deleted
                            } else {
                                tok_color(&t, *tok)
                            },
                            background_color: None,
                            underline: (*m || *bad).then(|| UnderlineStyle {
                                color: Some(if *bad { t.deleted } else { t.text }),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            strikethrough: None,
                        })
                        .collect();
                    window.text_system().shape_line(
                        SharedString::from(disp),
                        font_size,
                        &text_runs,
                        None,
                    )
                });
                let number = (e.cfg.line_numbers && r.first).then(|| {
                    let s = (r.line + 1).to_string();
                    let color = if r.line == e.display().rows()[head_row].line {
                        t.text
                    } else {
                        t.gutter
                    };
                    let run = TextRun {
                        len: s.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_line(SharedString::from(s), font_size, &[run], None)
                });
                let shown = match ws_mode {
                    Whitespace::All => Some(None),
                    Whitespace::Selection if sel_a != sel_b => {
                        Some(Some(sel_a.min(sel_b)..sel_a.max(sel_b)))
                    }
                    _ => None,
                };
                let invisibles = shown
                    .and_then(|sh| invisibles_row(&text, r.range.clone(), tab, sh))
                    .map(|s| {
                        let run = TextRun {
                            len: s.len(),
                            font: font.clone(),
                            color: t.editor_invisible,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        window.text_system().shape_line(
                            SharedString::from(s),
                            font_size,
                            &[run],
                            None,
                        )
                    });
                let chip = chips
                    .iter()
                    .find(|c| r.first && c.offset >= r.range.start && c.offset <= r.range.end)
                    .map(|c| {
                        let label = c.label.clone();
                        let run = TextRun {
                            len: label.len(),
                            font: font.clone(),
                            color: t.text_placeholder,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        (
                            window
                                .text_system()
                                .shape_line(label, font_size * 0.85, &[run], None),
                            c.tag,
                        )
                    });
                painted.push(RowPaint {
                    y,
                    line,
                    invisibles,
                    chip,
                    number,
                });
            }
            if empty && !e.placeholder.is_empty() {
                let run = TextRun {
                    len: e.placeholder.len(),
                    font: font.clone(),
                    color: t.text_placeholder,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let l =
                    window
                        .text_system()
                        .shape_line(e.placeholder.clone(), font_size, &[run], None);
                if let Some(p) = painted.first_mut() {
                    p.line = Some(l);
                }
            }
            let caret = (focused
                && e.caret_visible
                && sel_a == sel_b
                && head_row >= first
                && head_row < last)
                .then(|| {
                    let y = bounds.top() + line_h * (head_row as f32 - e.top_row);
                    fill(
                        Bounds::new(
                            point(origin_x + char_w * head_col as f32, y),
                            size(px(1.5), line_h),
                        ),
                        t.accent,
                    )
                });
            let mut gutter_bg = vec![fill(
                Bounds::new(bounds.origin, size(gutter_w, bounds.size.height)),
                t.editor,
            )];
            // A bar in the gutter beside every read-only row (Docs).
            for (i, r) in dm_rows.iter().enumerate() {
                if read_only.iter().any(|ro| {
                    r.range.start < ro.end && r.range.end >= ro.start && ro.end > ro.start
                }) {
                    let y = bounds.top() + line_h * (i as f32 - frac);
                    gutter_bg.push(fill(
                        Bounds::new(
                            point(bounds.left() + gutter_w - px(4.), y),
                            size(px(2.), line_h),
                        ),
                        t.border,
                    ));
                }
            }
            (painted, quads, caret, gutter_w, gutter_bg)
        });
        let scroll_x = self.editor.read(cx).scroll_x;
        Prepaint {
            rows,
            quads,
            caret,
            text_origin_x: bounds.left() + gutter_w - scroll_x,
            gutter_bg,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        pre: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let e = self.editor.clone();
        window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
            if ev.pressed_button == Some(MouseButton::Left) {
                e.update(cx, |t, cx| t.drag_to(ev.position, cx));
            }
        });
        let line_h = window.line_height();
        let mut rects: Vec<(Bounds<Pixels>, usize)> = Vec::new();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for q in pre.quads.drain(..) {
                window.paint_quad(q);
            }
            for r in &pre.rows {
                if let Some(l) = &r.line {
                    let _ = l.paint(point(pre.text_origin_x, r.y), line_h, window, cx);
                }
                if let Some(l) = &r.invisibles {
                    let _ = l.paint(point(pre.text_origin_x, r.y), line_h, window, cx);
                }
            }
            // The gutter paints over text scrolled under it.
            for q in pre.gutter_bg.drain(..) {
                window.paint_quad(q);
            }
            let gutter_w = self.editor.read(cx).gutter_w;
            for r in &pre.rows {
                if let Some((c, tag)) = &r.chip {
                    let x = bounds.right() - c.width - px(12.);
                    let _ = c.paint(point(x, r.y), line_h, window, cx);
                    rects.push((Bounds::new(point(x, r.y), size(c.width, line_h)), *tag));
                }
                if let Some(n) = &r.number {
                    let x = bounds.left() + gutter_w - n.width - px(10.);
                    let _ = n.paint(point(x, r.y), line_h, window, cx);
                }
            }
            if let Some(q) = pre.caret.take() {
                window.paint_quad(q);
            }
        });
        self.editor.update(cx, |e, _| e.chip_rects = rects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_and_runs_cover_the_row() {
        let src = "\ta";
        let (s, runs) = expand_row(src, 0..2, &vec![], 4, None, &[], &[]);
        assert_eq!(s, "    a");
        assert_eq!(runs.iter().map(|r| r.0).sum::<usize>(), s.len());
    }

    #[test]
    fn runs_split_on_token_marked_and_read_only() {
        let src = "ab cd";
        let spans: Spans = vec![(0, 2, Tok::Keyword)];
        let (s, runs) = expand_row(
            src,
            0..5,
            &spans,
            2,
            Some(&(3..4)),
            &[std::ops::Range { start: 4, end: 5 }],
            &[],
        );
        assert_eq!(s, "ab cd");
        let lens: Vec<_> = runs.iter().map(|r| r.0).collect();
        assert_eq!(lens, [2, 1, 1, 1]);
        assert_eq!(runs[0].1, Tok::Keyword);
        assert!(runs[2].2 && runs[3].3);
    }

    #[test]
    fn a_wrapped_row_only_styles_its_own_range() {
        let spans: Spans = vec![(0, 10, Tok::String)];
        let (s, runs) = expand_row("abcdefghij", 4..8, &spans, 2, None, &[], &[]);
        assert_eq!(s, "efgh");
        assert_eq!(runs, vec![(4, Tok::String, false, false, false)]);
    }

    #[test]
    fn invisibles_mark_spaces_and_tabs_inside_the_shown_range() {
        assert_eq!(invisibles_row("a b", 0..3, 4, None).as_deref(), Some(" · "));
        assert_eq!(
            invisibles_row("\tx", 0..2, 4, None).as_deref(),
            Some("→    ")
        );
        assert_eq!(invisibles_row("ab", 0..2, 4, None), None);
        assert_eq!(
            invisibles_row("a b c", 0..5, 4, Some(0..2)).as_deref(),
            Some(" ·   ")
        );
        assert_eq!(invisibles_row("a b", 0..3, 4, Some(2..3)), None);
    }

    #[test]
    fn guide_columns_follow_the_tab_size() {
        assert_eq!(guide_columns(8, 2).collect::<Vec<_>>(), [2, 4, 6]);
        assert_eq!(guide_columns(4, 4).count(), 0);
        assert_eq!(guide_columns(5, 4).collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn indent_width_expands_tabs_and_blank_rows_inherit() {
        assert_eq!(indent_width("  x", 4, 0), 2);
        assert_eq!(indent_width("\t x", 4, 0), 5);
        assert_eq!(indent_width("   ", 4, 6), 6);
        assert_eq!(indent_width("", 4, 3), 3);
    }

    #[test]
    fn var_ranges_report_each_reference_and_whether_it_resolves() {
        let check: Option<crate::code_editor::VarCheck> =
            Some(std::rc::Rc::new(|n: &str| n == "ok"));
        let r = var_ranges("a {{ok}} b {{ no }} {{open", 10, &check);
        assert_eq!(r, vec![(12, 18, true), (21, 29, false)]);
        assert!(var_ranges("{{x}}", 0, &None).is_empty());
    }

    #[test]
    fn variables_and_unresolved_runs_are_marked() {
        let vars = [(0, 5, true), (6, 11, false)];
        let (s, runs) = expand_row("{{a}} {{b}}", 0..11, &vec![], 2, None, &[], &vars);
        assert_eq!(s, "{{a}} {{b}}");
        assert_eq!(runs.len(), 3);
        assert!(runs[0].1 == Tok::Variable && !runs[0].4);
        assert!(runs[2].4);
    }
}
