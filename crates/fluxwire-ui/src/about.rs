//! The About dialog's content (M14): version, commit, licence and links. Pure data so it is
//! tested without GPUI; `app_dialogs.rs` shows it.

pub const REPO_URL: &str = "https://github.com/0funct0ry/fluxwire";
pub const SITE_URL: &str = "https://0funct0ry.github.io/fluxwire/";
pub const ISSUES_URL: &str = "https://github.com/0funct0ry/fluxwire/issues";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutInfo {
    pub version: String,
    /// Short commit hash the binary was built from; `None` for a build outside CI/make.
    pub commit: Option<String>,
    pub license: String,
}

impl AboutInfo {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: option_env!("FLUXWIRE_COMMIT")
                .filter(|c| !c.is_empty())
                .map(str::to_string),
            license: env!("CARGO_PKG_LICENSE").to_string(),
        }
    }

    /// Body lines of the dialog.
    pub fn lines(&self) -> Vec<String> {
        let commit = self.commit.as_deref().unwrap_or("unknown");
        vec![
            format!("Version {} (commit {commit})", self.version),
            format!(
                "Licensed under {}. No accounts, no telemetry.",
                self.license
            ),
            format!("Website: {SITE_URL}"),
            format!("Source: {REPO_URL}"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_show_version_commit_license_and_links() {
        let i = AboutInfo {
            version: "1.2.3".into(),
            commit: Some("abc1234".into()),
            license: "MIT".into(),
        };
        let l = i.lines();
        assert!(l[0].contains("1.2.3") && l[0].contains("abc1234"));
        assert!(l[1].contains("MIT"));
        assert!(l.iter().any(|x| x.contains(SITE_URL)));
        assert!(l.iter().any(|x| x.contains(REPO_URL)));
    }

    #[test]
    fn missing_commit_reads_unknown() {
        let i = AboutInfo {
            commit: None,
            ..AboutInfo::current()
        };
        assert!(i.lines()[0].contains("unknown"));
        assert_eq!(AboutInfo::current().version, env!("CARGO_PKG_VERSION"));
    }
}
