//! UI actions and input for the in-app MCP host (M9.4, ADR-009): list and invoke actions, typed
//! commands, keystrokes, text, clicks and scrolls. Every operation goes through the same
//! methods and dispatch paths the UI uses; the results carry the new `ui.state` generation.

use crate::actions::{ACTIONS, KeymapState, find_action};
use crate::workspace::WorkspaceView;
use fluxwire_agent::bridge::{UiCommand, UiRequest};
use gpui::{Context, Keystroke, Modifiers, Window};
use serde::Serialize;

#[derive(Serialize, Debug, PartialEq)]
pub struct ActionRow {
    pub name: &'static str,
    pub label: &'static str,
    pub binding: Option<String>,
    pub context: Option<&'static str>,
    pub palette: bool,
    pub enabled: bool,
}

/// The one-keystroke form of a character typed into the focused input.
pub fn char_keystroke(c: char) -> Keystroke {
    let (key, shift) = match c {
        '\n' => ("enter".to_string(), false),
        '\t' => ("tab".to_string(), false),
        ' ' => ("space".to_string(), false),
        c => (c.to_string(), c.is_uppercase()),
    };
    Keystroke {
        modifiers: Modifiers {
            shift,
            ..Modifiers::default()
        },
        key,
        key_char: match c {
            '\n' => Some("\n".into()),
            '\t' => Some("\t".into()),
            c => Some(c.to_string()),
        },
    }
}

impl WorkspaceView {
    /// Handles the M9.4 requests; returns `None` for requests owned by `dev_mcp`.
    pub(crate) fn handle_input_request(
        &mut self,
        req: UiRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Result<serde_json::Value, String>> {
        self.notice = None;
        let out = match req {
            UiRequest::ListActions => Ok(serde_json::json!({"actions": self.list_actions(window, cx)})),
            UiRequest::InvokeAction { name } => self.invoke_action(&name, window, cx),
            UiRequest::Command { command } => self.run_command(command, window, cx),
            UiRequest::Key { keys } => self.send_keys(&keys, window, cx),
            UiRequest::TypeText { text } => {
                for c in text.chars() {
                    window.dispatch_keystroke(char_keystroke(c), cx);
                }
                Ok(serde_json::json!({}))
            }
            // gpui 0.2.2 exposes no public pointer-event injection (`Window::dispatch_event`
            // returns a crate-private type). Pointer input arrives with the semantic tree (M9.5),
            // which maps element ids to the actions they trigger.
            UiRequest::Click { .. } | UiRequest::Scroll { .. } => Err(
                "pointer input is not supported by gpui 0.2.2; use ui.invoke_action, ui.command or ui.key".into(),
            ),
            UiRequest::Idle => {
                let pending = self.pending_work();
                let settled = self.last_state.as_ref() == Some(&self.ui_state(window, cx))
                    || self.last_state.as_ref().is_some_and(|l| {
                        let mut s = self.ui_state(window, cx);
                        s.generation = l.generation;
                        *l == s
                    });
                return Some(Ok(serde_json::json!({
                    "idle": !pending && settled,
                    "generation": self.generation,
                })));
            }
            _ => return None,
        };
        Some(out.map(|v| self.with_outcome(v, window, cx)))
    }

    /// True while a request is sending or a folder run is active.
    fn pending_work(&self) -> bool {
        self.scope.responses.values().any(|r| r.sending.is_some())
            || self.scope.runs.values().any(|r| r.running())
    }

    /// Adds the new generation and any notice the operation raised.
    fn with_outcome(
        &mut self,
        mut v: serde_json::Value,
        window: &Window,
        cx: &gpui::App,
    ) -> serde_json::Value {
        self.track_generation(window, cx);
        if let Some(o) = v.as_object_mut() {
            o.insert("generation".into(), self.generation.into());
            o.insert("notice".into(), self.notice.clone().into());
        }
        v
    }

    pub fn list_actions(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<ActionRow> {
        let keymap = cx.try_global::<KeymapState>().cloned().unwrap_or_default();
        ACTIONS
            .iter()
            .map(|a| ActionRow {
                name: a.name,
                label: a.label,
                binding: keymap
                    .label_for(a.name)
                    .or_else(|| a.binding.map(str::to_string)),
                context: a.context,
                palette: a.palette,
                enabled: window.is_action_available(&*(a.build)(), cx),
            })
            .collect()
    }

    pub fn invoke_action(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        let info = find_action(name).ok_or_else(|| format!("unknown action: {name}"))?;
        if !window.is_action_available(&*(info.build)(), cx) {
            return Err(format!("action {} is not enabled right now", info.name));
        }
        window.dispatch_action((info.build)(), cx);
        Ok(serde_json::json!({"invoked": info.name}))
    }

    fn send_keys(
        &mut self,
        keys: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        let parsed: Vec<Keystroke> = keys
            .split_whitespace()
            .map(|k| Keystroke::parse(k).map_err(|e| format!("bad keystroke {k:?}: {e}")))
            .collect::<Result<_, _>>()?;
        if parsed.is_empty() {
            return Err("no keystroke given".into());
        }
        for k in parsed {
            window.dispatch_keystroke(k, cx);
        }
        Ok(serde_json::json!({}))
    }

    /// Typed operations; each calls the method the UI itself uses.
    pub fn run_command(
        &mut self,
        command: UiCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        match command {
            UiCommand::OpenFile { key, permanent } => {
                self.open_request(&key, permanent, window, cx);
                match &self.notice {
                    Some(n) => Err(n.clone()),
                    None => Ok(serde_json::json!({"opened": key})),
                }
            }
            UiCommand::SetUrl { value } => {
                let key = self.active_key().ok_or("no active request")?;
                self.set_url_text(&key, &value, cx);
                Ok(serde_json::json!({"url": value}))
            }
            UiCommand::SelectTab { key } => {
                let id = self
                    .scope
                    .tabs
                    .id_for_key(&key)
                    .ok_or_else(|| format!("no open tab {key}"))?;
                self.scope.tabs.activate(id);
                cx.notify();
                Ok(serde_json::json!({"active": key}))
            }
            UiCommand::ChooseEnvironment { name } => {
                if let Some(n) = &name
                    && !self.env_names().iter().any(|e| e == n)
                {
                    return Err(format!("no environment {n}"));
                }
                self.set_active_env(name.clone(), cx);
                Ok(serde_json::json!({"environment": name}))
            }
        }
    }
}
