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
}
