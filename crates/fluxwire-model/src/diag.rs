use serde::{Deserialize, Serialize};

/// Half-open byte range into a document's source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn at(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    pub const fn contains(&self, pos: usize) -> bool {
        pos >= self.start && pos < self.end
    }
}

/// A replacement of one span of the source with new text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub span: Span,
    pub text: String,
}

impl Edit {
    pub fn replace(span: Span, text: impl Into<String>) -> Self {
        Self {
            span,
            text: text.into(),
        }
    }

    pub fn insert(pos: usize, text: impl Into<String>) -> Self {
        Self {
            span: Span::at(pos),
            text: text.into(),
        }
    }

    pub fn delete(span: Span) -> Self {
        Self {
            span,
            text: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A suggested fix: a label for the UI and the edits that apply it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable code, e.g. `F104`, `F201`, `F401` (SPEC §7.11).
    pub code: &'static str,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    pub fix: Option<Fix>,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        severity: Severity,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity,
            span,
            message: message.into(),
            fix: None,
        }
    }

    pub fn with_fix(mut self, label: impl Into<String>, edits: Vec<Edit>) -> Self {
        self.fix = Some(Fix {
            label: label.into(),
            edits,
        });
        self
    }
}

/// Apply non-overlapping edits to `src`. Returns `None` if edits overlap or are out of range.
pub fn apply_edits(src: &str, edits: &[Edit]) -> Option<String> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.span.start, e.span.end));
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0;
    for e in sorted {
        if e.span.start < cursor
            || e.span.end > src.len()
            || e.span.start > e.span.end
            || !src.is_char_boundary(e.span.start)
            || !src.is_char_boundary(e.span.end)
        {
            return None;
        }
        out.push_str(&src[cursor..e.span.start]);
        out.push_str(&e.text);
        cursor = e.span.end;
    }
    out.push_str(&src[cursor..]);
    Some(out)
}

/// 0-based (line, column-in-bytes) of a byte offset.
pub fn line_col(src: &str, pos: usize) -> (usize, usize) {
    let pos = pos.min(src.len());
    let before = &src[..pos];
    let line = before.bytes().filter(|b| *b == b'\n').count();
    let col = pos - before.rfind('\n').map_or(0, |i| i + 1);
    (line, col)
}
