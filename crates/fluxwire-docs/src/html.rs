//! Static, no-JavaScript HTML for docs (SPEC §16). Everything is escaped; link schemes are
//! limited to `http`, `https`, `mailto`, relative and `#`.

use crate::chips;
use crate::generated::{Content, Section};
use crate::headings::anchor;
use crate::model::{Align, Block, Inline, ListItem};
use fluxwire_model::FluxDoc;

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// Rewrites link and image targets (for example relative `.flux.md` links to page anchors).
pub struct Rewrite<'a> {
    pub link: &'a dyn Fn(&str) -> String,
    pub image: &'a dyn Fn(&str) -> String,
}

fn safe_href(h: &str) -> bool {
    let l = h.trim().to_ascii_lowercase();
    if l.starts_with("//") {
        return false;
    }
    match l.split_once(':') {
        Some((scheme, _)) if !scheme.contains(['/', '#', '?']) => {
            matches!(scheme, "http" | "https" | "mailto")
        }
        _ => true,
    }
}

fn inlines(inl: &[Inline], rw: &Rewrite) -> String {
    let mut o = String::new();
    for i in inl {
        if let Some(src) = &i.image {
            if safe_href(src) {
                o.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\">",
                    esc(&(rw.image)(src)),
                    esc(&i.text)
                ));
            } else {
                o.push_str(&esc(&i.text));
            }
            continue;
        }
        let mut s = esc(&i.text).replace('\n', "<br>");
        if i.code {
            s = format!("<code>{s}</code>");
        }
        if i.bold {
            s = format!("<strong>{s}</strong>");
        }
        if i.italic {
            s = format!("<em>{s}</em>");
        }
        if i.strike {
            s = format!("<del>{s}</del>");
        }
        if let Some(l) = &i.link
            && safe_href(l)
        {
            s = format!("<a href=\"{}\">{s}</a>", esc(&(rw.link)(l)));
        }
        o.push_str(&s);
    }
    o
}

/// Minimal JSON token colouring (strings, keys, numbers, literals); other languages are plain.
pub fn highlight(lang: &str, text: &str) -> String {
    if !matches!(lang, "json" | "jsonc") {
        return esc(text);
    }
    let b: Vec<char> = text.chars().collect();
    let (mut o, mut i) = (String::new(), 0);
    while i < b.len() {
        let c = b[i];
        if c == '"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != '"' {
                if b[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let j = (j + 1).min(b.len());
            let s: String = b[i..j].iter().collect();
            let mut k = j;
            while k < b.len() && b[k] == ' ' {
                k += 1;
            }
            let cls = if k < b.len() && b[k] == ':' { "k" } else { "s" };
            o.push_str(&format!("<span class=\"{cls}\">{}</span>", esc(&s)));
            i = j;
        } else if c.is_ascii_digit() || c == '-' {
            let mut j = i + 1;
            while j < b.len()
                && (b[j].is_ascii_digit() || matches!(b[j], '.' | 'e' | 'E' | '+' | '-'))
            {
                j += 1;
            }
            let s: String = b[i..j].iter().collect();
            o.push_str(&format!("<span class=\"n\">{}</span>", esc(&s)));
            i = j;
        } else if c.is_ascii_alphabetic() {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            let s: String = b[i..j].iter().collect();
            if matches!(s.as_str(), "true" | "false" | "null") {
                o.push_str(&format!("<span class=\"n\">{s}</span>"));
            } else {
                o.push_str(&esc(&s));
            }
            i = j;
        } else {
            o.push_str(&esc(&c.to_string()));
            i += 1;
        }
    }
    o
}

/// Context for one file: its parsed document (for chips and examples) and the offset of the
/// Markdown in the file.
pub struct FileCtx<'a> {
    pub doc: Option<&'a FluxDoc>,
    pub base: usize,
}

