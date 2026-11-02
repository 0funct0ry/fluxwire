//! `OpenWorkspace`: everything that belongs to the one Workspace open in a window. Opening builds
//! a fresh value; closing calls `shutdown` and replaces it, so nothing workspace-scoped can
//! outlive the Workspace (SPEC §17.9).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::{
    envs::EnvStore,
    tabs::Tabs,
    workspace::{BottomTab, CellEdit, InlineNew, Panel, TabUi, TreeMenu},
    workspace_model::{Target, Workspace},
};

pub struct OpenWorkspace {
    /// The on-disk model; `None` while the welcome view is shown.
    pub workspace: Option<Workspace>,
    pub tabs: Tabs,
    pub tab_ui: HashMap<u64, TabUi>,
    /// Run state and response per request tab.
    pub responses: HashMap<u64, crate::response::TabResponse>,
    /// Send log shown in the bottom dock.
    pub console: Vec<String>,
    /// Tab whose send waits on the script trust prompt (SPEC §21).
    pub trust_prompt: Option<u64>,
    /// Collections trusted this session when there is no config dir to store the decision in.
    pub session_trust: HashSet<PathBuf>,
    pub left_open: bool,
    pub left_width: f32,
    pub panel: Panel,
    pub bottom_open: bool,
    pub bottom_tab: BottomTab,
    pub bottom_height: f32,
    pub split_vertical: bool,
    /// Share of the request/response area given to the editor (0.15..=0.85).
    pub split_frac: f32,
    pub envs: EnvStore,
    pub collapsed: HashSet<String>,
    /// Tab context menu: tab id and where it opened.
    pub tab_menu: Option<(u64, gpui::Point<gpui::Pixels>)>,
    /// Tab whose scroll offset `editor_scroll` currently shows.
    pub scroll_for: Option<u64>,
    pub search: fluxwire_search::Index,
    pub tree_menu: Option<TreeMenu>,
    pub inline: Option<InlineNew>,
    /// Selected tree row (workspace-relative key); target of rename/duplicate/delete/new.
    pub selected: Option<String>,
    pub cell: Option<CellEdit>,
    /// Surface that receives editor key presses.
    pub target: Target,
    pub watcher: Option<fluxwire_format::WatchHandle>,
    /// URL bar input of each request, by key; created when its tab is first drawn.
    pub url_inputs: std::cell::RefCell<HashMap<String, gpui::Entity<crate::text_input::TextInput>>>,
    /// Code editors by (request key, surface); created when first drawn.
    pub code_editors:
        std::cell::RefCell<HashMap<(String, Target), gpui::Entity<crate::code_editor::CodeEditor>>>,
    /// Environment bulk-edit editors by (document, pane: 0 `.env`, 1 `.local.env`).
    pub env_bulk_editors: std::cell::RefCell<
        HashMap<(crate::env_editor::DocKey, usize), gpui::Entity<crate::code_editor::CodeEditor>>,
    >,
    pub method_menu: bool,
    /// WebSocket composers by tab id.
    pub composers: HashMap<u64, crate::session::Composer>,
    /// WebSocket composer editors by tab id.
    pub composer_editors:
        std::cell::RefCell<HashMap<u64, gpui::Entity<crate::code_editor::CodeEditor>>>,
    /// Transcripts of WebSocket History entries opened this session.
    pub history_logs: HashMap<i64, crate::session_log::SessionLog>,
    /// Folder run tabs by tab id (SPEC §12).
    pub runs: HashMap<u64, crate::runs::RunTab>,
}

impl Default for OpenWorkspace {
    fn default() -> Self {
        Self {
            workspace: None,
            tabs: Tabs::new(),
            tab_ui: HashMap::new(),
            responses: HashMap::new(),
            console: Vec::new(),
            trust_prompt: None,
            session_trust: HashSet::new(),
            left_open: true,
            left_width: 260.0,
            panel: Panel::Workspace,
            bottom_open: false,
            bottom_tab: BottomTab::Console,
            bottom_height: 200.0,
            split_vertical: true,
            split_frac: 0.5,
            envs: EnvStore::default(),
            collapsed: HashSet::new(),
            tab_menu: None,
            scroll_for: None,
            search: fluxwire_search::Index::new(),
            tree_menu: None,
            inline: None,
            selected: None,
            cell: None,
            target: Target::Body,
            watcher: None,
            url_inputs: Default::default(),
            code_editors: Default::default(),
            env_bulk_editors: Default::default(),
            method_menu: false,
            composers: HashMap::new(),
            composer_editors: Default::default(),
            history_logs: HashMap::new(),
            runs: HashMap::new(),
        }
    }
}

impl OpenWorkspace {
    /// Number of sends, WebSocket sessions and folder runs still active.
    pub fn running(&self) -> usize {
        self.responses
            .values()
            .filter(|r| r.sending.is_some())
            .count()
            + self.runs.values().filter(|r| r.cancel.is_some()).count()
    }

    /// Cancel every send, close every WebSocket session and cancel every folder run.
    pub fn shutdown(&mut self) {
        for r in self.responses.values_mut() {
            if let Some(s) = r.sending.take() {
                s.cancel.cancel();
                s.session.close();
            }
        }
        for r in self.runs.values_mut() {
            if let Some(c) = r.cancel.take() {
                c.cancel();
            }
        }
        self.watcher = None;
    }
}

impl Drop for OpenWorkspace {
    fn drop(&mut self) {
        self.shutdown();
    }
}
