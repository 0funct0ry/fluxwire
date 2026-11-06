//! Embedded fonts, Lucide icons and themes, served to GPUI as an `AssetSource`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};

macro_rules! embed {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../assets/", $path)).as_slice())),*]
    };
}

const FILES: &[(&str, &[u8])] = embed![
    "icons/chevron-down.svg",
    "icons/chevron-right.svg",
    "icons/folder.svg",
    "icons/folder-open.svg",
    "icons/git-branch.svg",
    "icons/globe.svg",
    "icons/plus.svg",
    "icons/search.svg",
    "icons/x.svg",
    "icons/terminal.svg",
    "icons/triangle-alert.svg",
    "icons/circle-x.svg",
    "icons/file-text.svg",
    "icons/book-open.svg",
    "icons/refresh-cw.svg",
    "icons/play.svg",
    "icons/layers.svg",
    "icons/pin.svg",
    "icons/palette.svg",
    "icons/ellipsis-vertical.svg",
    "icons/ellipsis.svg",
    "icons/check.svg",
    "icons/square.svg",
    "icons/history.svg",
    "icons/copy.svg",
    "icons/eye.svg",
    "icons/pencil.svg",
    "icons/trash-2.svg",
    "icons/lock.svg",
    "icons/folder-plus.svg",
    "icons/file-plus.svg",
    "icons/logo.svg",
];

