//! Theme switcher (`theme: select…`) and the palette's `~` theme rows. Both share one
//! preview/restore/commit path: the highlighted theme becomes the global, leaving the modal
//! in any way but Enter or a click restores the original (SPEC §17.7).

use gpui::{App, Context};

use crate::{
    envs::ConfigDir,
    theme::{Appearance, Theme, ThemeChoice, registry, theme},
    workspace::{ItemRun, ModalItem, WorkspaceView, appearance_of, fuzzy, set_theme},
};

/// Label of the row that clears the `theme` setting.
pub const FOLLOW_SYSTEM: &str = "Follow system appearance";

/// Rows of the theme picker: "Follow system appearance", then every theme in registry order.
pub fn picker_items(query: &str) -> Vec<ModalItem> {
    let system = ModalItem {
        id: String::new(),
        label: FOLLOW_SYSTEM.into(),
        detail: "One Dark / One Light".into(),
        indices: Vec::new(),
        run: ItemRun::ThemeSystem,
    };
    std::iter::once(system)
        .chain(registry().all().iter().map(|t| ModalItem {
            id: format!("theme:{}", t.name),
            label: t.name.clone(),
            detail: if t.is_dark() { "dark" } else { "light" }.into(),
            indices: Vec::new(),
            run: ItemRun::Theme(t.name.clone()),
        }))
        .filter(|i| fuzzy(query, &i.label))
        .collect()
}

impl WorkspaceView {
    /// Name of the theme that is current outside any preview.
    pub fn original_theme_name(&self, cx: &App) -> String {
        self.theme_restore
            .as_ref()
            .map_or_else(|| theme(cx).name.clone(), |t| t.name.clone())
    }

    /// Index of the current theme among the picker rows (0 = "Follow system" when unknown).
    pub fn current_theme_row(&self, cx: &App) -> usize {
        let name = self.original_theme_name(cx);
        self.modal
            .as_ref()
            .and_then(|m| {
                m.results
                    .iter()
                    .position(|i| matches!(&i.run, ItemRun::Theme(n) if *n == name))
            })
            .unwrap_or(0)
    }

    /// Makes the highlighted row's theme the global; any other row (or no row) restores the
    /// original.
    pub fn preview_highlighted_theme(&mut self, cx: &mut Context<Self>) {
        let run = self
            .modal
            .as_ref()
            .and_then(|m| m.results.get(m.selected))
            .map(|i| i.run.clone());
        let target = match run {
            Some(ItemRun::Theme(name)) => Theme::named(&name),
            Some(ItemRun::ThemeSystem) => Some(os_default(cx)),
            _ => None,
        };
        match target {
            Some(t) => {
                if self.theme_restore.is_none() {
                    self.theme_restore = Some(theme(cx).clone());
                }
                if theme(cx).name != t.name {
                    set_theme(t, cx);
                }
            }
            None => self.revert_theme_preview(cx),
        }
    }

    /// Puts the original theme back, if a preview is showing.
    pub fn revert_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(original) = self.theme_restore.take()
            && theme(cx).name != original.name
        {
            set_theme(original, cx);
        }
    }

    /// Keeps `choice` (`None` = follow the OS), writes `settings.toml` and shows a notice.
    pub fn commit_theme(&mut self, choice: Option<String>, cx: &mut Context<Self>) {
        self.theme_restore = None;
        let applied = match &choice {
            Some(name) => Theme::named(name).unwrap_or_else(|| os_default(cx)),
            None => os_default(cx),
        };
        let label = applied.name.clone();
        set_theme(applied, cx);
        cx.set_global(ThemeChoice {
            configured: choice.clone(),
        });
        let saved = cx
            .try_global::<ConfigDir>()
            .map(|d| fluxwire_format::save_theme(&d.0, choice.as_deref()));
        self.notice = Some(match saved {
            Some(Err(e)) => format!("Theme: {label} (not saved: {e})"),
            _ => format!("Theme: {label}"),
        });
    }
}

