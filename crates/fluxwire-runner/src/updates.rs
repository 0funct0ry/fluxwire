//! The update check (SPEC §21, M14): one GET of the latest GitHub release, compared with the
//! running version. Runs only when the user asks ("Check now") or has turned
//! `check_for_updates` on. Nothing is sent but the request itself.

use serde_json::Value;

/// Where releases are listed.
pub const RELEASES_API: &str = "https://api.github.com/repos/0funct0ry/fluxwire/releases/latest";

/// The newest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Version without the leading `v`.
    pub version: String,
    pub url: String,
}

/// Result of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available(Release),
    Failed(String),
}

/// Reads `tag_name` and `html_url` from a GitHub "latest release" body; drafts and
/// prereleases are ignored.
pub fn parse_latest(json: &str) -> Result<Release, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Not valid JSON: {e}"))?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return Err("The latest release is not a stable release.".into());
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or("The response has no tag_name.")?;
    let version = tag.trim_start_matches('v').to_string();
    parse_version(&version).ok_or_else(|| format!("`{tag}` is not a version number."))?;
    let url = v["html_url"]
        .as_str()
        .unwrap_or("https://github.com/0funct0ry/fluxwire/releases")
        .to_string();
    Ok(Release { version, url })
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    it.next().is_none().then_some((a, b, c))
}

/// True when `latest` is a higher `X.Y.Z` than `current` (a leading `v` is ignored).
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (
        parse_version(current.trim_start_matches('v')),
        parse_version(latest.trim_start_matches('v')),
    ) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Compares a response body with `current`.
pub fn evaluate(current: &str, status: u16, body: &str) -> UpdateStatus {
    if status != 200 {
        return UpdateStatus::Failed(format!("GitHub answered {status}."));
    }
    match parse_latest(body) {
        Ok(r) if is_newer(current, &r.version) => UpdateStatus::Available(r),
        Ok(_) => UpdateStatus::UpToDate,
        Err(e) => UpdateStatus::Failed(e),
    }
}

/// Queries GitHub for the latest release.
#[cfg(feature = "http")]
pub async fn check(current: &str, net: fluxwire_protocol::NetConfig) -> UpdateStatus {
    let ua = format!("fluxwire/{current}");
    match fluxwire_http::get_text(RELEASES_API, &ua, net).await {
        Ok((status, body)) => evaluate(current, status, &body),
        Err(e) => UpdateStatus::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison() {
        assert!(is_newer("1.0.0", "v1.0.1"));
        assert!(is_newer("0.9.9", "1.0.0"));
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "garbage"));
        assert!(!is_newer("1.0.0", "1.0"));
    }

    #[test]
    fn parses_release_json() {
        let r = parse_latest(r#"{"tag_name":"v1.2.3","html_url":"https://x/r"}"#).unwrap();
        assert_eq!(r.version, "1.2.3");
        assert_eq!(r.url, "https://x/r");
        assert!(parse_latest(r#"{"tag_name":"v1.2.3","draft":true}"#).is_err());
        assert!(parse_latest(r#"{"tag_name":"v1.2.3","prerelease":true}"#).is_err());
        assert!(parse_latest(r#"{"tag_name":"nightly"}"#).is_err());
        assert!(parse_latest("nope").is_err());
    }

    #[test]
    fn evaluates_statuses() {
        let body = r#"{"tag_name":"v2.0.0","html_url":"u"}"#;
        assert!(matches!(
            evaluate("1.0.0", 200, body),
            UpdateStatus::Available(r) if r.version == "2.0.0"
        ));
        assert_eq!(evaluate("2.0.0", 200, body), UpdateStatus::UpToDate);
        assert!(matches!(
            evaluate("1.0.0", 403, ""),
            UpdateStatus::Failed(_)
        ));
    }
}
