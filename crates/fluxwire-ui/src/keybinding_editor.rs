//! The keybinding editor behind `keymap: edit`: capture a chord for an action, clear it, or reset
//! it to the default. Edits go through the `keymap.toml` writer in `fluxwire-format`, which only
//! touches the `[[bindings]]` entries it owns, and the live keymap reloads on save.

use gpui::{Context, Keystroke, Subscription};

use crate::{
    actions::{
        ACTIONS, ActionInfo, BindingRow, BindingSource, KeymapState, default_bindings, find_action,
        norm_keys,
    },
    envs::ConfigDir,
    tabs::TabKind,
    workspace::WorkspaceView,
};

/// An existing binding on the same keystroke and context as the one being assigned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub label: &'static str,
    pub keys: String,
    pub context: Option<String>,
    pub user: bool,
}

/// A row being edited. `keys` is `None` while waiting for a chord.
pub struct Pending {
    pub action: &'static str,
    pub keys: Option<String>,
    pub conflict: Option<Conflict>,
    _capture: Option<Subscription>,
}

fn short(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn is_action(info: &ActionInfo) -> impl Fn(&str) -> bool + '_ {
    move |a| find_action(a).is_some_and(|f| f.name == info.name)
}

/// The binding in `rows` that `keys` would collide with, if any.
pub fn find_conflict(
    rows: &[BindingRow],
    info: &ActionInfo,
    keys: &str,
    context: Option<&str>,
) -> Option<Conflict> {
    let nk = norm_keys(keys);
    rows.iter()
        .find(|r| {
            r.action != info.name && norm_keys(&r.keys) == nk && r.context.as_deref() == context
        })
        .map(|r| Conflict {
            label: r.label,
            keys: r.keys.clone(),
            context: r.context.clone(),
            user: r.source == BindingSource::User,
        })
}

/// `keymap.toml` text after binding `info` to `keys`, replacing its other bindings.
pub fn assign_text(
    text: &str,
    info: &ActionInfo,
    keys: &str,
    conflict: Option<&Conflict>,
) -> String {
    let mut t = fluxwire_format::remove_action_bindings(text, is_action(info));
    if let Some(c) = conflict.filter(|c| c.user) {
        t = fluxwire_format::remove_binding(&t, c.context.as_deref(), &c.keys);
    }
    fluxwire_format::set_binding(&t, info.context, keys, short(info.name))
}

/// `keymap.toml` text with every binding of `info` removed: its entries are dropped and each
/// default keystroke is unbound with a `none` entry.
pub fn clear_text(text: &str, info: &ActionInfo) -> String {
    let mut t = fluxwire_format::remove_action_bindings(text, is_action(info));
    for (a, keys) in default_bindings().filter(|(a, _)| a.name == info.name) {
        t = fluxwire_format::set_binding(&t, a.context, keys, "none");
    }
    t
}

/// `keymap.toml` text with `info` back on its default bindings.
pub fn reset_text(text: &str, info: &ActionInfo) -> String {
    let mut t = fluxwire_format::remove_action_bindings(text, is_action(info));
    for (a, keys) in default_bindings().filter(|(a, _)| a.name == info.name) {
        let nk = norm_keys(keys);
        t = fluxwire_format::remove_entries(&t, |c, k, act| {
            c == a.context
                && norm_keys(k) == nk
                && (act.trim().is_empty() || act.trim().eq_ignore_ascii_case("none"))
        });
    }
    t
}

/// A chord for `ks`, or `None` for a lone modifier press.
pub fn chord_of(ks: &Keystroke) -> Option<String> {
    let lone_modifier = matches!(
        ks.key.as_str(),
        "shift" | "control" | "alt" | "platform" | "function" | ""
    );
    (!lone_modifier).then(|| ks.unparse())
}

impl WorkspaceView {
    /// `keymap: edit`: the Keybindings tab in edit mode.
    pub fn edit_keymap(&mut self, cx: &mut Context<Self>) {
        self.keybind_editing = true;
        self.open_tab("keybindings", "Keybindings", TabKind::Keybindings, true);
        cx.notify();
    }

    fn action_info(name: &str) -> Option<&'static ActionInfo> {
        ACTIONS.iter().find(|a| a.name == name)
    }

    /// Wait for a chord for `action`; the next keystroke is consumed (Esc cancels).
    pub fn begin_capture(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let me = cx.entity();
        let sub = cx.intercept_keystrokes(move |ev, _w, cx| {
            cx.stop_propagation();
            let ks = ev.keystroke.clone();
            me.update(cx, |this, cx| this.capture_chord(&ks, cx));
        });
        self.keybind_edit = Some(Pending {
            action,
            keys: None,
            conflict: None,
            _capture: Some(sub),
        });
        cx.notify();
    }

    pub fn capture_chord(&mut self, ks: &Keystroke, cx: &mut Context<Self>) {
        let Some(p) = &self.keybind_edit else { return };
        if ks.key == "escape" {
            self.cancel_edit(cx);
            return;
        }
        if p.keys.is_some() {
            return;
        }
        let Some(keys) = chord_of(ks) else { return };
        let Some(info) = Self::action_info(p.action) else {
            return;
        };
        let rows = cx
            .try_global::<KeymapState>()
            .map(|s| s.rows.clone())
            .unwrap_or_default();
        let conflict = find_conflict(&rows, info, &keys, info.context);
        if conflict.is_none() {
            self.keybind_edit = None;
            self.apply_edit(cx, |t| assign_text(t, info, &keys, None));
            return;
        }
        if let Some(p) = &mut self.keybind_edit {
            p.keys = Some(keys);
            p.conflict = conflict;
            p._capture = None;
        }
        cx.notify();
    }

    /// Replace the conflicting binding with the captured chord.
    pub fn confirm_replace(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.keybind_edit.take() else {
            return;
        };
        if let (Some(info), Some(keys)) = (Self::action_info(p.action), p.keys) {
            self.apply_edit(cx, |t| assign_text(t, info, &keys, p.conflict.as_ref()));
        }
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.keybind_edit = None;
        cx.notify();
    }

    pub fn clear_keybinding(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if let Some(info) = Self::action_info(action) {
            self.apply_edit(cx, |t| clear_text(t, info));
        }
    }

    pub fn reset_keybinding(&mut self, action: &'static str, cx: &mut Context<Self>) {
        if let Some(info) = Self::action_info(action) {
            self.apply_edit(cx, |t| reset_text(t, info));
        }
    }

    fn apply_edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&str) -> String) {
        let Some(dir) = cx.try_global::<ConfigDir>().map(|c| c.0.clone()) else {
            self.notice = Some("No user config directory in this session.".into());
            cx.notify();
            return;
        };
        let old = std::fs::read_to_string(dir.join("keymap.toml")).unwrap_or_default();
        let new = f(&old);
        if new != old
            && let Err(e) = fluxwire_format::save_keymap(&dir, &new)
        {
            self.notice = Some(format!("Could not write keymap.toml: {e}"));
        }
        self.reload_keymap(cx);
    }

    /// Copy the open Workspace root to the clipboard.
    pub fn copy_workspace_path(&mut self, cx: &mut Context<Self>) {
        if let Some(w) = &self.scope.workspace {
            let text = w.root.display().to_string();
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            self.notice = Some("Copied workspace path.".into());
            cx.notify();
        }
    }
}
