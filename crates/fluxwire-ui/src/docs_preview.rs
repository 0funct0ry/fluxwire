//! The Docs *Preview*: the file's Markdown rendered with GPUI elements (M9.06). Read-only;
//! the *Source* mode is the Markdown `CodeEditor`. Links open in the browser, and the chip on a
//! request block opens the sub-tab that owns it.

use crate::code_editor::Chip;
use crate::editor::tok_color;
use crate::highlight::{Lang, highlight, line_runs};
use crate::markdown_view::{Align, Block, Inline, ListItem};
use crate::theme::Theme;
use crate::workspace::WorkspaceView;
use gpui::{
    AnyElement, Context, FontStyle, FontWeight, HighlightStyle, InteractiveText, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, StrikethroughStyle, Styled,
    StyledText, UnderlineStyle, Window, div, prelude::*, px,
};
use std::ops::Range;

/// Text, highlight ranges and the `(range, url)` of each link of a run of inlines.
type InlineParts = (
    String,
    Vec<(Range<usize>, HighlightStyle)>,
    Vec<(Range<usize>, String)>,
);

fn lang_of(name: &str) -> Lang {
    match name.to_ascii_lowercase().as_str() {
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "ts" | "typescript" => Lang::Ts,
        "js" | "javascript" => Lang::Js,
        "xml" => Lang::Xml,
        _ => Lang::Plain,
    }
}

/// Text, highlight ranges and the `(range, url)` of each link of a run of inlines.
fn inline_parts(inl: &[Inline], t: &Theme) -> InlineParts {
    let mut text = String::new();
    let mut hl = Vec::new();
    let mut links = Vec::new();
    for i in inl {
        let start = text.len();
        text.push_str(&i.text);
        let r = start..text.len();
        if r.is_empty() {
            continue;
        }
        let mut s = HighlightStyle::default();
        let mut styled = false;
        if i.bold {
            s.font_weight = Some(FontWeight::BOLD);
            styled = true;
        }
        if i.italic {
            s.font_style = Some(FontStyle::Italic);
            styled = true;
        }
        if i.strike {
            s.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(t.text_muted),
            });
            styled = true;
        }
        if i.code {
            s.color = Some(t.syntax_string);
            s.background_color = Some(t.element_hover);
            styled = true;
        }
        if let Some(u) = &i.link {
            s.color = Some(t.accent);
            s.underline = Some(UnderlineStyle {
                color: Some(t.accent),
                thickness: px(1.),
                wavy: false,
            });
            links.push((r.clone(), u.clone()));
            styled = true;
        }
        if styled {
            hl.push((r, s));
        }
    }
    (text, hl, links)
}

impl WorkspaceView {
    /// Follows a link of a Docs preview (SPEC §16): external links open in the browser, relative
    /// `.flux.md` links open that request or folder in the app.
    pub(crate) fn open_doc_link(
        &mut self,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fluxwire_docs::links::{Link, resolve};
        let from = self.active_key().unwrap_or_default();
        match resolve(&from, href) {
            Link::External(u) => cx.open_url(&u),
            Link::Doc { path, .. } if path.ends_with(".flux.md") => {
                self.open_request(&path, true, window, cx)
            }
            Link::Anchor(_) => {}
            Link::Doc { path, .. } => {
                self.notice = Some(format!("{path}: open Markdown files from the tree."));
                cx.notify();
            }
            Link::Other(o) => {
                self.notice = Some(format!("{o}: this link cannot be opened here."));
                cx.notify();
            }
        }
    }

