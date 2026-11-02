//! The event feed (M9.8): a bounded replay buffer of what happens in the app (runs, frames,
//! notices, console lines, state generation changes). The UI pushes into the process-wide
//! buffer; `ui.events` reads it, and the server forwards new events to the client as MCP
//! logging notifications, so an agent can wait on events instead of polling `ui.state`.

use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Events kept for replay; older ones are dropped.
pub const CAPACITY: usize = 1024;

/// Kinds pushed by the app.
pub const KINDS: &[&str] = &["run", "frame", "notice", "console", "generation"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Event {
    /// Increases by one per event for the life of the process; 1 is the first.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    pub kind: String,
    pub data: Value,
}

#[derive(Default)]
struct Inner {
    events: VecDeque<Event>,
    next: u64,
}

/// A replay buffer with blocking reads.
#[derive(Default)]
pub struct EventBuffer {
    inner: Mutex<Inner>,
    cv: Condvar,
    cap: usize,
}

impl EventBuffer {
    pub fn new(cap: usize) -> Self {
        EventBuffer {
            cap,
            ..Default::default()
        }
    }

    pub fn push(&self, kind: &str, data: Value) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.next += 1;
        let seq = g.next;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        g.events.push_back(Event {
            seq,
            ts,
            kind: kind.into(),
            data,
        });
        while g.events.len() > self.cap.max(1) {
            g.events.pop_front();
        }
        drop(g);
        self.cv.notify_all();
        seq
    }

    /// The seq of the newest event, 0 when none was pushed yet.
    pub fn latest(&self) -> u64 {
        self.inner.lock().unwrap().next
    }

    /// Events with `seq > since`, filtered by `kinds` (empty keeps all).
    pub fn since(&self, since: u64, kinds: &[String]) -> Vec<Event> {
        let g = self.inner.lock().unwrap();
        g.events
            .iter()
            .filter(|e| e.seq > since && (kinds.is_empty() || kinds.contains(&e.kind)))
            .cloned()
            .collect()
    }

    /// Like [`since`](Self::since), but waits up to `wait` for the first match.
    pub fn wait(&self, since: u64, kinds: &[String], wait: Duration) -> Vec<Event> {
        let end = Instant::now() + wait;
        let mut g = self.inner.lock().unwrap();
        loop {
            let hit: Vec<Event> = g
                .events
                .iter()
                .filter(|e| e.seq > since && (kinds.is_empty() || kinds.contains(&e.kind)))
                .cloned()
                .collect();
            let now = Instant::now();
            if !hit.is_empty() || now >= end {
                return hit;
            }
            g = self.cv.wait_timeout(g, end - now).unwrap().0;
        }
    }

    /// True when events before `since` were already dropped, so a replay has a gap.
    pub fn has_gap(&self, since: u64) -> bool {
        let g = self.inner.lock().unwrap();
        g.events.front().is_some_and(|e| e.seq > since + 1)
    }
}

/// The process-wide buffer the UI pushes into.
pub fn global() -> &'static Arc<EventBuffer> {
    static B: OnceLock<Arc<EventBuffer>> = OnceLock::new();
    B.get_or_init(|| Arc::new(EventBuffer::new(CAPACITY)))
}

/// Pushes onto the global buffer.
pub fn emit(kind: &str, data: Value) {
    global().push(kind, data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn seq_increases_and_since_replays() {
        let b = EventBuffer::new(8);
        assert_eq!(b.latest(), 0);
        b.push("run", json!({"a": 1}));
        let s = b.push("notice", json!("hi"));
        assert_eq!(s, 2);
        assert_eq!(b.since(0, &[]).len(), 2);
        let after = b.since(1, &[]);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].kind, "notice");
    }

    #[test]
    fn kinds_filter() {
        let b = EventBuffer::new(8);
        b.push("run", json!(1));
        b.push("frame", json!(2));
        let v = b.since(0, &["frame".to_string()]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].data, json!(2));
    }

    #[test]
    fn the_ring_drops_the_oldest_and_reports_the_gap() {
        let b = EventBuffer::new(3);
        for i in 0..5 {
            b.push("console", json!(i));
        }
        let v = b.since(0, &[]);
        assert_eq!(v.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4, 5]);
        assert!(b.has_gap(0));
        assert!(!b.has_gap(2));
    }

    #[test]
    fn wait_returns_on_a_push_and_times_out_empty() {
        let b = Arc::new(EventBuffer::new(8));
        assert!(b.wait(0, &[], Duration::from_millis(20)).is_empty());
        let b2 = b.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            b2.push("run", json!("done"));
        });
        let v = b.wait(0, &[], Duration::from_secs(5));
        t.join().unwrap();
        assert_eq!(v.len(), 1);
    }
}
