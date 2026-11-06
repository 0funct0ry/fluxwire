//! Link targets in docs (SPEC §16): relative `.flux.md` / `.md` links navigate inside the app.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A collection-relative path (normalised, `/` separated) with an optional `#fragment`.
    Doc {
        path: String,
        fragment: Option<String>,
    },
    /// `#fragment` within the current file.
    Anchor(String),
    /// `http`, `https` or `mailto`: opened in the browser.
    External(String),
    /// Anything else (unsafe schemes, non-doc relative files).
    Other(String),
}

/// Resolves `href` found in the file `from` (collection-relative, `/` separated).
pub fn resolve(from: &str, href: &str) -> Link {
    let href = href.trim();
    if let Some(f) = href.strip_prefix('#') {
        return Link::Anchor(f.to_string());
    }
    let lower = href.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return Link::External(href.to_string());
    }
    if href.contains(':') || href.starts_with("//") {
        return Link::Other(href.to_string());
    }
    let (target, fragment) = match href.split_once('#') {
        Some((t, f)) => (t, Some(f.to_string())),
        None => (href, None),
    };
    if !(target.ends_with(".flux.md") || target.ends_with(".md")) {
        return Link::Other(href.to_string());
    }
    match normalise(from, target) {
        Some(path) => Link::Doc { path, fragment },
        None => Link::Other(href.to_string()),
    }
}

/// `target` joined to the directory of `from`, with `.` and `..` folded; `None` if it escapes
/// the collection.
pub fn normalise(from: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        let mut p: Vec<&str> = from.split('/').collect();
        p.pop();
        p
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_links_resolve_against_the_file() {
        assert_eq!(
            resolve("payments/create.flux.md", "../refunds/folder.flux.md"),
            Link::Doc {
                path: "refunds/folder.flux.md".into(),
                fragment: None
            }
        );
        assert_eq!(
            resolve("payments/create.flux.md", "get.flux.md#notes"),
            Link::Doc {
                path: "payments/get.flux.md".into(),
                fragment: Some("notes".into())
            }
        );
    }

    #[test]
    fn external_anchor_and_unsafe() {
        assert!(matches!(
            resolve("a.flux.md", "https://e.com"),
            Link::External(_)
        ));
        assert_eq!(resolve("a.flux.md", "#x"), Link::Anchor("x".into()));
        assert!(matches!(
            resolve("a.flux.md", "javascript:alert(1)"),
            Link::Other(_)
        ));
        assert!(matches!(resolve("a.flux.md", "../../x.md"), Link::Other(_)));
        assert!(matches!(resolve("a.flux.md", "img.png"), Link::Other(_)));
    }
}
