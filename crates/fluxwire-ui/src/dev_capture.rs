//! `ui.screenshot` support on the UI thread (M9.6, ADR-009): activate, capture, set theme and
//! resize. Masking and image content are assembled by the tool layer in `fluxwire-agent`.

use crate::capture::{self, Region};
use crate::dev_tree::Record;
use crate::theme::{Theme, ThemeChoice, theme};
use crate::theme_picker::os_default;
use crate::workspace::{WorkspaceView, set_theme};
use fluxwire_agent::image;
use gpui::{Context, Window, px, size};

/// Labels of elements whose text changes between captures.
const VOLATILE_LABELS: [&str; 2] = ["timings", "clock"];

impl WorkspaceView {
    pub(crate) fn activate_for_capture(&mut self, window: &mut Window) -> serde_json::Value {
        window.activate_window();
        serde_json::json!({"activated": true})
    }

    /// The window's screen region in points, with the display scale.
    fn window_region(window: &Window) -> Region {
        let b = window.bounds();
        Region {
            x: f32::from(b.origin.x),
            y: f32::from(b.origin.y),
            w: f32::from(b.size.width),
            h: f32::from(b.size.height),
            scale: window.scale_factor(),
        }
    }

    pub(crate) fn capture_request(
        &mut self,
        element: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        let win = Self::window_region(window);
        let frame = crate::dev_id::last_frame(cx);
        let region = match &element {
            None => win,
            Some(id) => {
                let r = frame
                    .records
                    .iter()
                    .find(|r| &r.id == id)
                    .ok_or_else(|| format!("no element with id {id:?}"))?;
                let v = r.bounds.intersect(&r.clip);
                win.crop(v.x, v.y, v.w, v.h)
                    .ok_or_else(|| format!("element {id:?} is not visible"))?
            }
        };
        let cap = capture::select()?;
        let out = std::env::temp_dir().join(format!("fluxwire-capture-{}.png", std::process::id()));
        let got = capture::capture(cap.as_ref(), region, &out)?;
        let (ox, oy) = (region.x - win.x, region.y - win.y);
        let volatile: Vec<_> = frame
            .records
            .iter()
            .filter(|r| VOLATILE_LABELS.contains(&r.label.as_str()))
            .filter_map(|r| volatile_rect(r, ox, oy))
            .collect();
        let state = serde_json::to_value(self.ui_state(window, cx)).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({
            "png": image::to_base64(&got.png),
            "tier": got.tier.name(),
            "region": region,
            "scale": region.scale,
            "width_px": got.width,
            "height_px": got.height,
            "volatile": volatile,
            "state": state,
        }))
    }

    pub(crate) fn set_theme_request(
        &mut self,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<serde_json::Value, String> {
        let t = match name {
            Some(n) => Theme::named(&n)
                .or_else(|| Theme::named(&n.replace(['-', '_'], " ")))
                .ok_or_else(|| format!("unknown theme {n:?}"))?,
            None => match cx
                .try_global::<ThemeChoice>()
                .and_then(|c| c.configured.clone())
            {
                Some(c) => Theme::named(&c).unwrap_or_else(|| os_default(cx)),
                None => os_default(cx),
            },
        };
        self.theme_restore = None;
        set_theme(t, cx);
        Ok(serde_json::json!({"theme": theme(cx).name}))
    }

    pub(crate) fn resize_request(
        &mut self,
        width: f32,
        height: f32,
        window: &mut Window,
    ) -> Result<serde_json::Value, String> {
        window.resize(size(px(width), px(height)));
        Ok(serde_json::json!({"requested": {"width": width, "height": height}}))
    }
}

/// A volatile element's visible rectangle relative to the captured image, in points.
fn volatile_rect(r: &Record, ox: f32, oy: f32) -> Option<serde_json::Value> {
    let v = r.bounds.intersect(&r.clip);
    (!v.is_empty()).then(
        || serde_json::json!({"label": r.label, "x": v.x - ox, "y": v.y - oy, "w": v.w, "h": v.h}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_tree::{Rect, Role};

    fn rec(label: &str, b: Rect, clip: Rect) -> Record {
        Record {
            id: "x".into(),
            role: Role::Text,
            label: label.into(),
            enabled: true,
            bounds: b,
            clip,
            focused: false,
            parent: None,
        }
    }

    #[test]
    fn volatile_rects_are_clipped_and_relative_to_the_crop() {
        let r = rec(
            "timings",
            Rect::new(100.0, 50.0, 80.0, 20.0),
            Rect::new(0.0, 0.0, 150.0, 100.0),
        );
        let v = volatile_rect(&r, 90.0, 40.0).unwrap();
        assert_eq!(v["x"], 10.0);
        assert_eq!(v["y"], 10.0);
        assert_eq!(v["w"], 50.0);
    }

    #[test]
    fn a_clipped_away_element_has_no_rect() {
        let r = rec(
            "timings",
            Rect::new(500.0, 0.0, 10.0, 10.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        assert!(volatile_rect(&r, 0.0, 0.0).is_none());
    }
}
