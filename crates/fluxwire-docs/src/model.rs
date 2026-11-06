//! Markdown to a small block model for the Docs *Preview* (M9.06): headings, paragraphs with
//! inline styles and links, lists, quotes, tables, rules and code blocks. Pure; the GPUI
//! rendering is in `fluxwire-ui/src/docs_preview.rs` and `html.rs`.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// Link target when the text is (part of) a link.
    pub link: Option<String>,
    /// Image source when this run is an image; `text` is then its alt text.
    pub image: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Code {
        /// First word of the info string (may be empty).
        lang: String,
        /// The whole info string.
        info: String,
        text: String,
        /// Byte offset of the opening fence in the parsed text.
        offset: usize,
    },
    /// `start` is the first number of an ordered list. Each item is a list of blocks;
    /// `checked` is the state of a task item.
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Rule,
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        align: Vec<Align>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    link: Vec<String>,
    image: Vec<String>,
}

/// Blocks of `src`. Raw HTML is shown as text; images as their alt text.
pub fn parse(src: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut it = Parser::new_ext(src, opts).into_offset_iter().peekable();
    blocks_until(&mut it, None)
}

type Iter<'a> = std::iter::Peekable<pulldown_cmark::OffsetIter<'a>>;

fn heading_level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Reads inline events up to (and consuming) `end`.
fn inlines_until(it: &mut Iter, end: &TagEnd) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut st = Style::default();
    let push = |out: &mut Vec<Inline>, text: &str, code: bool, st: &Style| {
        if text.is_empty() {
            return;
        }
        let cur = Inline {
            text: text.to_string(),
            bold: st.bold > 0,
            italic: st.italic > 0,
            strike: st.strike > 0,
            code,
            link: st.link.last().cloned(),
            image: st.image.last().cloned(),
        };
        match out.last_mut() {
            Some(l) if same_style(l, &cur) => l.text.push_str(text),
            _ => out.push(cur),
        }
    };
    for (ev, _) in it.by_ref() {
        match ev {
            Event::End(e) if &e == end => break,
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                push(&mut out, &t, false, &st)
            }
            Event::Code(t) => push(&mut out, &t, true, &st),
            Event::SoftBreak => push(&mut out, " ", false, &st),
            Event::HardBreak => push(&mut out, "\n", false, &st),
            Event::Start(Tag::Strong) => st.bold += 1,
            Event::End(TagEnd::Strong) => st.bold = st.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => st.italic += 1,
            Event::End(TagEnd::Emphasis) => st.italic = st.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => st.strike += 1,
            Event::End(TagEnd::Strikethrough) => st.strike = st.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => st.link.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                st.link.pop();
            }
            Event::Start(Tag::Image { dest_url, .. }) => st.image.push(dest_url.to_string()),
            Event::End(TagEnd::Image) => {
                st.image.pop();
            }
            _ => {}
        }
    }
    out
}

fn same_style(a: &Inline, b: &Inline) -> bool {
    a.bold == b.bold
        && a.italic == b.italic
        && a.strike == b.strike
        && a.code == b.code
        && a.link == b.link
        && a.image == b.image
}

/// Reads block events until `end` (consumed), or to the end of input when `None`.
fn blocks_until(it: &mut Iter, end: Option<&TagEnd>) -> Vec<Block> {
    let mut out = Vec::new();
    // Text of a tight list item arrives without a paragraph wrapper.
    let mut loose: Vec<Inline> = Vec::new();
    let flush = |out: &mut Vec<Block>, loose: &mut Vec<Inline>| {
        if !loose.is_empty() {
            out.push(Block::Paragraph(std::mem::take(loose)));
        }
    };
    while let Some((ev, range)) = it.next() {
        match ev {
            Event::End(e) if Some(&e) == end => break,
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut out, &mut loose);
                let l = heading_level(level);
                let inl = inlines_until(it, &TagEnd::Heading(level));
                out.push(Block::Heading(l, inl));
            }
            Event::Start(Tag::Paragraph) => {
                flush(&mut out, &mut loose);
                let inl = inlines_until(it, &TagEnd::Paragraph);
                out.push(Block::Paragraph(inl));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut out, &mut loose);
                let info = match &kind {
                    CodeBlockKind::Fenced(i) => i.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut text = String::new();
                for (ev, _) in it.by_ref() {
                    match ev {
                        Event::Text(t) => text.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
                out.push(Block::Code {
                    lang: info.split_whitespace().next().unwrap_or("").to_string(),
                    info,
                    text,
                    offset: range.start,
                });
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut out, &mut loose);
                let mut items = Vec::new();
                while let Some((ev, _)) = it.peek() {
                    match ev {
                        Event::Start(Tag::Item) => {
                            it.next();
                            let mut checked = None;
                            if let Some((Event::TaskListMarker(c), _)) = it.peek() {
                                checked = Some(*c);
                                it.next();
                            }
                            let blocks = blocks_until(it, Some(&TagEnd::Item));
                            items.push(ListItem { checked, blocks });
                        }
                        Event::End(TagEnd::List(_)) => {
                            it.next();
                            break;
                        }
                        _ => {
                            it.next();
                        }
                    }
                }
                out.push(Block::List { start, items });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut out, &mut loose);
                let inner = blocks_until(it, Some(&TagEnd::BlockQuote(None)));
                out.push(Block::Quote(inner));
            }
            Event::Start(Tag::Table(aligns)) => {
                flush(&mut out, &mut loose);
                let align = aligns
                    .iter()
                    .map(|a| match a {
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                        _ => Align::Left,
                    })
                    .collect();
                let (mut header, mut rows) = (Vec::new(), Vec::new());
                let mut cur: Vec<Vec<Inline>> = Vec::new();
                for (ev, _) in it.by_ref() {
                    match ev {
                        Event::Start(Tag::TableCell) => {}
                        Event::End(TagEnd::TableCell) => {}
                        Event::End(TagEnd::TableHead) => header = std::mem::take(&mut cur),
                        Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut cur)),
                        Event::End(TagEnd::Table) => break,
                        Event::Text(t) | Event::Code(t) => cur.push(vec![Inline {
                            text: t.to_string(),
                            ..Default::default()
                        }]),
                        _ => {}
                    }
                }
                out.push(Block::Table {
                    header,
                    rows,
                    align,
                });
            }
            Event::Rule => {
                flush(&mut out, &mut loose);
                out.push(Block::Rule);
            }
            // Inline content directly inside a tight list item.
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => loose.push(Inline {
                text: t.to_string(),
                ..Default::default()
            }),
            Event::Code(t) => loose.push(Inline {
                text: t.to_string(),
                code: true,
                ..Default::default()
            }),
            Event::SoftBreak => loose.push(Inline {
                text: " ".into(),
                ..Default::default()
            }),
            Event::Start(Tag::Strong)
            | Event::Start(Tag::Emphasis)
            | Event::Start(Tag::Link { .. }) => {
                // Styled inline in a tight item: re-read it as inlines of a synthetic paragraph.
                let (end_tag, tmpl) = match &ev {
                    Event::Start(Tag::Strong) => (TagEnd::Strong, 0),
                    Event::Start(Tag::Emphasis) => (TagEnd::Emphasis, 1),
                    Event::Start(Tag::Link { dest_url, .. }) => {
                        let u = dest_url.to_string();
                        let mut inl = inlines_until(it, &TagEnd::Link);
                        for i in &mut inl {
                            i.link = Some(u.clone());
                        }
                        loose.extend(inl);
                        continue;
                    }
                    _ => unreachable!(),
                };
                let mut inl = inlines_until(it, &end_tag);
                for i in &mut inl {
                    if tmpl == 0 {
                        i.bold = true;
                    } else {
                        i.italic = true;
                    }
                }
                loose.extend(inl);
            }
            _ => {}
        }
    }
    flush(&mut out, &mut loose);
    out
}

