//! Feeds the MCP event buffer (M9.8) from the UI thread. [`changes`] is the pure part: it
//! compares two snapshots and names what happened; the rest forwards to
//! `fluxwire_agent::events`. Compiled only with `dev-mcp`.

use crate::state_snapshot::UiState;
use serde_json::{Value, json};

/// Events implied by the move from `old` to `new`: a `generation` event for any change, plus
/// `notice` and `run` events when those parts changed. Console lines are pushed separately
/// with their text by the caller (see [`console`]).
pub fn changes(old: Option<&UiState>, new: &UiState) -> Vec<(&'static str, Value)> {
    let mut out = vec![];
    if old == Some(new) {
        return out;
    }
    out.push(("generation", json!({"generation": new.generation})));
    let old_notice = old.and_then(|o| o.notice.as_ref());
    if new.notice.as_ref() != old_notice
        && let Some(n) = &new.notice
    {
        out.push(("notice", json!({"message": n})));
    }
    let status = |s: Option<&UiState>| {
        s.and_then(|s| s.response.as_ref())
            .map(|r| (r.sending, r.status, r.tests_passed, r.tests_failed))
    };
    if status(old) != status(Some(new))
        && let Some(r) = &new.response
    {
        out.push((
            "run",
            json!({
                "phase": if r.sending { "sending" } else { "finished" },
                "status": r.status,
                "error": r.error,
                "tests_passed": r.tests_passed,
                "tests_failed": r.tests_failed,
            }),
        ));
    }
    for r in &new.runs {
        let before = old.and_then(|o| o.runs.iter().find(|x| x.tab == r.tab));
        if before != Some(r) {
            out.push((
                "run",
                json!({
                    "folder_run": r.folder,
                    "running": r.running,
                    "done": r.done,
                    "entries": r.entries,
                }),
            ));
        }
    }
    out
}

/// Pushes the events of a snapshot change.
pub fn emit_changes(old: Option<&UiState>, new: &UiState) {
    for (kind, data) in changes(old, new) {
        fluxwire_agent::events::emit(kind, data);
    }
}

/// A new console line.
pub fn console(line: &str) {
    fluxwire_agent::events::emit("console", json!({"line": line}));
}

/// A frame of a session protocol, as a one-line description.
pub fn frame(tab: u64, desc: &str) {
    fluxwire_agent::events::emit("frame", json!({"tab": tab, "frame": desc}));
}

/// A run lifecycle step that has no snapshot change of its own (a request inside a folder run).
pub fn run(data: Value) {
    fluxwire_agent::events::emit("run", data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_snapshot::{ResponseState, RunState};

    fn base() -> UiState {
        UiState::default()
    }

    #[test]
    fn identical_snapshots_emit_nothing() {
        assert!(changes(Some(&base()), &base()).is_empty());
    }

    #[test]
    fn any_change_emits_a_generation_event() {
        let mut n = base();
        n.generation = 3;
        n.left_open = !n.left_open;
        let c = changes(Some(&base()), &n);
        assert_eq!(c[0].0, "generation");
        assert_eq!(c[0].1["generation"], 3);
    }

    #[test]
    fn a_new_notice_is_reported_once() {
        let mut n = base();
        n.notice = Some("Saved".into());
        let c = changes(Some(&base()), &n);
        assert!(
            c.iter()
                .any(|(k, d)| *k == "notice" && d["message"] == "Saved")
        );
        let mut again = n.clone();
        again.generation = 9;
        assert!(
            !changes(Some(&n), &again)
                .iter()
                .any(|(k, _)| *k == "notice")
        );
    }

    #[test]
    fn a_response_and_a_folder_run_become_run_events() {
        let mut n = base();
        n.response = Some(ResponseState {
            sending: false,
            status: Some(200),
            status_text: None,
            decoded_size: None,
            wire_size: None,
            total_ms: None,
            timings: vec![],
            error: None,
            tests_passed: 0,
            tests_failed: 0,
        });
        n.runs = vec![RunState {
            tab: "t".into(),
            folder: "payments".into(),
            running: true,
            entries: 3,
            done: 1,
        }];
        let c = changes(Some(&base()), &n);
        let runs: Vec<_> = c.iter().filter(|(k, _)| *k == "run").collect();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].1["status"], 200);
        assert_eq!(runs[1].1["done"], 1);
    }
}