fn block(b: &Block, cx: &FileCtx, rw: &Rewrite, id_prefix: &str, out: &mut String) {
    match b {
        Block::Heading(l, inl) => {
            let id = format!("{id_prefix}{}", anchor(&crate::model::plain(inl)));
            out.push_str(&format!(
                "<h{l} id=\"{}\">{}</h{l}>\n",
                esc(&id),
                inlines(inl, rw)
            ));
        }
        Block::Paragraph(inl) => out.push_str(&format!("<p>{}</p>\n", inlines(inl, rw))),
        Block::Code {
            lang,
            info,
            text,
            offset,
        } => {
            let at = cx.base + offset;
            if let Some(doc) = cx.doc {
                if chips::is_example_at(doc, at) {
                    out.push_str(&format!(
                        "<figure class=\"example\"><figcaption>Example</figcaption><pre><code>{}</code></pre></figure>\n",
                        highlight(&example_lang(text), &example_body(text))
                    ));
                    return;
                }
                if let Some(c) = chips::chip_at(doc, at) {
                    out.push_str(&format!(
                        "<p><span class=\"chip\">{}</span></p>\n",
                        esc(&c.label)
                    ));
                    return;
                }
            }
            out.push_str(&format!(
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                esc(lang),
                highlight(lang, text)
            ));
            let _ = info;
        }
        Block::List { start, items } => {
            let tag = if start.is_some() { "ol" } else { "ul" };
            match start {
                Some(n) if *n != 1 => out.push_str(&format!("<ol start=\"{n}\">\n")),
                _ => out.push_str(&format!("<{tag}>\n")),
            }
            for it in items {
                list_item(it, cx, rw, id_prefix, out);
            }
            out.push_str(&format!("</{tag}>\n"));
        }
        Block::Quote(inner) => {
            out.push_str("<blockquote>\n");
            for b in inner {
                block(b, cx, rw, id_prefix, out);
            }
            out.push_str("</blockquote>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
        Block::Table {
            header,
            rows,
            align,
        } => {
            let al = |i: usize| match align.get(i) {
                Some(Align::Center) => " style=\"text-align:center\"",
                Some(Align::Right) => " style=\"text-align:right\"",
                _ => "",
            };
            out.push_str("<table>\n<thead><tr>");
            for (i, c) in header.iter().enumerate() {
                out.push_str(&format!("<th{}>{}</th>", al(i), inlines(c, rw)));
            }
            out.push_str("</tr></thead>\n<tbody>\n");
            for r in rows {
                out.push_str("<tr>");
                for (i, c) in r.iter().enumerate() {
                    out.push_str(&format!("<td{}>{}</td>", al(i), inlines(c, rw)));
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody></table>\n");
        }
    }
}

fn list_item(it: &ListItem, cx: &FileCtx, rw: &Rewrite, p: &str, out: &mut String) {
    out.push_str("<li>");
    if let Some(c) = it.checked {
        out.push_str(&format!(
            "<input type=\"checkbox\" disabled{}> ",
            if c { " checked" } else { "" }
        ));
    }
    for b in &it.blocks {
        // Tight items: paragraph content without the wrapper.
        match b {
            Block::Paragraph(inl) => out.push_str(&inlines(inl, rw)),
            b => block(b, cx, rw, p, out),
        }
    }
    out.push_str("</li>\n");
}

/// The language of an example block's body (`json` when it parses as one, else text).
fn example_lang(text: &str) -> String {
    let body = example_body(text);
    if body.trim_start().starts_with(['{', '[']) {
        "json".into()
    } else {
        "text".into()
    }
}

fn example_body(text: &str) -> String {
    text.to_string()
}

pub fn render_blocks(blocks: &[Block], cx: &FileCtx, rw: &Rewrite, id_prefix: &str) -> String {
    let mut o = String::new();
    for b in blocks {
        block(b, cx, rw, id_prefix, &mut o);
    }
    o
}

pub fn render_sections(sections: &[Section]) -> String {
    let mut o = String::new();
    for s in sections {
        o.push_str(&format!(
            "<section class=\"generated\"><h4>{}</h4>\n",
            esc(&s.title)
        ));
        match &s.content {
            Content::Table(rows) => {
                o.push_str("<table><tbody>\n");
                for (k, v) in rows {
                    o.push_str(&format!(
                        "<tr><th>{}</th><td><code>{}</code></td></tr>\n",
                        esc(k),
                        esc(v)
                    ));
                }
                o.push_str("</tbody></table>\n");
            }
            Content::Lines(l) => {
                o.push_str("<ul>\n");
                for t in l {
                    o.push_str(&format!("<li>{}</li>\n", code_ticks(t)));
                }
                o.push_str("</ul>\n");
            }
            Content::Code { lang, text } => o.push_str(&format!(
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                esc(lang),
                highlight(lang, text)
            )),
        }
        o.push_str("</section>\n");
    }
    o
}

/// Escapes `t`, turning `` `x` `` spans into `<code>`.
fn code_ticks(t: &str) -> String {
    t.split('`')
        .enumerate()
        .map(|(i, s)| {
            if i % 2 == 1 {
                format!("<code>{}</code>", esc(s))
            } else {
                esc(s)
            }
        })
        .collect()
}

pub const STYLE: &str = r#":root{--bg:#fafafa;--fg:#383a42;--mute:#696c77;--line:#dbdbdc;--code:#f0f0f1;--acc:#4078f2;--k:#e45649;--s:#50a14f;--n:#986801}
@media (prefers-color-scheme:dark){:root{--bg:#21252b;--fg:#abb2bf;--mute:#7f848e;--line:#3e4451;--code:#282c34;--acc:#61afef;--k:#e06c75;--s:#98c379;--n:#d19a66}}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,sans-serif}
.layout{display:flex;max-width:1100px;margin:0 auto}nav{width:260px;flex:none;padding:24px 16px;position:sticky;top:0;align-self:flex-start;max-height:100vh;overflow:auto;font-size:14px}
nav ul{list-style:none;padding-left:12px;margin:4px 0}nav a{color:var(--mute);text-decoration:none}nav a:hover{color:var(--acc)}
main{flex:1;min-width:0;padding:24px 32px}a{color:var(--acc)}
pre{background:var(--code);padding:12px;border-radius:6px;overflow:auto}code{font:13px ui-monospace,monospace}
p code,li code,td code{background:var(--code);padding:1px 4px;border-radius:3px}
table{border-collapse:collapse;margin:8px 0}th,td{border:1px solid var(--line);padding:4px 10px;text-align:left}
blockquote{border-left:3px solid var(--line);margin:8px 0;padding-left:12px;color:var(--mute)}
.chip{display:inline-block;background:var(--code);border:1px solid var(--line);border-radius:12px;padding:0 10px;font:12px ui-monospace,monospace}
.example figcaption{font-size:12px;color:var(--mute)}.generated h4{margin:16px 0 4px;color:var(--mute)}
.file{border-top:1px solid var(--line);padding-top:8px;margin-top:32px}img{max-width:100%}
.k{color:var(--k)}.s{color:var(--s)}.n{color:var(--n)}
@media (max-width:760px){.layout{display:block}nav{position:static;width:auto;max-height:none}}"#;

/// One page: `title`, a navigation list of `(anchor, label)` and the rendered body.
pub fn page(title: &str, nav: &[(String, String)], body: &str) -> String {
    let mut o = String::new();
    o.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    o.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    o.push_str("<meta name=\"color-scheme\" content=\"light dark\">\n");
    o.push_str(&format!(
        "<title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        esc(title)
    ));
    o.push_str("<div class=\"layout\">\n<nav aria-label=\"Contents\"><ul>\n");
    for (a, l) in nav {
        o.push_str(&format!(
            "<li><a href=\"#{}\">{}</a></li>\n",
            esc(a),
            esc(l)
        ));
    }
    o.push_str("</ul></nav>\n<main>\n");
    o.push_str(body);
    o.push_str("</main>\n</div>\n</body>\n</html>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse;

    fn ident(s: &str) -> String {
        s.to_string()
    }

    fn render(src: &str) -> String {
        let rw = Rewrite {
            link: &ident,
            image: &ident,
        };
        render_blocks(&parse(src), &FileCtx { doc: None, base: 0 }, &rw, "")
    }

    #[test]
    fn escapes_text_and_blocks_unsafe_links() {
        let h =
            render("a <script>x</script> [bad](javascript:alert(1)) [ok](https://e.com?a=1&b=2)\n");
        assert!(!h.contains("<script>"));
        assert!(!h.contains("href=\"javascript:"), "{h}");
        assert!(h.contains("href=\"https://e.com?a=1&amp;b=2\""));
    }

    #[test]
    fn tables_lists_tasks_and_code() {
        let h = render(
            "| a | b |\n|---|--:|\n| 1 | 2 |\n\n- [x] done\n- todo\n\n```json\n{\"a\": 1}\n```\n",
        );
        assert!(h.contains("<th style=\"text-align:right\">"));
        assert!(h.contains("checkbox\" disabled checked"));
        assert!(h.contains("<span class=\"k\">&quot;a&quot;</span>"));
    }

    #[test]
    fn chips_and_examples_for_flux_blocks() {
        let src = "---\nname: A\n---\n\n# A\n\n```http\nGET {{u}}/x\n```\n\n```http example\nHTTP 200\n\n{\"id\": 1}\n```\n";
        let doc = fluxwire_format::parse(src);
        let base = crate::base_of(&doc);
        let rw = Rewrite {
            link: &ident,
            image: &ident,
        };
        let h = render_blocks(
            &parse(&src[base..]),
            &FileCtx {
                doc: Some(&doc),
                base,
            },
            &rw,
            "",
        );
        assert!(h.contains("<span class=\"chip\">GET {{u}}/x</span>"));
        assert!(h.contains("class=\"example\""));
    }

    #[test]
    fn page_is_scriptless_and_themed() {
        let p = page("T", &[("a".into(), "A".into())], "<p>x</p>");
        assert!(p.contains("prefers-color-scheme:dark"));
        assert!(!p.contains("<script"));
    }
}