/// Bundled themes (SPEC §17.7), sorted by file name: Fluxwire's One Dark and One Light as Zed
/// JSON, the rest microterm TOML. A test fails when `assets/themes/` and this list differ.
pub const BUNDLED_THEMES: &[(&str, &str)] = &[
    (
        "ayu-dark.toml",
        include_str!("../assets/themes/ayu-dark.toml"),
    ),
    (
        "ayu-light.toml",
        include_str!("../assets/themes/ayu-light.toml"),
    ),
    (
        "ayu-mirage.toml",
        include_str!("../assets/themes/ayu-mirage.toml"),
    ),
    (
        "carbonfox.toml",
        include_str!("../assets/themes/carbonfox.toml"),
    ),
    (
        "catppuccin-frappe.toml",
        include_str!("../assets/themes/catppuccin-frappe.toml"),
    ),
    (
        "catppuccin-latte.toml",
        include_str!("../assets/themes/catppuccin-latte.toml"),
    ),
    (
        "catppuccin-macchiato.toml",
        include_str!("../assets/themes/catppuccin-macchiato.toml"),
    ),
    (
        "catppuccin-mocha.toml",
        include_str!("../assets/themes/catppuccin-mocha.toml"),
    ),
    (
        "dawnfox.toml",
        include_str!("../assets/themes/dawnfox.toml"),
    ),
    ("dayfox.toml", include_str!("../assets/themes/dayfox.toml")),
    (
        "dracula.toml",
        include_str!("../assets/themes/dracula.toml"),
    ),
    (
        "duskfox.toml",
        include_str!("../assets/themes/duskfox.toml"),
    ),
    (
        "everforest-dark.toml",
        include_str!("../assets/themes/everforest-dark.toml"),
    ),
    (
        "everforest-light.toml",
        include_str!("../assets/themes/everforest-light.toml"),
    ),
    (
        "flexoki-dark.toml",
        include_str!("../assets/themes/flexoki-dark.toml"),
    ),
    (
        "flexoki-light.toml",
        include_str!("../assets/themes/flexoki-light.toml"),
    ),
    (
        "github-dark-dimmed.toml",
        include_str!("../assets/themes/github-dark-dimmed.toml"),
    ),
    (
        "github-dark.toml",
        include_str!("../assets/themes/github-dark.toml"),
    ),
    (
        "github-light.toml",
        include_str!("../assets/themes/github-light.toml"),
    ),
    (
        "gruvbox-dark-hard.toml",
        include_str!("../assets/themes/gruvbox-dark-hard.toml"),
    ),
    (
        "gruvbox-dark-soft.toml",
        include_str!("../assets/themes/gruvbox-dark-soft.toml"),
    ),
    (
        "gruvbox-dark.toml",
        include_str!("../assets/themes/gruvbox-dark.toml"),
    ),
    (
        "gruvbox-light-hard.toml",
        include_str!("../assets/themes/gruvbox-light-hard.toml"),
    ),
    (
        "gruvbox-light-soft.toml",
        include_str!("../assets/themes/gruvbox-light-soft.toml"),
    ),
    (
        "gruvbox-light.toml",
        include_str!("../assets/themes/gruvbox-light.toml"),
    ),
    (
        "iceberg-dark.toml",
        include_str!("../assets/themes/iceberg-dark.toml"),
    ),
    (
        "iceberg-light.toml",
        include_str!("../assets/themes/iceberg-light.toml"),
    ),
    (
        "kanagawa-dragon.toml",
        include_str!("../assets/themes/kanagawa-dragon.toml"),
    ),
    (
        "kanagawa-lotus.toml",
        include_str!("../assets/themes/kanagawa-lotus.toml"),
    ),
    (
        "kanagawa-wave.toml",
        include_str!("../assets/themes/kanagawa-wave.toml"),
    ),
    (
        "light-owl.toml",
        include_str!("../assets/themes/light-owl.toml"),
    ),
    (
        "melange-dark.toml",
        include_str!("../assets/themes/melange-dark.toml"),
    ),
    (
        "melange-light.toml",
        include_str!("../assets/themes/melange-light.toml"),
    ),
    (
        "night-owl.toml",
        include_str!("../assets/themes/night-owl.toml"),
    ),
    (
        "nightfox.toml",
        include_str!("../assets/themes/nightfox.toml"),
    ),
    ("nord.toml", include_str!("../assets/themes/nord.toml")),
    (
        "nordfox.toml",
        include_str!("../assets/themes/nordfox.toml"),
    ),
    (
        "one-dark.json",
        include_str!("../assets/themes/one-dark.json"),
    ),
    (
        "one-light.json",
        include_str!("../assets/themes/one-light.json"),
    ),
    (
        "oxocarbon.toml",
        include_str!("../assets/themes/oxocarbon.toml"),
    ),
    (
        "poimandres.toml",
        include_str!("../assets/themes/poimandres.toml"),
    ),
    (
        "rose-pine-dawn.toml",
        include_str!("../assets/themes/rose-pine-dawn.toml"),
    ),
    (
        "rose-pine-moon.toml",
        include_str!("../assets/themes/rose-pine-moon.toml"),
    ),
    (
        "rose-pine.toml",
        include_str!("../assets/themes/rose-pine.toml"),
    ),
    (
        "solarized-dark.toml",
        include_str!("../assets/themes/solarized-dark.toml"),
    ),
    (
        "solarized-light.toml",
        include_str!("../assets/themes/solarized-light.toml"),
    ),
    (
        "terafox.toml",
        include_str!("../assets/themes/terafox.toml"),
    ),
    (
        "tokyo-night-day.toml",
        include_str!("../assets/themes/tokyo-night-day.toml"),
    ),
    (
        "tokyo-night-moon.toml",
        include_str!("../assets/themes/tokyo-night-moon.toml"),
    ),
    (
        "tokyo-night-storm.toml",
        include_str!("../assets/themes/tokyo-night-storm.toml"),
    ),
    (
        "tokyo-night.toml",
        include_str!("../assets/themes/tokyo-night.toml"),
    ),
    (
        "tomorrow-night.toml",
        include_str!("../assets/themes/tomorrow-night.toml"),
    ),
    (
        "tomorrow.toml",
        include_str!("../assets/themes/tomorrow.toml"),
    ),
];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexSans_400Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans_500Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans_600SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono_400Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono_400Regular_Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono_700Bold.ttf"),
];

/// UI font family (SPEC §17.7).
pub const UI_FONT: &str = "IBM Plex Sans";
/// Buffer font family. SPEC §17.7 names IBM Plex Mono; it is not bundled yet (see ADR-002),
/// so JetBrains Mono stands in until the font files are added.
pub const BUFFER_FONT: &str = "JetBrains Mono";

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

pub fn load_fonts(cx: &App) -> Result<()> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|b| Cow::Borrowed(*b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_theme_list_matches_the_directory() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/themes");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json") || n.ends_with(".toml"))
            .collect();
        on_disk.sort();
        let listed: Vec<&str> = BUNDLED_THEMES.iter().map(|(n, _)| *n).collect();
        assert_eq!(on_disk, listed);
    }
}
