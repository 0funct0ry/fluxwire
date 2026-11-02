//! Buffer offset ↔ display row/column for the code editor (pure, monospace cells).
//!
//! A buffer line becomes one display row, or several with soft wrap. Columns are cells after
//! tab expansion (`tab_size` stops). Wrapping breaks after whitespace or `-` where it can,
//! and at the cell limit for tokens longer than a row.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Byte range of the row's text (no newline).
    pub range: Range<usize>,
    /// Index of the buffer line this row belongs to.
    pub line: usize,
    /// First row of its line.
    pub first: bool,
}

#[derive(Debug, Clone)]
pub struct DisplayMap {
    text: String,
    tab_size: usize,
    rows: Vec<Row>,
}

fn advance(col: usize, c: char, tab: usize) -> usize {
    if c == '\t' {
        col + tab - col % tab
    } else {
        col + 1
    }
}

impl DisplayMap {
    /// `wrap`: maximum cells per row, or `None` for no soft wrap.
    pub fn new(text: &str, wrap: Option<usize>, tab_size: usize) -> Self {
        let tab = tab_size.max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        for (line, l) in text.split('\n').enumerate() {
            let end = start + l.len();
            match wrap.filter(|w| *w > 0) {
                None => rows.push(Row {
                    range: start..end,
                    line,
                    first: true,
                }),
                Some(w) => Self::wrap_line(text, start, end, line, w, tab, &mut rows),
            }
            start = end + 1;
        }
        Self {
            text: text.to_string(),
            tab_size: tab,
            rows,
        }
    }

