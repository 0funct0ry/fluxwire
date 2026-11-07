//! About, Updates and crash-log glue (M14). Content lives in `about.rs`, `updates_state.rs` and
//! `crash.rs`; this file shows it with the shared Confirm dialog and runs the update check.

use gpui::{Context, Window};

use crate::{
    about::{AboutInfo, ISSUES_URL, REPO_URL, SITE_URL},
    crash,
    dialogs::{Confirm, ConfirmAction, Dialog},
    envs::ConfigDir,
    updates_state::UpdateState,
    workspace::WorkspaceView,
};
use fluxwire_runner::updates::{self, UpdateStatus};

const UPDATES_TITLE: &str = "Updates";

impl WorkspaceView {
    fn app_config_dir(cx: &Context<Self>) -> Option<std::path::PathBuf> {
        cx.try_global::<ConfigDir>().map(|c| c.0.clone())
    }

    fn auto_updates_on(cx: &Context<Self>) -> bool {
        Self::app_config_dir(cx)
            .map(|d| fluxwire_format::load_settings(&d).check_for_updates)
            .unwrap_or(false)
    }

    pub fn show_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let info = AboutInfo::current();
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: "About Fluxwire".into(),
            lines: info.lines(),
            buttons: vec![
                ("Close".into(), ConfirmAction::Cancel),
                ("Website".into(), ConfirmAction::OpenUrl(SITE_URL.into())),
                ("Source".into(), ConfirmAction::OpenUrl(REPO_URL.into())),
                (
                    "Report an issue".into(),
                    ConfirmAction::OpenUrl(ISSUES_URL.into()),
                ),
            ],
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    /// The Updates dialog: current version, last result, the startup toggle and "Check now".
    pub fn show_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let auto = Self::auto_updates_on(cx);
        let mut buttons = vec![
            ("Check now".into(), ConfirmAction::CheckUpdates),
            (
                if auto {
                    "Turn off automatic checks"
                } else {
                    "Turn on automatic checks"
                }
                .into(),
                ConfirmAction::ToggleAutoUpdates,
            ),
        ];
        if let UpdateState::Done(UpdateStatus::Available(r)) = &self.updates {
            buttons.insert(
                0,
                (
                    "Open release page".into(),
                    ConfirmAction::OpenUrl(r.url.clone()),
                ),
            );
        }
        buttons.push(("Close".into(), ConfirmAction::Cancel));
        self.dialog = Some(Dialog::Confirm(Confirm {
            title: UPDATES_TITLE.into(),
            lines: self.updates.dialog_lines(env!("CARGO_PKG_VERSION"), auto),
            buttons,
        }));
        window.focus(&self.dialog_focus);
        cx.notify();
    }

    pub(crate) fn toggle_auto_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let on = Self::auto_updates_on(cx);
        if let Some(dir) = Self::app_config_dir(cx)
            && let Err(e) = fluxwire_format::save_check_for_updates(&dir, !on)
        {
            self.notice = Some(format!("Could not save settings.toml: {e}"));
        }
        self.show_updates(window, cx);
    }

    /// Runs the check. `interactive` reopens the dialog with the result; the startup check
    /// only raises a notice when a newer version exists.
    pub fn check_updates(
        &mut self,
        interactive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.updates = UpdateState::Checking;
        if interactive {
            self.show_updates(window, cx);
        }
        let net = Self::app_config_dir(cx)
            .map(|d| fluxwire_runner::net_config_from_dir(&d))
            .unwrap_or_default();
        let task = fluxwire_runner::runner()
            .handle()
            .spawn(updates::check(env!("CARGO_PKG_VERSION"), net));
        cx.spawn_in(window, async move |this, cx| {
            let status = task
                .await
                .unwrap_or_else(|e| UpdateStatus::Failed(e.to_string()));
            this.update_in(cx, |this, window, cx| {
                this.updates = UpdateState::Done(status);
                this.notice = this.updates.notice().or(this.notice.take());
                let showing =
                    matches!(&this.dialog, Some(Dialog::Confirm(c)) if c.title == UPDATES_TITLE);
                if showing {
                    this.show_updates(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_latest_crash_log(&mut self, cx: &mut Context<Self>) {
        let dir = Self::app_config_dir(cx).map(|d| crash::crash_dir(&d));
        let latest = dir.as_ref().and_then(|d| newest_log(d));
        match latest {
            Some(p) => cx.open_url(&format!("file://{}", p.display())),
            None => self.notice = Some("There is no crash log.".into()),
        }
        cx.notify();
    }

    /// Launch-time work: offer the last crash log, and check for updates when enabled.
    pub fn startup_checks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = Self::app_config_dir(cx).map(|d| crash::crash_dir(&d))
            && let Some(log) = crash::newest_unseen(&dir)
        {
            let _ = crash::mark_seen(&dir, &log);
            self.dialog = Some(Dialog::Confirm(Confirm {
                title: "Fluxwire quit unexpectedly last time".into(),
                lines: vec![
                    format!("A crash log was saved at {}.", log.display()),
                    "Logs stay on this machine; nothing is uploaded.".into(),
                ],
                buttons: vec![
                    ("Open crash log".into(), ConfirmAction::OpenPath(log)),
                    ("Dismiss".into(), ConfirmAction::Cancel),
                ],
            }));
            window.focus(&self.dialog_focus);
        }
        if Self::auto_updates_on(cx) {
            self.check_updates(false, window, cx);
        }
    }
}

/// The newest `crash-*.log` in `dir`, seen or not.
fn newest_log(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut logs: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log"))
        })
        .collect();
    logs.sort_by_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| {
                n.trim_start_matches("crash-")
                    .trim_end_matches(".log")
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0)
    });
    logs.pop()
}
