//! The set of available themes, built from (file name, source) pairs.

use super::{ThemeTokens, parse_toml_theme, parse_zed_theme};

pub const DEFAULT_DARK: &str = "One Dark";
pub const DEFAULT_LIGHT: &str = "One Light";

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: Vec<ThemeTokens>,
}

fn key(s: &str) -> String {
    s.trim().to_lowercase()
}

impl ThemeRegistry {
    /// Parses every file by extension (`.json` Zed, `.toml` microterm). Panics, naming the
    /// file, on a broken bundled theme. On a name collision the first file wins.
    pub fn bundled(files: &[(&str, &str)]) -> Self {
        let mut themes: Vec<ThemeTokens> = Vec::new();
        for (file, src) in files {
            let parsed = if file.ends_with(".json") {
                parse_zed_theme(src)
            } else if file.ends_with(".toml") {
                parse_toml_theme(src)
            } else {
                panic!("bundled theme {file}: unknown extension");
            };
            let t = parsed.unwrap_or_else(|e| panic!("bundled theme {file}: {e}"));
            if !themes.iter().any(|x| key(&x.name) == key(&t.name)) {
                themes.push(t);
            }
        }
        themes.sort_by_key(|t| key(&t.name));
        Self { themes }
    }

    pub fn all(&self) -> &[ThemeTokens] {
        &self.themes
    }

    pub fn len(&self) -> usize {
        self.themes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&ThemeTokens> {
        let k = key(name);
        self.themes.iter().find(|t| key(&t.name) == k)
    }

    /// The named theme, else One Dark / One Light for the OS appearance. `None` or an
    /// unknown name falls back.
    pub fn resolve(&self, name: Option<&str>, os_dark: bool) -> &ThemeTokens {
        name.and_then(|n| self.get(n)).unwrap_or_else(|| {
            let d = if os_dark { DEFAULT_DARK } else { DEFAULT_LIGHT };
            self.get(d).expect("default themes are bundled")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = include_str!("../../tests/themes/one-light.toml");
    const D: &str = include_str!("../../../fluxwire-ui/assets/themes/one-dark.json");
    const L: &str = include_str!("../../../fluxwire-ui/assets/themes/one-light.json");

    fn reg() -> ThemeRegistry {
        ThemeRegistry::bundled(&[("a.toml", A), ("one-dark.json", D), ("one-light.json", L)])
    }

    #[test]
    fn collisions_keep_the_first_and_names_sort() {
        let r = reg();
        assert_eq!(r.len(), 2);
        assert_eq!(r.all()[0].name, "One Dark");
        assert!(!r.get("one light").unwrap().author.is_empty());
    }

    #[test]
    fn lookup_is_case_insensitive_and_trimmed_with_fallback() {
        let r = reg();
        assert_eq!(r.get("  one DARK ").unwrap().name, "One Dark");
        assert_eq!(r.resolve(Some("Nope"), true).name, "One Dark");
        assert_eq!(r.resolve(None, false).name, "One Light");
    }

    #[test]
    fn sort_is_case_insensitive_as_written() {
        let src = |n: &str| A.replace("One Light", n);
        let (a, b, c) = (src("Zed"), src("rosé"), src("Alpha"));
        let r = ThemeRegistry::bundled(&[("1.toml", &a), ("2.toml", &b), ("3.toml", &c)]);
        let names: Vec<_> = r.all().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "rosé", "Zed"]);
    }

    #[test]
    #[should_panic(expected = "bundled theme broken.toml")]
    fn broken_theme_panics_with_the_file_name() {
        ThemeRegistry::bundled(&[("broken.toml", "name = 1")]);
    }
}
