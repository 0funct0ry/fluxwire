//! Where the Help entries point: the deployed docs site and the GitHub issue tracker (M15).
//! Pure data, so the URLs are tested without GPUI.

use crate::about::ISSUES_URL;

pub const SITE_URL: &str = "https://0funct0ry.github.io/fluxwire";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTarget {
    Documentation,
    FormatReference,
    KeybindingsReference,
    ReportIssue,
}

impl HelpTarget {
    pub fn url(self) -> String {
        match self {
            Self::Documentation => format!("{SITE_URL}/docs/"),
            Self::FormatReference => format!("{SITE_URL}/docs/reference/flux-format/"),
            Self::KeybindingsReference => format!("{SITE_URL}/docs/reference/keybindings/"),
            Self::ReportIssue => ISSUES_URL.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentation_links_stay_under_the_deployed_site() {
        for t in [
            HelpTarget::Documentation,
            HelpTarget::FormatReference,
            HelpTarget::KeybindingsReference,
        ] {
            let u = t.url();
            assert!(
                u.starts_with("https://0funct0ry.github.io/fluxwire/docs/"),
                "{u}"
            );
            assert!(u.ends_with('/'), "{u}");
        }
    }

    #[test]
    fn report_an_issue_opens_the_tracker() {
        assert_eq!(
            HelpTarget::ReportIssue.url(),
            "https://github.com/0funct0ry/fluxwire/issues"
        );
    }

    #[test]
    fn every_link_is_distinct() {
        let all = [
            HelpTarget::Documentation,
            HelpTarget::FormatReference,
            HelpTarget::KeybindingsReference,
            HelpTarget::ReportIssue,
        ];
        let mut urls: Vec<_> = all.iter().map(|t| t.url()).collect();
        urls.sort();
        urls.dedup();
        assert_eq!(urls.len(), 4);
    }
}
