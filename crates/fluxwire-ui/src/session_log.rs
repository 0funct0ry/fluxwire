//! Session log (SPEC §10, §17.4): the frames of a live or stored session protocol run.
//!
//! Pure state, shared by the WebSocket tab and History entries, and by later session
//! protocols (SSE, MCP notifications, brokers). It keeps at most [`LOG_CAP`] frames, counts
//! the ones dropped from the front, and tracks which frames pass the direction and text
//! filters so the list can be virtualized.

use std::collections::VecDeque;

use fluxwire_runner::Frame;
use gpui::UniformListScrollHandle;

use crate::{highlight::Lang, viewer};

/// Frames kept in the log; older ones are dropped and counted.
pub const LOG_CAP: usize = 50_000;
/// Longest detail shown for one frame.
const DETAIL_CAP: usize = 256 * 1024;
const PREVIEW_CHARS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DirFilter {
    #[default]
    All,
    Sent,
    Received,
}

impl DirFilter {
    fn allows(self, outbound: bool) -> bool {
        match self {
            DirFilter::All => true,
            DirFilter::Sent => outbound,
            DirFilter::Received => !outbound,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    /// Increases by one per frame for the life of the log, so ids survive dropping.
    pub id: u64,
    pub frame: Frame,
}

pub struct SessionLog {
    entries: VecDeque<LogEntry>,
    next_id: u64,
    /// Frames dropped from the front because of the cap.
    pub dropped: u64,
    pub dir: DirFilter,
    pub filter: String,
    pub filter_input: crate::text_input::LazyInput,
    /// Ids of the entries that pass the filters, oldest first.
    visible: VecDeque<u64>,
    pub selected: Option<u64>,
    pub scroll: UniformListScrollHandle,
    pub follow: bool,
    cap: usize,
}

impl Default for SessionLog {
    fn default() -> Self {
        Self::with_cap(LOG_CAP)
    }
}

impl SessionLog {
    pub fn with_cap(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 0,
            dropped: 0,
            dir: DirFilter::All,
            filter: String::new(),
            filter_input: Default::default(),
            visible: VecDeque::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            follow: true,
            cap,
        }
    }

    pub fn from_frames(frames: &[Frame]) -> Self {
        let mut log = Self::default();
        for f in frames {
            log.push(f.clone());
        }
        log.follow = false;
        log
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// Total frames seen, including dropped ones.
    pub fn total(&self) -> u64 {
        self.next_id
    }

    fn first_id(&self) -> u64 {
        self.next_id - self.entries.len() as u64
    }

    pub fn get(&self, id: u64) -> Option<&LogEntry> {
        let first = self.first_id();
        id.checked_sub(first)
            .and_then(|i| self.entries.get(i as usize))
    }

    /// The `row`-th entry that passes the filters.
    pub fn visible_entry(&self, row: usize) -> Option<&LogEntry> {
        self.visible.get(row).and_then(|id| self.get(*id))
    }

    pub fn matches(&self, f: &Frame) -> bool {
        if !self.dir.allows(f.outbound) {
            return false;
        }
        let q = self.filter.trim();
        q.is_empty()
            || f.kind.to_lowercase().contains(&q.to_lowercase())
            || String::from_utf8_lossy(&f.data)
                .to_lowercase()
                .contains(&q.to_lowercase())
    }

    pub fn push(&mut self, frame: Frame) {
        let id = self.next_id;
        self.next_id += 1;
        if self.matches(&frame) {
            self.visible.push_back(id);
        }
        self.entries.push_back(LogEntry { id, frame });
        while self.entries.len() > self.cap {
            self.entries.pop_front();
            self.dropped += 1;
        }
        let first = self.first_id();
        while self.visible.front().is_some_and(|id| *id < first) {
            self.visible.pop_front();
        }
        if self.selected.is_some_and(|id| id < first) {
            self.selected = None;
        }
    }

    /// Recompute which entries pass the filters (after the direction or text changed).
    pub fn refilter(&mut self) {
        let ids: Vec<u64> = self
            .entries
            .iter()
            .filter(|e| self.matches(&e.frame))
            .map(|e| e.id)
            .collect();
        self.visible = ids.into();
    }

    pub fn set_dir(&mut self, dir: DirFilter) {
        self.dir = dir;
        self.refilter();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.visible.clear();
        self.dropped = 0;
        self.selected = None;
        // ids keep counting so a late frame never reuses one
        self.next_id = self.next_id.max(1);
    }

    pub fn counts(&self) -> (usize, usize) {
        let sent = self.entries.iter().filter(|e| e.frame.outbound).count();
        (sent, self.entries.len() - sent)
    }
}

/// `+12.345s` style offset from the start of the session.
pub fn fmt_offset(ts_ms: f64) -> String {
    if ts_ms < 1000.0 {
        format!("+{ts_ms:.0} ms")
    } else if ts_ms < 60_000.0 {
        format!("+{:.3} s", ts_ms / 1000.0)
    } else {
        let s = ts_ms / 1000.0;
        format!("+{}:{:06.3}", (s / 60.0) as u64, s % 60.0)
    }
}

/// One line for the list: whitespace collapsed, long payloads cut.
pub fn preview(f: &Frame) -> String {
    match f.kind.as_str() {
        "binary" => {
            let hex: Vec<String> = f.data.iter().take(24).map(|b| format!("{b:02x}")).collect();
            format!(
                "{}{}",
                hex.join(" "),
                if f.data.len() > 24 { " …" } else { "" }
            )
        }
        "close" => match (f.close_code, f.data.is_empty()) {
            (Some(c), true) => format!("code {c}"),
            (Some(c), false) => format!("code {c} · {}", String::from_utf8_lossy(&f.data)),
            (None, _) => String::new(),
        },
        "ping" | "pong" if f.data.is_empty() => String::new(),
        _ => {
            let text = String::from_utf8_lossy(&f.data);
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if flat.chars().count() > PREVIEW_CHARS {
                let cut: String = flat.chars().take(PREVIEW_CHARS).collect();
                format!("{cut}…")
            } else {
                flat
            }
        }
    }
}

/// What the detail pane shows for a frame: JSON pretty-printed, binary as a hex dump.
pub fn detail(f: &Frame) -> (String, Lang) {
    match f.kind.as_str() {
        "binary" => (viewer::hex_dump(&f.data, DETAIL_CAP / 4), Lang::Plain),
        "close" => (preview(f), Lang::Plain),
        _ => {
            let raw = String::from_utf8_lossy(&f.data[..f.data.len().min(DETAIL_CAP)]);
            match viewer::pretty_json(&raw) {
                Some(p) => (p, Lang::Json),
                None => (raw.into_owned(), Lang::Plain),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_runner::Frame;

    fn frame(outbound: bool, kind: &str, data: &str) -> Frame {
        Frame {
            outbound,
            kind: kind.into(),
            data: data.as_bytes().to_vec().into(),
            ts_ms: 1.0,
            close_code: None,
        }
    }

    #[test]
    fn cap_drops_oldest_and_counts_them() {
        let mut log = SessionLog::with_cap(3);
        for i in 0..5 {
            log.push(frame(false, "text", &format!("m{i}")));
        }
        assert_eq!((log.len(), log.dropped, log.total()), (3, 2, 5));
        assert_eq!(log.visible_len(), 3);
        assert_eq!(log.visible_entry(0).unwrap().frame.data, "m2");
    }

    #[test]
    fn selection_is_cleared_when_its_frame_is_dropped() {
        let mut log = SessionLog::with_cap(2);
        log.push(frame(true, "text", "a"));
        log.selected = Some(0);
        log.push(frame(true, "text", "b"));
        log.push(frame(true, "text", "c"));
        assert_eq!(log.selected, None);
    }

    #[test]
    fn direction_and_text_filters_apply_to_new_and_old_frames() {
        let mut log = SessionLog::default();
        log.push(frame(true, "text", "hello"));
        log.push(frame(false, "text", "world"));
        log.set_dir(DirFilter::Received);
        assert_eq!(log.visible_len(), 1);
        log.push(frame(true, "text", "sent later"));
        log.push(frame(false, "text", "got later"));
        assert_eq!(log.visible_len(), 2);
        log.set_dir(DirFilter::All);
        log.filter = "LATER".into();
        log.refilter();
        assert_eq!(log.visible_len(), 2);
        assert_eq!(log.counts(), (2, 2));
    }

    #[test]
    fn json_frames_are_pretty_printed_and_binary_is_hex() {
        let (text, lang) = detail(&frame(false, "text", r#"{"a":1}"#));
        assert!(text.contains('\n') && lang == Lang::Json);
        let (hex, _) = detail(&frame(false, "binary", "AB"));
        assert!(hex.contains("41 42"));
        assert_eq!(preview(&frame(false, "text", "a\n  b")), "a b");
    }

    #[test]
    fn offsets_read_naturally() {
        assert_eq!(fmt_offset(12.0), "+12 ms");
        assert_eq!(fmt_offset(1500.0), "+1.500 s");
        assert_eq!(fmt_offset(61_000.0), "+1:01.000");
    }
}
