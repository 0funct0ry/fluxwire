//! Window capture prototype (ADR-009, tier 1): the window's bounds in screen coordinates and
//! `screencapture -x -R` on macOS. gpui 0.2.2 has no render-to-image, so this needs a visible
//! window and the Screen Recording permission for the parent process.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// A screen rectangle in points, with the display scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: f32,
}

impl Region {
    /// The `-R x,y,w,h` argument of `screencapture`.
    pub fn arg(&self) -> String {
        format!(
            "{},{},{},{}",
            self.x.round() as i64,
            self.y.round() as i64,
            self.w.round() as i64,
            self.h.round() as i64
        )
    }
}

/// Smallest file a real capture of a window produces; the permission-denied case writes a
/// near-empty or wallpaper-only image.
const MIN_BYTES: u64 = 2_000;

/// Captures `region` into a PNG at `out`.
pub fn capture(region: Region, out: &Path) -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("window capture is only implemented on macOS in this prototype".into());
    }
    let status = std::process::Command::new("screencapture")
        .args(["-x", "-t", "png", "-R", &region.arg()])
        .arg(out)
        .output()
        .map_err(|e| format!("could not run screencapture: {e}"))?;
    if !status.status.success() {
        return Err(permission_error(&String::from_utf8_lossy(&status.stderr)));
    }
    let len = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    if len < MIN_BYTES {
        return Err(permission_error("the capture is blank"));
    }
    Ok(out.to_path_buf())
}

fn permission_error(detail: &str) -> String {
    format!(
        "capture failed ({}). Grant Screen Recording to the app that started fluxwire (Terminal, \
         iTerm or your editor) in System Settings > Privacy & Security > Screen Recording, then \
         restart it.",
        detail.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_formats_the_screencapture_argument() {
        let r = Region {
            x: 100.4,
            y: 50.6,
            w: 1280.0,
            h: 800.0,
            scale: 2.0,
        };
        assert_eq!(r.arg(), "100,51,1280,800");
    }

    #[test]
    fn a_failed_capture_names_the_permission() {
        let e = permission_error("the capture is blank");
        assert!(e.contains("Screen Recording"), "{e}");
    }
}
