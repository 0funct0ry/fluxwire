//! The slice of `settings.toml` (user config dir, SPEC §19) that variables and cookies need.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct UserSettings {
    /// Allow `{{process.env.X}}` in the GUI (always on in the CLI).
    #[serde(default)]
    pub allow_process_env: bool,
    /// Store cookie jars in the data dir between runs (SPEC §9.3).
    #[serde(default)]
    pub persist_cookies: bool,
    /// Name of the colour theme; absent follows the OS appearance (SPEC §17.7).
    #[serde(default)]
    pub theme: Option<String>,
    /// Global variables, the lowest precedence layer (SPEC §8.2).
    #[serde(default)]
    pub globals: BTreeMap<String, String>,
}

pub fn parse_settings(text: &str) -> UserSettings {
    toml::from_str(text).unwrap_or_default()
}

/// Missing or malformed files yield the defaults.
pub fn load_settings(config_dir: &Path) -> UserSettings {
    std::fs::read_to_string(config_dir.join("settings.toml"))
        .map(|t| parse_settings(&t))
        .unwrap_or_default()
}

/// Sets (`Some`) or removes (`None`) the top-level `theme` key of `settings.toml`, leaving
/// every other byte alone. Creates the file and directory when missing; a file that does not
/// parse is left untouched and reported. Atomic (temp file, then rename).
pub fn save_theme(config_dir: &Path, theme: Option<&str>) -> std::io::Result<()> {
    use toml_edit::{DocumentMut, value};
    let path = config_dir.join("settings.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let mut doc: DocumentMut = text.parse().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("settings.toml does not parse, not changed: {e}"),
        )
    })?;
    match theme {
        Some(name) => doc["theme"] = value(name),
        None => {
            doc.remove("theme");
        }
    }
    let out = doc.to_string();
    std::fs::create_dir_all(config_dir)?;
    let tmp = config_dir.join("settings.toml.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_keys_and_ignores_the_rest() {
        let s = parse_settings("allow_process_env = true\nfoo = 1\n[globals]\nregion = \"eu\"\n");
        assert!(s.allow_process_env && !s.persist_cookies);
        assert_eq!(s.globals["region"], "eu");
        assert_eq!(parse_settings("not toml ==="), UserSettings::default());
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fw_settings_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn read(d: &Path) -> String {
        std::fs::read_to_string(d.join("settings.toml")).unwrap()
    }

    #[test]
    fn parses_theme_key() {
        assert_eq!(
            parse_settings("theme = \"Nord\"").theme.as_deref(),
            Some("Nord")
        );
        assert_eq!(parse_settings("").theme, None);
    }

    #[test]
    fn save_theme_creates_file_and_directory() {
        let d = dir("create").join("nested");
        save_theme(&d, Some("Nord")).unwrap();
        assert_eq!(load_settings(&d).theme.as_deref(), Some("Nord"));
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn save_theme_inserts_replaces_and_removes_keeping_the_rest() {
        let d = dir("edit");
        std::fs::create_dir_all(&d).unwrap();
        let src = "# my settings\nallow_process_env = true\n\n# vars\n[globals]\nregion = \"eu\" # where\n";
        std::fs::write(d.join("settings.toml"), src).unwrap();
        save_theme(&d, Some("Nord")).unwrap();
        let s = read(&d);
        assert_eq!(load_settings(&d).theme.as_deref(), Some("Nord"));
        assert!(s.contains("# my settings") && s.contains("# vars") && s.contains("# where"));
        assert_eq!(load_settings(&d).globals["region"], "eu");
        save_theme(&d, Some("Dracula")).unwrap();
        assert_eq!(load_settings(&d).theme.as_deref(), Some("Dracula"));
        assert_eq!(read(&d).matches("theme").count(), 1);
        save_theme(&d, None).unwrap();
        assert_eq!(read(&d), src);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_theme_refuses_malformed_input() {
        let d = dir("bad");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("settings.toml"), "not toml ===").unwrap();
        assert!(save_theme(&d, Some("Nord")).is_err());
        assert_eq!(read(&d), "not toml ===");
        let _ = std::fs::remove_dir_all(&d);
    }
}
