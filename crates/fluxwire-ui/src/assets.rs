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
    "icons/folder-plus.svg",
    "icons/file-plus.svg",
    "icons/logo.svg",
];

pub const THEME_ONE_DARK: &str = include_str!("../assets/themes/one-dark.json");
pub const THEME_ONE_LIGHT: &str = include_str!("../assets/themes/one-light.json");

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
