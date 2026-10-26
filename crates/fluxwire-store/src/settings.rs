use std::path::Path;

use serde::Deserialize;

/// `[history]` in `settings.toml` (SPEC §13): the smaller of the two limits wins.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    pub max_entries: usize,
    pub max_age_days: u64,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_age_days: 30,
        }
    }
}

impl HistorySettings {
    /// Read `[history]` from a `settings.toml`; a missing or malformed file yields defaults.
    pub fn load(path: &Path) -> Self {
        #[derive(Deserialize, Default)]
        struct File {
            #[serde(default)]
            history: HistorySettings,
        }
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str::<File>(&t).ok())
            .map(|f| f.history)
            .unwrap_or_default()
    }

    /// `settings.toml` in the user config dir.
    pub fn load_default() -> Self {
        directories::ProjectDirs::from("", "", "fluxwire")
            .map(|d| Self::load(&d.config_dir().join("settings.toml")))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_defaults() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("settings.toml");
        assert_eq!(HistorySettings::load(&p), HistorySettings::default());
        std::fs::write(&p, "[history]\nmax_entries = 3\n").unwrap();
        let s = HistorySettings::load(&p);
        assert_eq!((s.max_entries, s.max_age_days), (3, 30));
    }
}