/// Plain text of inlines (for tests and copy).
pub fn plain(inl: &[Inline]) -> String {
    inl.iter().map(|i| i.text.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let b = parse(
            "# Title\n\nSome **bold**, *it*, `code`, ~~gone~~ and [a link](https://e.com/x).\n",
        );
        assert!(matches!(&b[0], Block::Heading(1, i) if plain(i) == "Title"));
        let Block::Paragraph(p) = &b[1] else {
            panic!("{b:?}")
        };
        assert_eq!(plain(p), "Some bold, it, code, gone and a link.");
        assert!(p.iter().any(|i| i.bold && i.text == "bold"));
        assert!(p.iter().any(|i| i.italic && i.text == "it"));
        assert!(p.iter().any(|i| i.code && i.text == "code"));
        assert!(p.iter().any(|i| i.strike && i.text == "gone"));
        assert!(
            p.iter()
                .any(|i| i.link.as_deref() == Some("https://e.com/x") && i.text == "a link")
        );
    }

    #[test]
    fn code_blocks_keep_info_text_and_offset() {
        let src = "intro\n\n```ts pre\nlet a = 1;\n```\n";
        let b = parse(src);
        let Block::Code {
            lang,
            info,
            text,
            offset,
        } = &b[1]
        else {
            panic!("{b:?}")
        };
        assert_eq!(
            (lang.as_str(), info.as_str(), text.as_str()),
            ("ts", "ts pre", "let a = 1;")
        );
        assert_eq!(&src[*offset..*offset + 3], "```");
    }

    #[test]
    fn lists_nest_and_number() {
        let b = parse("1. one\n2. two\n   - inner\n- [x] done\n- [ ] todo\n");
        let Block::List { start, items } = &b[0] else {
            panic!("{b:?}")
        };
        assert_eq!(*start, Some(1));
        assert_eq!(items.len(), 2);
        assert!(matches!(
            &items[1].blocks[1],
            Block::List { start: None, .. }
        ));
        let Block::List { items: tasks, .. } = &b[1] else {
            panic!("{b:?}")
        };
        assert_eq!(tasks[0].checked, Some(true));
        assert_eq!(tasks[1].checked, Some(false));
    }

    #[test]
    fn tight_list_items_keep_their_styled_text() {
        let b = parse("- plain **bold** [l](http://x)\n");
        let Block::List { items, .. } = &b[0] else {
            panic!("{b:?}")
        };
        let Block::Paragraph(p) = &items[0].blocks[0] else {
            panic!("{b:?}")
        };
        assert_eq!(plain(p), "plain bold l");
        assert!(p.iter().any(|i| i.bold) && p.iter().any(|i| i.link.is_some()));
    }

    #[test]
    fn quotes_rules_and_tables() {
        let b = parse("> quoted\n\n---\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n");
        assert!(
            matches!(&b[0], Block::Quote(q) if matches!(&q[0], Block::Paragraph(p) if plain(p) == "quoted"))
        );
        assert_eq!(b[1], Block::Rule);
        let Block::Table {
            header,
            rows,
            align,
        } = &b[2]
        else {
            panic!("{b:?}")
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(align, &[Align::Left, Align::Right]);
    }

    #[test]
    fn empty_input_has_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
    }
}