/// One Dark or One Light, by the OS appearance.
pub fn os_default(cx: &App) -> Theme {
    Theme::resolve(
        None,
        appearance_of(cx.window_appearance()) == Appearance::Dark,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::{Entity, TestAppContext, VisualTestContext};

    use super::*;
    use crate::workspace::{ModalKind, install_theme};

    fn config_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw_picker_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        tag: &str,
        configured: Option<&str>,
    ) -> (Entity<WorkspaceView>, &'a mut VisualTestContext, PathBuf) {
        let dir = config_dir(tag);
        let c = dir.clone();
        let configured = configured.map(str::to_string);
        cx.update(move |cx| {
            crate::actions::install(cx);
            cx.set_global(ConfigDir(c));
            install_theme(configured, cx);
        });
        let (view, cx) = cx.add_window_view(WorkspaceView::new);
        (view, cx, dir)
    }

    fn name(cx: &mut VisualTestContext) -> String {
        cx.update(|_, cx| theme(cx).name.clone())
    }

    fn highlighted(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> String {
        view.read_with(cx, |w, _| {
            let m = w.modal.as_ref().unwrap();
            m.results[m.selected].label.clone()
        })
    }

    fn open_picker(cx: &mut VisualTestContext) {
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("theme: select");
        cx.simulate_keystrokes("enter");
    }

    #[gpui::test]
    fn picker_lists_every_theme_and_starts_on_the_current_one(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, "list", Some("Nord"));
        open_picker(cx);
        let (kind, labels) = view.read_with(cx, |w, _| {
            (
                w.modal.as_ref().map(|m| m.kind),
                w.modal_items()
                    .into_iter()
                    .map(|i| i.label)
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(kind, Some(ModalKind::ThemePicker));
        assert_eq!(labels.len(), 54);
        assert_eq!(labels[0], FOLLOW_SYSTEM);
        assert_eq!(highlighted(&view, cx), "Nord");
        assert_eq!(name(cx), "Nord");
    }

    #[gpui::test]
    fn moving_previews_and_escape_restores(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "preview", Some("Nord"));
        open_picker(cx);
        cx.simulate_keystrokes("down");
        let moved = highlighted(&view, cx);
        assert_ne!(moved, "Nord");
        assert_eq!(name(cx), moved, "the highlighted theme is live");
        cx.simulate_keystrokes("escape");
        assert_eq!(name(cx), "Nord");
        assert!(!dir.join("settings.toml").exists());
    }

    #[gpui::test]
    fn enter_keeps_the_theme_writes_settings_and_notifies(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "commit", None);
        std::fs::write(
            dir.join("settings.toml"),
            "# mine\nallow_process_env = true\n",
        )
        .unwrap();
        open_picker(cx);
        cx.simulate_input("dracula");
        cx.simulate_keystrokes("enter");
        assert_eq!(name(cx), "Dracula");
        assert_eq!(
            view.read_with(cx, |w, _| w.notice.clone()).as_deref(),
            Some("Theme: Dracula")
        );
        let s = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(s.contains("theme = \"Dracula\"") && s.contains("# mine"));
        assert!(s.contains("allow_process_env = true"));
        assert_eq!(
            cx.update(|_, cx| cx.global::<ThemeChoice>().configured.clone())
                .as_deref(),
            Some("Dracula")
        );
    }

    #[gpui::test]
    fn reopening_and_cancelling_restores_the_theme_current_at_reopen(cx: &mut TestAppContext) {
        let (_view, cx, _) = setup(cx, "reopen", Some("Nord"));
        open_picker(cx);
        cx.simulate_input("gruvbox light hard");
        cx.simulate_keystrokes("enter");
        assert_eq!(name(cx), "Gruvbox Light Hard");
        open_picker(cx);
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("escape");
        assert_eq!(name(cx), "Gruvbox Light Hard");
    }

    #[gpui::test]
    fn no_match_restores_and_follow_system_clears_the_key(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "follow", Some("Nord"));
        std::fs::write(dir.join("settings.toml"), "theme = \"Nord\"\n").unwrap();
        open_picker(cx);
        cx.simulate_input("dracula");
        assert_eq!(name(cx), "Dracula");
        cx.simulate_input("zzzz");
        assert_eq!(view.read_with(cx, |w, _| w.modal_items().len()), 0);
        assert_eq!(
            name(cx),
            "Nord",
            "no highlighted theme restores the original"
        );
        cx.simulate_keystrokes("escape");
        open_picker(cx);
        cx.simulate_input("follow");
        cx.simulate_keystrokes("enter");
        let s = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(!s.contains("theme"), "{s}");
        assert!(matches!(name(cx).as_str(), "One Dark" | "One Light"));
        assert_eq!(
            cx.update(|_, cx| cx.global::<ThemeChoice>().configured.clone()),
            None
        );
    }

    #[gpui::test]
    fn tilde_prefix_lists_themes_previews_and_commits(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "tilde", Some("Nord"));
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("~nord");
        let labels = view.read_with(cx, |w, _| {
            w.modal_items()
                .into_iter()
                .map(|i| i.label)
                .collect::<Vec<_>>()
        });
        assert_eq!(labels, ["Theme: Nord", "Theme: Nordfox"]);
        cx.simulate_keystrokes("down");
        assert_eq!(name(cx), "Nordfox");
        cx.simulate_keystrokes("up");
        assert_eq!(name(cx), "Nord");
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("escape");
        assert_eq!(name(cx), "Nord");
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("~nord");
        cx.simulate_keystrokes("down enter");
        assert_eq!(name(cx), "Nordfox");
        let s = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(s.contains("theme = \"Nordfox\""));
    }

    #[gpui::test]
    fn themes_stay_out_of_commands_and_quick_search(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "scope", None);
        // Quick search is a Workspace-level action.
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        view.update_in(cx, |w, window, cx| w.open_workspace(&ws, window, cx));
        cx.run_until_parked();
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("nord");
        assert!(view.read_with(cx, |w, _| w.modal_items().is_empty()));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("secondary-k");
        assert!(view.read_with(cx, |w, _| w.modal.is_some()));
        cx.simulate_input("nord");
        assert!(view.read_with(cx, |w, _| w.modal_items().is_empty()));
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes("secondary-k");
        cx.simulate_input("~nord");
        assert_eq!(view.read_with(cx, |w, _| w.modal_items().len()), 2);
    }

    #[gpui::test]
    fn a_non_theme_row_restores_the_original(cx: &mut TestAppContext) {
        let (_view, cx, _) = setup(cx, "nontheme", Some("Nord"));
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("~");
        cx.simulate_keystrokes("down");
        assert_ne!(name(cx), "Nord");
        cx.simulate_input("zzzz");
        assert_eq!(name(cx), "Nord");
    }

    #[gpui::test]
    fn unknown_setting_falls_back_with_a_notice(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, "unknown", Some("No Such Theme"));
        assert!(matches!(name(cx).as_str(), "One Dark" | "One Light"));
        let notice = view.read_with(cx, |w, _| w.notice.clone()).unwrap();
        assert!(notice.starts_with("Unknown theme \"No Such Theme\" in settings.toml; using One "));
        assert_eq!(
            cx.update(|_, cx| cx.global::<ThemeChoice>().configured.clone()),
            None
        );
    }

    #[gpui::test]
    fn toggle_does_not_touch_settings(cx: &mut TestAppContext) {
        let (_view, cx, dir) = setup(cx, "toggle", Some("Nord"));
        cx.simulate_keystrokes("secondary-shift-p");
        cx.simulate_input("theme: toggle");
        cx.simulate_keystrokes("enter");
        assert_eq!(name(cx), "One Light");
        assert!(!dir.join("settings.toml").exists());
    }

    #[gpui::test]
    fn malformed_settings_is_not_overwritten_but_the_theme_applies(cx: &mut TestAppContext) {
        let (view, cx, dir) = setup(cx, "malformed", None);
        std::fs::write(dir.join("settings.toml"), "not toml ===").unwrap();
        open_picker(cx);
        cx.simulate_input("dracula");
        cx.simulate_keystrokes("enter");
        assert_eq!(name(cx), "Dracula");
        assert_eq!(
            std::fs::read_to_string(dir.join("settings.toml")).unwrap(),
            "not toml ==="
        );
        let notice = view.read_with(cx, |w, _| w.notice.clone()).unwrap();
        assert!(notice.contains("not saved"), "{notice}");
    }
}
