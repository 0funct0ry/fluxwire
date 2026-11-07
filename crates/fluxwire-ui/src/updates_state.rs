//! Model of the Updates dialog (M14): what the last check found and the lines to show.

use fluxwire_runner::updates::UpdateStatus;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Done(UpdateStatus),
}

impl UpdateState {
    /// The status line under the current version.
    pub fn status_line(&self) -> String {
        match self {
            UpdateState::Idle => "No check has run yet.".into(),
            UpdateState::Checking => "Checking GitHub releases…".into(),
            UpdateState::Done(UpdateStatus::UpToDate) => "You are on the latest version.".into(),
            UpdateState::Done(UpdateStatus::Available(r)) => {
                format!("Version {} is available.", r.version)
            }
            UpdateState::Done(UpdateStatus::Failed(e)) => format!("The check failed: {e}"),
        }
    }

    /// One-line notice for the status bar after a check; `None` while nothing is worth saying.
    pub fn notice(&self) -> Option<String> {
        match self {
            UpdateState::Done(UpdateStatus::Available(r)) => {
                Some(format!("Fluxwire {} is available: {}", r.version, r.url))
            }
            _ => None,
        }
    }

    pub fn dialog_lines(&self, current: &str, auto: bool) -> Vec<String> {
        vec![
            format!("Current version: {current}"),
            self.status_line(),
            format!(
                "Automatic check at startup: {}. A check sends one request to api.github.com and nothing else.",
                if auto { "on" } else { "off" }
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluxwire_runner::updates::Release;

    #[test]
    fn lines_follow_the_state() {
        assert!(UpdateState::Idle.status_line().contains("No check"));
        assert!(UpdateState::Checking.status_line().contains("Checking"));
        let avail = UpdateState::Done(UpdateStatus::Available(Release {
            version: "2.0.0".into(),
            url: "https://x".into(),
        }));
        assert!(avail.status_line().contains("2.0.0"));
        assert!(avail.notice().unwrap().contains("https://x"));
        assert!(UpdateState::Done(UpdateStatus::UpToDate).notice().is_none());
        let f = UpdateState::Done(UpdateStatus::Failed("boom".into()));
        assert!(f.status_line().contains("boom"));
    }

    #[test]
    fn dialog_states_the_automatic_setting() {
        let l = UpdateState::Idle.dialog_lines("1.0.0", false);
        assert!(l[0].contains("1.0.0") && l[2].contains("off"));
        assert!(UpdateState::Idle.dialog_lines("1.0.0", true)[2].contains(": on"));
    }
}
