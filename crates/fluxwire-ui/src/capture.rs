//! Window capture (ADR-009, M9.6). gpui 0.2.2 has no render-to-image, so capture grabs the
//! window's region of the screen with an OS tool: `screencapture -x -R` on macOS, `grim` or
//! ImageMagick `import` on Linux. It needs a visible window and, on macOS, the Screen Recording
//! permission for the parent process. The tier used is reported with every result.

use fluxwire_agent::image;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

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
    fn rounded(&self) -> (i64, i64, i64, i64) {
        (
            self.x.round() as i64,
            self.y.round() as i64,
            self.w.round() as i64,
            self.h.round() as i64,
        )
    }

    /// The `-R x,y,w,h` argument of `screencapture`.
    pub fn arg(&self) -> String {
        let (x, y, w, h) = self.rounded();
        format!("{x},{y},{w},{h}")
    }

    /// The image size in pixels a capture of this region produces: rounded points times scale.
    pub fn expected_pixels(&self) -> (u32, u32) {
        let (_, _, w, h) = self.rounded();
        (
            (w as f32 * self.scale).round() as u32,
            (h as f32 * self.scale).round() as u32,
        )
    }

    /// The part of this (window) region covered by `r`, given in points relative to the window,
    /// clipped to the window. `None` when nothing of it is inside.
    pub fn crop(&self, rx: f32, ry: f32, rw: f32, rh: f32) -> Option<Region> {
        let x0 = rx.max(0.0);
        let y0 = ry.max(0.0);
        let x1 = (rx + rw).min(self.w);
        let y1 = (ry + rh).min(self.h);
        (x1 > x0 && y1 > y0).then_some(Region {
            x: self.x + x0,
            y: self.y + y0,
            w: x1 - x0,
            h: y1 - y0,
            scale: self.scale,
        })
    }
}

/// Which mechanism took the capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    MacosScreencapture,
    LinuxGrim,
    LinuxImport,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::MacosScreencapture => "macos-screencapture",
            Tier::LinuxGrim => "linux-grim",
            Tier::LinuxImport => "linux-import",
        }
    }
}

/// Takes a PNG of a screen region.
pub trait Capture {
    fn tier(&self) -> Tier;
    fn grab(&self, region: Region, out: &Path) -> Result<(), String>;
}

struct MacScreencapture;

impl Capture for MacScreencapture {
    fn tier(&self) -> Tier {
        Tier::MacosScreencapture
    }

    fn grab(&self, region: Region, out: &Path) -> Result<(), String> {
        let o = Command::new("screencapture")
            .args(["-x", "-t", "png", "-R", &region.arg()])
            .arg(out)
            .output()
            .map_err(|e| format!("could not run screencapture: {e}"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(permission_error(&String::from_utf8_lossy(&o.stderr)))
        }
    }
}

struct LinuxGrim;

impl Capture for LinuxGrim {
    fn tier(&self) -> Tier {
        Tier::LinuxGrim
    }

    fn grab(&self, region: Region, out: &Path) -> Result<(), String> {
        let (x, y, w, h) = region.rounded();
        run_tool(
            Command::new("grim")
                .args(["-g", &format!("{x},{y} {w}x{h}")])
                .arg(out),
            "grim",
        )
    }
}

struct LinuxImport;

impl Capture for LinuxImport {
    fn tier(&self) -> Tier {
        Tier::LinuxImport
    }