    fn wrap_line(
        text: &str,
        start: usize,
        end: usize,
        line: usize,
        w: usize,
        tab: usize,
        rows: &mut Vec<Row>,
    ) {
        let mut row_start = start;
        let mut col = 0;
        let mut brk: Option<usize> = None; // last break opportunity (byte offset after the char)
        let mut first = true;
        let mut i = start;
        for (o, c) in text[start..end].char_indices() {
            let o = start + o;
            let next = advance(col, c, tab);
            if next > w && o > row_start {
                let cut = brk.filter(|b| *b > row_start).unwrap_or(o);
                rows.push(Row {
                    range: row_start..cut,
                    line,
                    first,
                });
                first = false;
                row_start = cut;
                // recompute column of text carried over to the new row
                col = 0;
                for ch in text[row_start..o].chars() {
                    col = advance(col, ch, tab);
                }
                brk = None;
            }
            col = advance(col, c, tab);
            i = o + c.len_utf8();
            if c == ' ' || c == '\t' || c == '-' {
                brk = Some(i);
            }
        }
        let _ = i;
        rows.push(Row {
            range: row_start..end,
            line,
            first,
        });
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn line_count(&self) -> usize {
        self.rows.last().map_or(1, |r| r.line + 1)
    }

    /// Row holding `offset`. At a wrap boundary the caret belongs to the next row, except at
    /// the end of a line, which stays on the line's last row.
    pub fn row_of(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let idx = self.rows.partition_point(|r| r.range.start <= offset);
        idx.saturating_sub(1)
    }

    /// Display column (cells) of `offset` within its row.
    pub fn col_of(&self, offset: usize) -> usize {
        let r = &self.rows[self.row_of(offset)];
        let off = offset.clamp(r.range.start, r.range.end);
        self.text[r.range.start..off]
            .chars()
            .fold(0, |c, ch| advance(c, ch, self.tab_size))
    }

    /// Width of a row in cells.
    pub fn row_width(&self, row: usize) -> usize {
        let r = &self.rows[row];
        self.text[r.range.clone()]
            .chars()
            .fold(0, |c, ch| advance(c, ch, self.tab_size))
    }

    /// Offset nearest to cell `col` in `row` (clicks, Up/Down with a goal column). A click in
    /// the right half of a cell lands after it.
    pub fn offset_at(&self, row: usize, col: usize) -> usize {
        let row = row.min(self.rows.len() - 1);
        let r = &self.rows[row];
        let mut c = 0;
        for (o, ch) in self.text[r.range.clone()].char_indices() {
            let n = advance(c, ch, self.tab_size);
            if col < c + (n - c).div_ceil(2) {
                return r.range.start + o;
            }
            if col < n {
                return r.range.start + o + ch.len_utf8();
            }
            c = n;
        }
        // past the end; for a wrapped (non-last) row stay before the break
        r.range.end
    }

    /// Rows `[first, last)` that intersect a viewport of `height` rows starting at `top`.
    pub fn visible(&self, top: usize, height: usize) -> Range<usize> {
        top.min(self.rows.len())..(top + height).min(self.rows.len())
    }

    /// Row reached by moving `delta` rows from `offset` keeping goal column `goal`.
    pub fn vertical(&self, offset: usize, delta: isize, goal: usize) -> usize {
        let row = self.row_of(offset) as isize + delta;
        if row < 0 {
            return 0;
        }
        if row as usize >= self.rows.len() {
            return self.text.len();
        }
        let o = self.offset_at(row as usize, goal);
        // keep the caret on the intended row at a wrap boundary
        let r = &self.rows[row as usize];
        if o == r.range.end && self.row_of(o) != row as usize {
            o.saturating_sub(self.text[..o].chars().last().map_or(0, char::len_utf8))
        } else {
            o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(m: &DisplayMap, s: &str) -> Vec<String> {
        m.rows()
            .iter()
            .map(|r| s[r.range.clone()].to_string())
            .collect()
    }

    #[test]
    fn no_wrap_is_one_row_per_line() {
        let s = "ab\n\ncde";
        let m = DisplayMap::new(s, None, 2);
        assert_eq!(texts(&m, s), ["ab", "", "cde"]);
        assert_eq!((m.row_count(), m.line_count()), (3, 3));
        assert_eq!(m.row_of(3), 1);
        assert_eq!(m.row_of(s.len()), 2);
    }

    #[test]
    fn wraps_at_word_boundaries() {
        let s = "hello big world";
        let m = DisplayMap::new(s, Some(10), 2);
        assert_eq!(texts(&m, s), ["hello big ", "world"]);
        assert!(m.rows()[0].first && !m.rows()[1].first);
        assert_eq!(m.rows()[1].line, 0);
    }

    #[test]
    fn long_token_breaks_at_the_limit() {
        let s = "abcdefghij";
        let m = DisplayMap::new(s, Some(4), 2);
        assert_eq!(texts(&m, s), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn tabs_expand_to_stops() {
        let s = "\tx\ty";
        let m = DisplayMap::new(s, None, 4);
        assert_eq!(m.col_of(1), 4);
        assert_eq!(m.col_of(2), 5);
        assert_eq!(m.col_of(3), 8);
        assert_eq!(m.row_width(0), 9);
    }

    #[test]
    fn wrap_boundary_caret_belongs_to_next_row() {
        let s = "abcdefghij";
        let m = DisplayMap::new(s, Some(4), 2);
        assert_eq!(m.row_of(4), 1);
        assert_eq!(m.col_of(4), 0);
        assert_eq!(m.row_of(10), 2);
        assert_eq!(m.col_of(10), 2);
    }

    #[test]
    fn offset_at_maps_clicks() {
        let s = "abc\ndef";
        let m = DisplayMap::new(s, None, 2);
        assert_eq!(m.offset_at(0, 0), 0);
        assert_eq!(m.offset_at(0, 2), 2);
        assert_eq!(m.offset_at(0, 99), 3);
        assert_eq!(m.offset_at(1, 1), 5);
        assert_eq!(m.offset_at(9, 1), 5);
    }

    #[test]
    fn click_in_right_half_of_a_tab_lands_after_it() {
        let m = DisplayMap::new("\tx", None, 4);
        assert_eq!(m.offset_at(0, 1), 0);
        assert_eq!(m.offset_at(0, 3), 1);
    }

    #[test]
    fn multibyte_offsets_are_char_boundaries() {
        let s = "é😀x";
        let m = DisplayMap::new(s, None, 2);
        assert_eq!(m.offset_at(0, 1), 2);
        assert_eq!(m.offset_at(0, 2), 6);
        assert_eq!(m.col_of(6), 2);
    }

    #[test]
    fn vertical_keeps_goal_column_across_wrapped_rows() {
        let s = "abcdefghij\nxy";
        let m = DisplayMap::new(s, Some(4), 2);
        assert_eq!(m.vertical(2, 1, 2), 6); // row 0 col 2 → row 1 col 2
        assert_eq!(m.vertical(6, 1, 2), 10);
    }

    #[test]
    fn vertical_clamps_at_the_ends() {
        let s = "ab\ncd";
        let m = DisplayMap::new(s, None, 2);
        assert_eq!(m.vertical(1, -1, 1), 0);
        assert_eq!(m.vertical(4, 1, 1), s.len());
    }

    #[test]
    fn visible_clamps() {
        let m = DisplayMap::new("a\nb\nc", None, 2);
        assert_eq!(m.visible(1, 10), 1..3);
        assert_eq!(m.visible(9, 2), 3..3);
    }
}
