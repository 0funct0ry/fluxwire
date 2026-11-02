//! A serializable view model of the running app (`ui.state`, ADR-009). Plain data built from the
//! view's own fields on the UI thread; no GPUI types leave this module.

use crate::tabs::TabKind;
use crate::workspace::{BottomTab, Panel, WorkspaceView};
use gpui::{App, Window};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    /// Increases every time anything below changes; unchanged between identical snapshots.
    pub generation: u64,
    pub workspace_path: Option<String>,
    pub workspace_name: String,
    pub tabs: Vec<TabState>,
    pub active_tab: Option<String>,
    pub left_open: bool,
    pub panel: String,
    pub bottom_open: bool,
    pub bottom_tab: String,
    pub problem_total: usize,
    pub problems: Vec<ProblemState>,
    pub console_lines: usize,
    pub response: Option<ResponseState>,
    pub session_log: Option<SessionLogState>,
    pub runs: Vec<RunState>,
    pub modal: Option<String>,
    pub dialog: Option<String>,
    pub notice: Option<String>,
    pub open_error: Option<String>,
    pub trust_prompt: Option<String>,
    pub focus: String,
    pub window: WindowState,
    pub theme: ThemeState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub preview: bool,
    pub pinned: bool,
    pub dirty: bool,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProblemState {
    pub code: String,
    pub severity: String,
    pub file: String,
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseState {
    pub sending: bool,
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub decoded_size: Option<u64>,
    pub wire_size: Option<u64>,
    pub total_ms: Option<f64>,
    pub timings: Vec<TimingState>,
    pub error: Option<String>,
    pub tests_passed: usize,
    pub tests_failed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimingState {
    pub name: String,
    pub duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionLogState {
    pub sent: usize,
    pub received: usize,
    pub dropped: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub tab: String,
    pub folder: String,
    pub running: bool,
    pub entries: usize,
    pub done: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Window origin in screen coordinates (points).
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeState {
    pub name: String,
    pub appearance: String,
}

pub fn panel_name(p: Panel) -> &'static str {
    match p {
        Panel::Workspace => "workspace",
        Panel::History => "history",
        Panel::Environments => "environments",
        Panel::Sync => "sync",
    }
}

pub fn bottom_tab_name(b: BottomTab) -> &'static str {
    match b {
        BottomTab::Console => "console",
        BottomTab::Problems => "problems",
    }
}

pub fn kind_name(k: &TabKind) -> String {
    match k {
        TabKind::Http { method } => format!("http:{}", method.to_lowercase()),
        other => format!("{other:?}").to_lowercase(),
    }
}

impl WorkspaceView {
    /// The snapshot with this view's current `generation`.
    pub fn ui_state(&self, window: &Window, cx: &App) -> UiState {
        let mut s = self.snapshot(window, cx);
        s.generation = self.generation;
        s
    }

    /// Bumps `generation` when the observable state differs from the previous snapshot. Called
    /// from `render`, so it covers every change that reaches the screen.
    pub(crate) fn track_generation(&mut self, window: &Window, cx: &App) {
        let s = self.snapshot(window, cx);
        if self.last_state.as_ref() != Some(&s) {
            self.generation += 1;
            #[cfg(feature = "dev-mcp")]
            {
                let mut cur = s.clone();
                cur.generation = self.generation;
                crate::dev_events::emit_changes(self.last_state.as_ref(), &cur);
            }
            self.last_state = Some(s);
        }
    }

    fn snapshot(&self, window: &Window, cx: &App) -> UiState {
        let sc = &self.scope;
        let active_id = sc.tabs.active_id();
        let tabs = sc
            .tabs
            .all()
            .iter()
            .map(|t| TabState {
                key: t.key.clone(),
                title: t.title.clone(),
                kind: kind_name(&t.kind),
                preview: t.preview,
                pinned: t.pinned,
                dirty: t.dirty,
                active: Some(t.id) == active_id,
            })
            .collect();
        let problems = self.all_problems();
        let theme = crate::theme::theme(cx);
        let bounds = window.bounds();
        let size = bounds.size;
        let response = active_id.and_then(|id| sc.responses.get(&id)).map(|r| {
            let result = r.result.as_deref();
            let exchange = result
                .and_then(|r| r.outcome.as_deref())
                .and_then(|o| o.exchange.as_ref());
            let tests = result.map_or(&[][..], |r| r.tests.as_slice());
            ResponseState {
                sending: r.sending.is_some(),
                status: exchange.map(|e| e.status),
                status_text: exchange.map(|e| e.status_text.clone()),
                decoded_size: exchange.map(|e| e.decoded_size),
                wire_size: exchange.map(|e| e.wire_size),
                total_ms: result
                    .and_then(|r| r.outcome.as_deref())
                    .map(|o| o.total_ms),
                timings: result
                    .and_then(|r| r.outcome.as_deref())
                    .map(|o| {
                        o.timings
                            .iter()
                            .map(|t| TimingState {
                                name: t.name.clone(),
                                duration_ms: t.duration_ms,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                error: result
                    .and_then(|r| r.error.as_ref())
                    .map(|e| format!("{e:?}")),
                tests_passed: tests.iter().filter(|t| t.passed).count(),
                tests_failed: tests.iter().filter(|t| !t.passed).count(),
            }
        });
        let session_log = active_id
            .and_then(|id| sc.responses.get(&id))
            .and_then(|r| {
                (!r.log.is_empty()).then(|| {
                    let (sent, received) = r.log.counts();
                    SessionLogState {
                        sent,
                        received,
                        dropped: r.log.dropped,
                    }
                })
            });
        let mut runs: Vec<RunState> = sc
            .runs
            .iter()
            .map(|(id, r)| RunState {
                tab: sc.tabs.get(*id).map(|t| t.key.clone()).unwrap_or_default(),
                folder: r.folder.clone(),
                running: r.running(),
                entries: r.entries.len(),
                done: r
                    .entries
                    .iter()
                    .filter(|e| matches!(e.slot, crate::runs::Slot::Done(_)))
                    .count(),
            })
            .collect();
        runs.sort_by(|a, b| a.tab.cmp(&b.tab));
        UiState {
            generation: 0,
            workspace_path: sc.workspace.as_ref().map(|w| w.root.display().to_string()),
            workspace_name: self.workspace_label(),
            tabs,
            active_tab: active_id
                .and_then(|id| sc.tabs.get(id))
                .map(|t| t.key.clone()),
            left_open: sc.left_open,
            panel: panel_name(sc.panel).into(),
            bottom_open: sc.bottom_open,
            bottom_tab: bottom_tab_name(sc.bottom_tab).into(),
            problem_total: self.problem_total(),
            problems: problems
                .iter()
                .map(|p| ProblemState {
                    code: p.code.into(),
                    severity: format!("{:?}", p.severity).to_lowercase(),
                    file: p.file.clone(),
                    line: p.line,
                    message: p.message.clone(),
                })
                .collect(),
            console_lines: sc.console.len(),
            response,
            session_log,
            runs,
            modal: self
                .modal
                .as_ref()
                .map(|m| format!("{:?}", m.kind).to_lowercase()),
            dialog: self.dialog.as_ref().map(crate::dialogs::Dialog::name),
            notice: self.notice.clone(),
            open_error: self.open_error.clone(),
            trust_prompt: sc
                .trust_prompt
                .and_then(|id| sc.tabs.get(id))
                .map(|t| t.key.clone()),
            focus: self.focus_name().into(),
            window: WindowState {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(size.width),
                height: f32::from(size.height),
                scale: window.scale_factor(),
            },
            theme: ThemeState {
                name: theme.name.clone(),
                appearance: format!("{:?}", theme.appearance).to_lowercase(),
            },
        }
    }

    fn focus_name(&self) -> &'static str {
        let named = [
            (&self.modal_focus, "modal"),
            (&self.dialog_focus, "dialog"),
            (&self.env_focus, "environment"),
            (&self.editor_focus, "editor"),
            (&self.filter_focus, "response_filter"),
            (&self.find_focus, "response_find"),
            (&self.log_focus, "session_log"),
            (&self.history_focus, "history"),
            (&self.inline_focus, "inline"),
            (&self.keybind_focus, "keybindings"),
            (&self.root_focus, "root"),
        ];
        named.into_iter().find(|(h, _)| self.is_focused(h)).map_or(
            if self.focused.is_some() {
                "other"
            } else {
                "none"
            },
            |(_, n)| n,
        )
    }
}