    /// The generated sections beneath the prose (SPEC §16): never written to disk.
    fn render_generated_sections(&self, t: &Theme, key: &str) -> Vec<AnyElement> {
        use fluxwire_docs::generated::{Content, generate};
        let Some(doc) = self.scope.workspace.as_ref().and_then(|p| p.doc(key)) else {
            return vec![];
        };
        generate(&doc.parsed)
            .into_iter()
            .map(|s| {
                let body: AnyElement = match s.content {
                    Content::Table(rows) => div()
                        .flex()
                        .flex_col()
                        .children(rows.into_iter().map(|(k, v)| {
                            div()
                                .flex()
                                .flex_row()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(140.))
                                        .flex_none()
                                        .text_color(t.text_muted)
                                        .child(k),
                                )
                                .child(div().min_w_0().child(v))
                        }))
                        .into_any_element(),
                    Content::Lines(l) => div()
                        .flex()
                        .flex_col()
                        .children(
                            l.into_iter()
                                .map(|x| div().child(format!("• {}", x.replace('`', "")))),
                        )
                        .into_any_element(),
                    Content::Code { text, .. } => div()
                        .p_2()
                        .rounded(px(4.))
                        .bg(t.element_hover)
                        .child(text)
                        .into_any_element(),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .border_t_1()
                    .border_color(t.border_variant)
                    .pt_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(s.title.to_uppercase()),
                    )
                    .child(body)
                    .into_any_element()
            })
            .collect()
    }

    fn md_inline(
        &self,
        id: SharedString,
        inl: &[Inline],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (text, hl, links) = inline_parts(inl, t);
        let styled = StyledText::new(text).with_highlights(hl);
        if links.is_empty() {
            return styled.into_any_element();
        }
        let ranges: Vec<Range<usize>> = links.iter().map(|l| l.0.clone()).collect();
        let urls: Vec<String> = links.into_iter().map(|l| l.1).collect();
        let ws = cx.entity();
        InteractiveText::new(id, styled)
            .on_click(ranges, move |ix, window, cx| {
                let url = urls[ix].clone();
                ws.update(cx, |this, cx| this.open_doc_link(&url, window, cx));
            })
            .into_any_element()
    }

    fn md_block(
        &self,
        id: String,
        b: &Block,
        t: &Theme,
        chips: &[Chip],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match b {
            Block::Heading(level, inl) => {
                let size = match level {
                    1 => 22.,
                    2 => 18.,
                    3 => 16.,
                    _ => 14.,
                };
                div()
                    .mt(px(if *level <= 2 { 10. } else { 6. }))
                    .text_size(px(size))
                    .font_weight(FontWeight::BOLD)
                    .when(*level <= 2, |d| {
                        d.pb_1().border_b_1().border_color(t.border_variant)
                    })
                    .child(self.md_inline(id.into(), inl, t, cx))
                    .into_any_element()
            }
            Block::Paragraph(inl) => div()
                .child(self.md_inline(id.into(), inl, t, cx))
                .into_any_element(),
            Block::Code {
                info,
                text,
                offset,
                lang,
            } => {
                let spans = highlight(text, lang_of(lang));
                let rows = line_runs(text, &spans);
                let chip = chips.iter().find(|c| c.offset == *offset).cloned();
                let mut card = div()
                    .flex()
                    .flex_col()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border_variant)
                    .bg(t.element_hover);
                if let Some(c) = chip {
                    let tag = c.tag;
                    card = card.child(
                        div()
                            .id(SharedString::from(format!("{id}-chip")))
                            .px_2()
                            .py_1()
                            .border_b_1()
                            .border_color(t.border_variant)
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .cursor_pointer()
                            .hover({
                                let a = t.accent;
                                move |s| s.text_color(a)
                            })
                            .child(c.label)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open_chip_target(tag, cx)),
                            ),
                    );
                } else if !info.is_empty() {
                    card = card.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(t.text_placeholder)
                            .child(info.clone()),
                    );
                }
                card.overflow_hidden()
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-code")))
                            .overflow_x_scroll()
                            .px_2()
                            .py_1()
                            .font_family(crate::assets::BUFFER_FONT)
                            .text_size(px(12.))
                            .flex()
                            .flex_col()
                            .children(rows.into_iter().map(|line| {
                                div().flex().flex_row().min_h(px(16.)).children(
                                    line.into_iter().map(|(s, tok)| {
                                        div()
                                            .text_color(tok_color(t, tok))
                                            .whitespace_nowrap()
                                            .child(s)
                                    }),
                                )
                            })),
                    )
                    .into_any_element()
            }
            Block::List { start, items } => {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(items.iter().enumerate().map(|(n, it)| {
                        self.md_item(format!("{id}-{n}"), *start, n, it, t, chips, cx)
                    }))
                    .into_any_element()
            }
            Block::Quote(inner) => div()
                .pl_3()
                .border_l_2()
                .border_color(t.border)
                .text_color(t.text_muted)
                .flex()
                .flex_col()
                .gap_2()
                .children(
                    inner
                        .iter()
                        .enumerate()
                        .map(|(n, b)| self.md_block(format!("{id}-{n}"), b, t, chips, cx)),
                )
                .into_any_element(),
            Block::Rule => div().my_1().h(px(1.)).bg(t.border).into_any_element(),
            Block::Table {
                header,
                rows,
                align,
            } => {
                let cell = |text: &str, i: usize, bold: bool| {
                    div()
                        .flex_1()
                        .px_2()
                        .py_1()
                        .when(bold, |d| d.font_weight(FontWeight::BOLD))
                        .when(matches!(align.get(i), Some(Align::Right)), |d| {
                            d.text_right()
                        })
                        .when(matches!(align.get(i), Some(Align::Center)), |d| {
                            d.text_center()
                        })
                        .child(text.to_string())
                };
                let line = |cells: &Vec<Vec<Inline>>, bold: bool| {
                    div()
                        .flex()
                        .flex_row()
                        .border_b_1()
                        .border_color(t.border_variant)
                        .children(
                            cells
                                .iter()
                                .enumerate()
                                .map(|(i, c)| cell(&crate::markdown_view::plain(c), i, bold)),
                        )
                };
                div()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.border_variant)
                    .child(line(header, true).bg(t.element_hover))
                    .children(rows.iter().map(|r| line(r, false)))
                    .into_any_element()
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn md_item(
        &self,
        id: String,
        start: Option<u64>,
        n: usize,
        it: &ListItem,
        t: &Theme,
        chips: &[Chip],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let marker = match (it.checked, start) {
            (Some(true), _) => "☑".to_string(),
            (Some(false), _) => "☐".to_string(),
            (None, Some(s)) => format!("{}.", s + n as u64),
            (None, None) => "•".to_string(),
        };
        div()
            .flex()
            .flex_row()
            .gap_2()
            .child(
                div()
                    .w(px(20.))
                    .flex_none()
                    .text_color(t.text_muted)
                    .child(marker),
            )
            .child(
                div().flex_1().min_w_0().flex().flex_col().gap_1().children(
                    it.blocks
                        .iter()
                        .enumerate()
                        .map(|(i, b)| self.md_block(format!("{id}-{i}"), b, t, chips, cx)),
                ),
            )
            .into_any_element()
    }

    /// The rendered Docs of `key` (blocks of its prose and recognised fences).
    pub(crate) fn render_docs_preview(
        &self,
        t: &Theme,
        key: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let blocks = crate::markdown_view::parse(text);
        let chips = self.docs_chips(key, crate::workspace_model::Target::Docs);
        let body = if blocks.is_empty() {
            div()
                .text_color(t.text_placeholder)
                .child("No documentation yet. Switch to Source to write it in Markdown.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    blocks
                        .iter()
                        .enumerate()
                        .map(|(i, b)| self.md_block(format!("md{i}"), b, t, &chips, cx)),
                )
                .into_any_element()
        };
        let generated = self.render_generated_sections(t, key);
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(body)
            .children(generated);
        div()
            .id("docs-preview-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .text_size(px(14.))
            .line_height(px(21.))
            .child(div().max_w(px(820.)).child(body))
            .into_any_element()
    }
}
