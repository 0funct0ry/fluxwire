use fluxwire_model::Span;

/// A line of source: `start..end` excludes the line terminator, `next` is where the next line starts.
#[derive(Debug, Clone, Copy)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub next: usize,
}

impl Line {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    pub fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }
}

/// Split `span` of `src` into lines, treating `\n` and `\r\n` as terminators.
pub fn lines(src: &str, span: Span) -> Vec<Line> {
    let mut out = Vec::new();
    let mut pos = span.start;
    let bytes = src.as_bytes();
    while pos < span.end {
        let mut end = pos;
        while end < span.end && bytes[end] != b'\n' {
            end += 1;
        }
        let next = if end < span.end { end + 1 } else { end };
        let mut text_end = end;
        if text_end > pos && bytes[text_end - 1] == b'\r' {
            text_end -= 1;
        }
        out.push(Line {
            start: pos,
            end: text_end,
            next,
        });
        pos = next;
    }
    out
}

/// Start of the line containing `pos`.
pub fn line_start(src: &str, pos: usize) -> usize {
    src[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// End (exclusive of terminator) of the line containing `pos`.
pub fn line_end(src: &str, pos: usize) -> usize {
    let e = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
    if e > pos && src.as_bytes()[e - 1] == b'\r' {
        e - 1
    } else {
        e
    }
}