    fn grab(&self, region: Region, out: &Path) -> Result<(), String> {
        let (x, y, w, h) = region.rounded();
        run_tool(
            Command::new("import")
                .args([
                    "-window",
                    "root",
                    "-crop",
                    &format!("{w}x{h}+{x}+{y}"),
                    "+repage",
                ])
                .arg(out),
            "import",
        )
    }
}

fn run_tool(cmd: &mut Command, name: &str) -> Result<(), String> {
    let o = cmd
        .output()
        .map_err(|e| format!("could not run {name}: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
}

/// Picks the capture mechanism for this platform, or says exactly what to install.
pub fn select() -> Result<Box<dyn Capture>, String> {
    pick(std::env::consts::OS, on_path)
}

fn pick(os: &str, has: impl Fn(&str) -> bool) -> Result<Box<dyn Capture>, String> {
    match os {
        "macos" => Ok(Box::new(MacScreencapture)),
        "linux" if has("grim") => Ok(Box::new(LinuxGrim)),
        "linux" if has("import") => Ok(Box::new(LinuxImport)),
        "linux" => Err(
            "no capture tool found: install `grim` (Wayland) or ImageMagick's \
                        `import` (X11, for example under xvfb), then restart fluxwire"
                .into(),
        ),
        other => Err(format!(
            "screenshots are not supported on {other}: gpui 0.2.2 cannot render to an image and \
             only macOS and Linux have an OS capture tool"
        )),
    }
}

/// A capture that passed validation.
pub struct Captured {
    pub png: Vec<u8>,
    pub tier: Tier,
    pub width: u32,
    pub height: u32,
}

/// Captures `region` with `cap`, then checks the image has the size the region implies and is
/// not blank or a permission placeholder.
pub fn capture(cap: &dyn Capture, region: Region, out: &Path) -> Result<Captured, String> {
    let _ = std::fs::remove_file(out);
    cap.grab(region, out)?;
    let png =
        std::fs::read(out).map_err(|e| permission_error(&format!("no image written: {e}")))?;
    let _ = std::fs::remove_file(out);
    validate(png, region, cap.tier())
}

pub fn validate(png: Vec<u8>, region: Region, tier: Tier) -> Result<Captured, String> {
    let img = image::decode(&png).map_err(|e| permission_error(&e))?;
    let want = region.expected_pixels();
    if (img.w, img.h) != want {
        return Err(format!(
            "capture is {}x{} pixels but the window region is {}x{} (the window may be partly \
             off screen or on a display with a different scale)",
            img.w, img.h, want.0, want.1
        ));
    }
    if image::is_blank(&img) {
        return Err(permission_error("the capture is blank"));
    }
    Ok(Captured {
        png,
        tier,
        width: img.w,
        height: img.h,
    })
}

fn permission_error(detail: &str) -> String {
    let fix = if cfg!(target_os = "macos") {
        "Grant Screen Recording to the app that started fluxwire (Terminal, iTerm or your \
         editor) in System Settings > Privacy & Security > Screen Recording, then restart it."
    } else {
        "Make sure the window is visible on the display the tool captures (under xvfb, run \
         fluxwire inside the same xvfb session)."
    };
    format!("capture failed ({}). {fix}", detail.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> Region {
        Region {
            x: 100.4,
            y: 50.6,
            w: 1280.0,
            h: 800.0,
            scale: 2.0,
        }
    }

    fn busy_png(w: u32, h: u32) -> Vec<u8> {
        let mut img = image::Rgba {
            w,
            h,
            data: [10, 10, 10, 255].repeat((w * h) as usize),
        };
        image::fill(
            &mut img,
            image::PxRect {
                x: 2,
                y: 2,
                w: w / 2,
                h: h / 2,
            },
        );
        image::encode(&img).unwrap()
    }

    #[test]
    fn region_formats_the_screencapture_argument() {
        assert_eq!(region().arg(), "100,51,1280,800");
    }

    #[test]
    fn expected_pixels_are_rounded_points_times_scale() {
        assert_eq!(region().expected_pixels(), (2560, 1600));
        let r = Region {
            scale: 1.0,
            ..region()
        };
        assert_eq!(r.expected_pixels(), (1280, 800));
    }

    #[test]
    fn crops_are_offset_by_the_window_origin_and_clipped() {
        let c = region().crop(10.0, 20.0, 100.0, 40.0).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (110.4, 70.6, 100.0, 40.0));
        let c = region().crop(1250.0, 790.0, 100.0, 100.0).unwrap();
        assert_eq!((c.w, c.h), (30.0, 10.0));
        assert!(region().crop(2000.0, 0.0, 10.0, 10.0).is_none());
    }

    #[test]
    fn a_correctly_sized_busy_image_validates() {
        let r = Region {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 30.0,
            scale: 2.0,
        };
        let c = validate(busy_png(80, 60), r, Tier::MacosScreencapture).unwrap();
        assert_eq!((c.width, c.height), (80, 60));
    }

    #[test]
    fn a_wrong_size_is_reported_with_both_sizes() {
        let r = Region {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 30.0,
            scale: 2.0,
        };
        let e = validate(busy_png(40, 30), r, Tier::LinuxGrim)
            .err()
            .unwrap();
        assert!(e.contains("40x30") && e.contains("80x60"), "{e}");
    }

    #[test]
    fn a_blank_image_names_the_permission_not_a_blank_png() {
        let r = Region {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 30.0,
            scale: 1.0,
        };
        let flat = image::encode(&image::Rgba {
            w: 40,
            h: 30,
            data: [0, 0, 0, 255].repeat(1200),
        })
        .unwrap();
        let e = validate(flat, r, Tier::MacosScreencapture).err().unwrap();
        assert!(e.contains("blank"), "{e}");
        if cfg!(target_os = "macos") {
            assert!(e.contains("Screen Recording"), "{e}");
        }
    }

    #[test]
    fn garbage_bytes_are_a_capture_failure() {
        let r = Region {
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
            scale: 1.0,
        };
        assert!(validate(b"nope".to_vec(), r, Tier::LinuxGrim).is_err());
    }

    #[test]
    fn tier_selection_follows_the_platform_and_installed_tools() {
        let t =
            |os, tools: &'static [&'static str]| pick(os, |n| tools.contains(&n)).map(|c| c.tier());
        assert_eq!(t("macos", &[]).unwrap(), Tier::MacosScreencapture);
        assert_eq!(t("linux", &["grim", "import"]).unwrap(), Tier::LinuxGrim);
        assert_eq!(t("linux", &["import"]).unwrap(), Tier::LinuxImport);
        let e = t("linux", &[]).err().unwrap();
        assert!(e.contains("grim") && e.contains("import"), "{e}");
        assert!(t("windows", &[]).err().unwrap().contains("windows"));
    }
}
