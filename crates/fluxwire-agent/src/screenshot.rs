//! `ui.screenshot` orchestration (M9.6): apply a state, wait for the UI to settle, capture,
//! mask volatile text and attach the image. Everything goes through the [`UiBridge`], so the
//! steps run the same code paths as the individual `ui.*` tools, and tests use a stub bridge.

use crate::UiBridge;
use crate::bridge::UiRequest;
use crate::image;
use crate::server::IMAGES_KEY;
use crate::state::{self, Cell, Mask, Size, StateFile, StateSpec, Step};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Waits used while settling; tests use [`Pace::instant`].
#[derive(Clone, Copy)]
pub struct Pace {
    /// After bringing the window to the front.
    pub activate: Duration,
    /// Between polls.
    pub poll: Duration,
    pub timeout: Duration,
}

impl Pace {
    pub fn normal() -> Self {
        Pace {
            activate: Duration::from_millis(250),
            poll: Duration::from_millis(40),
            timeout: Duration::from_secs(10),
        }
    }

    pub fn instant() -> Self {
        Pace {
            activate: Duration::ZERO,
            poll: Duration::ZERO,
            timeout: Duration::from_millis(200),
        }
    }
}

/// Everything a capture needs besides the bridge.
pub struct Env<'a> {
    pub ui: &'a dyn UiBridge,
    pub pace: Pace,
    /// Directories searched for `fixtures/ui-states`.
    pub search: Vec<PathBuf>,
}

fn call(ui: &dyn UiBridge, req: UiRequest) -> Result<Value, String> {
    ui.call(req)
}

fn poll(
    env: &Env,
    what: &str,
    timeout: Option<Duration>,
    req: UiRequest,
    ok: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let end = Instant::now() + timeout.unwrap_or(env.pace.timeout);
    loop {
        let v = call(env.ui, req.clone())?;
        if ok(&v) {
            return Ok(v);
        }
        if Instant::now() >= end {
            return Err(format!("timed out waiting for {what}"));
        }
        std::thread::sleep(env.pace.poll);
    }
}

/// Waits for idle on two consecutive polls with the same generation, so a frame that is still
/// being laid out (or a late notice) is not captured half done.
pub fn wait_stable(env: &Env) -> Result<(), String> {
    let end = Instant::now() + env.pace.timeout;
    let mut last: Option<u64> = None;
    loop {
        let v = call(env.ui, UiRequest::Idle)?;
        let gen_ = v["generation"].as_u64();
        if v["idle"] == true && last.is_some() && last == gen_ {
            return Ok(());
        }
        last = if v["idle"] == true { gen_ } else { None };
        if Instant::now() >= end {
            return Err("timed out waiting for the UI to become idle".into());
        }
        std::thread::sleep(env.pace.poll);
    }
}

/// Applies one step through the bridge.
pub fn run_step(env: &Env, step: &Step) -> Result<(), String> {
    let ui = env.ui;
    match step {
        Step::Open { key, permanent } => call(
            ui,
            UiRequest::OpenRequest {
                key: key.clone(),
                permanent: *permanent,
            },
        )
        .map(drop),
        Step::Invoke { name } => call(ui, UiRequest::InvokeAction { name: name.clone() }).map(drop),
        Step::Key { keys } => call(ui, UiRequest::Key { keys: keys.clone() }).map(drop),
        Step::Type { text } => call(ui, UiRequest::TypeText { text: text.clone() }).map(drop),
        Step::Click { id } => call(ui, UiRequest::ClickElement { id: id.clone() }).map(drop),
        Step::Command { command } => call(
            ui,
            UiRequest::Command {
                command: command.clone(),
            },
        )
        .map(drop),
        Step::SetTheme { name } => call(
            ui,
            UiRequest::SetTheme {
                name: Some(name.clone()),
            },
        )
        .map(drop),
        Step::Resize { size } => resize(env, Size::parse(size)?),
        Step::WaitFor {
            path,
            equals,
            timeout_ms,
        } => {
            let (p, want) = (path.clone(), equals.clone());
            poll(
                env,
                &format!("{path} to match"),
                timeout_ms.map(Duration::from_millis),
                UiRequest::State,
                move |v| match (v.pointer(&p), &want) {
                    (Some(got), Some(w)) => got == w,
                    (Some(got), None) => !got.is_null(),
                    _ => false,
                },
            )
            .map(drop)
        }
        Step::WaitIdle => wait_stable(env),
    }
}

/// Resizes and waits until the window reports the new size.
pub fn resize(env: &Env, size: Size) -> Result<(), String> {
    call(
        env.ui,
        UiRequest::Resize {
            width: size.w,
            height: size.h,
        },
    )?;
    poll(
        env,
        &format!("the window to become {}", size.label()),
        None,
        UiRequest::State,
        |v| {
            let w = v["window"]["width"].as_f64().unwrap_or(0.0);
            let h = v["window"]["height"].as_f64().unwrap_or(0.0);
            (w - f64::from(size.w)).abs() < 1.5 && (h - f64::from(size.h)).abs() < 1.5
        },
    )
    .map(drop)
}

pub fn run_steps(env: &Env, steps: &[Step]) -> Result<(), String> {
    for (i, s) in steps.iter().enumerate() {
        run_step(env, s).map_err(|e| format!("step {} ({}): {e}", i + 1, step_name(s)))?;
    }
    Ok(())
}

pub fn step_name(s: &Step) -> &'static str {
    match s {
        Step::Open { .. } => "open",
        Step::Invoke { .. } => "invoke",
        Step::Key { .. } => "key",
        Step::Type { .. } => "type",
        Step::Click { .. } => "click",
        Step::Command { .. } => "command",
        Step::SetTheme { .. } => "set_theme",
        Step::Resize { .. } => "resize",
        Step::WaitFor { .. } => "wait_for",
        Step::WaitIdle => "wait_idle",
    }
}

/// The steps of a state spec. A named state first opens its Workspace when the app has a
/// different one open (a state without `workspace` leaves the Workspace alone).
pub fn resolve(env: &Env, spec: &StateSpec) -> Result<Vec<Step>, String> {
    match spec {
        StateSpec::Steps(s) => Ok(s.clone()),
        StateSpec::Named(name) => {
            let dir = env
                .search
                .iter()
                .find_map(|d| state::find_state_dir(d))
                .ok_or("no fixtures/ui-states directory found; set FLUXWIRE_UI_STATES")?;
            let file = state::load_named(&dir, name)?;
            open_workspace_for(env, &dir, &file)?;
            Ok(file.all_steps())
        }
    }
}

fn open_workspace_for(env: &Env, dir: &Path, file: &StateFile) -> Result<(), String> {
    let Some(ws) = &file.workspace else {
        return Ok(());
    };
    // `dir` is `<repo>/fixtures/ui-states`.
    let repo = dir.parent().and_then(Path::parent).unwrap_or(dir);
    let path = if Path::new(ws).is_absolute() {
        PathBuf::from(ws)
    } else {
        repo.join(ws)
    };
    let path = path.canonicalize().unwrap_or(path);
    let cur = call(env.ui, UiRequest::State)?;
    let same = cur["workspace_path"]
        .as_str()
        .map(|p| Path::new(p).canonicalize().unwrap_or_else(|_| p.into()) == path)
        .unwrap_or(false);
    if !same {
        call(
            env.ui,
            UiRequest::OpenWorkspace {
                path: path.to_string_lossy().into_owned(),
            },
        )?;
    }
    Ok(())
}

/// Captures the window (or `element`) now: settle, capture, mask, attach.
pub fn capture_now(
    env: &Env,
    element: Option<&str>,
    masks: &[Mask],
    label: Option<&str>,
) -> Result<Value, String> {
    call(env.ui, UiRequest::Activate)?;
    std::thread::sleep(env.pace.activate);
    wait_stable(env)?;
    let mut v = call(
        env.ui,
        UiRequest::Capture {
            element: element.map(String::from),
        },
    )?;
    let png = image::from_base64(v["png"].as_str().ok_or("capture returned no image")?)?;
    let mut img = image::decode(&png)?;
    let scale = v["scale"].as_f64().unwrap_or(1.0) as f32;
    let mut rects = vec![];
    for m in masks {
        match m {
            Mask::Named(n) => {
                for r in v["volatile"].as_array().into_iter().flatten() {
                    if r["label"] == n.as_str() {
                        rects.push(rect_of(r));
                    }
                }
            }
            Mask::Rect { x, y, w, h } => rects.push((*x, *y, *w, *h)),
        }
    }
    let mut masked = 0;
    for (x, y, w, h) in rects {
        if let Some(px) = image::to_px(x, y, w, h, scale, img.w, img.h) {
            image::fill(&mut img, px);
            masked += 1;
        }
    }
    let out = image::encode(&img)?;
    let o = v.as_object_mut().ok_or("capture returned no object")?;
    o.remove("png");
    o.remove("volatile");
    o.insert("masked".into(), masked.into());
    o.insert("size".into(), json!({"width": img.w, "height": img.h}));
    let mut image = json!({"mime": "image/png", "data": image::to_base64(&out)});
    if let Some(l) = label {
        image["label"] = l.into();
    }
    o.insert(IMAGES_KEY.into(), json!([image]));
    Ok(v)
}

fn rect_of(r: &Value) -> (f32, f32, f32, f32) {
    let f = |k: &str| r[k].as_f64().unwrap_or(0.0) as f32;
    (f("x"), f("y"), f("w"), f("h"))
}

/// `ui.screenshot`: apply `state` (if any), then capture.
pub fn screenshot(
    env: &Env,
    state: Option<&StateSpec>,
    element: Option<&str>,
    masks: Option<&[Mask]>,
) -> Result<Value, String> {
    if let Some(s) = state {
        run_steps(env, &resolve(env, s)?)?;
    }
    let default = state::default_masks();
    capture_now(env, element, masks.unwrap_or(&default), None)
}

/// `ui.screenshot_all`: one labelled capture per cell of states x themes x sizes, then the
/// theme and window size are put back. An axis entry of `-` is skipped.
pub fn screenshot_all(
    env: &Env,
    states: &[String],
    themes: &[String],
    sizes: &[String],
    masks: Option<&[Mask]>,
) -> Result<Value, String> {
    let cells = state::matrix(states, themes, sizes);
    for c in &cells {
        if c.size != "-" {
            Size::parse(&c.size)?;
        }
    }
    let start = call(env.ui, UiRequest::State)?;
    let (w0, h0) = (
        start["window"]["width"].as_f64().unwrap_or(0.0) as f32,
        start["window"]["height"].as_f64().unwrap_or(0.0) as f32,
    );
    let default = state::default_masks();
    let masks = masks.unwrap_or(&default);
    let mut images = vec![];
    let mut rows = vec![];
    let result = (|| -> Result<(), String> {
        for c in &cells {
            let mut v = one_cell(env, c, masks)?;
            if let Some(img) = v
                .as_object_mut()
                .and_then(|o| o.remove(IMAGES_KEY))
                .and_then(|i| i.get(0).cloned())
            {
                images.push(img);
            }
            rows.push(json!({
                "label": c.label,
                "tier": v["tier"],
                "size": v["size"],
                "scale": v["scale"],
                "theme": v["state"]["theme"],
                "generation": v["state"]["generation"],
            }));
        }
        Ok(())
    })();
    // Restore even after a failure part-way through the matrix.
    call(env.ui, UiRequest::SetTheme { name: None })?;
    if w0 > 0.0 && h0 > 0.0 {
        resize(env, Size { w: w0, h: h0 }).ok();
    }
    result?;
    Ok(json!({"cells": rows, IMAGES_KEY: images}))
}

fn one_cell(env: &Env, c: &Cell, masks: &[Mask]) -> Result<Value, String> {
    if c.state != "-" {
        let steps = resolve(env, &StateSpec::Named(c.state.clone()))?;
        // The cell's own theme and size override what the state sets, so run them after.
        let steps: Vec<_> = steps
            .into_iter()
            .filter(|s| {
                !(matches!(s, Step::SetTheme { .. }) && c.theme != "-"
                    || matches!(s, Step::Resize { .. }) && c.size != "-")
            })
            .collect();
        run_steps(env, &steps)?;
    }
    if c.theme != "-" {
        run_step(
            env,
            &Step::SetTheme {
                name: c.theme.clone(),
            },
        )?;
    }
    if c.size != "-" {
        run_step(
            env,
            &Step::Resize {
                size: c.size.clone(),
            },
        )?;
    }
    capture_now(env, None, masks, Some(&c.label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records requests and answers like a minimal app with a fixed 40x30 window.
    struct Stub {
        log: Mutex<Vec<UiRequest>>,
        png: String,
        idle_after: Mutex<u32>,
        workspace: Mutex<Option<String>>,
        size: Mutex<(f32, f32)>,
    }

    impl Stub {
        fn new() -> Self {
            let mut img = image::Rgba {
                w: 80,
                h: 60,
                data: [10, 10, 10, 255].repeat(4800),
            };
            image::fill(
                &mut img,
                image::PxRect {
                    x: 4,
                    y: 4,
                    w: 30,
                    h: 20,
                },
            );
            // A pixel inside the volatile rect that differs per capture would be masked.
            Stub {
                log: Mutex::new(vec![]),
                png: image::to_base64(&image::encode(&img).unwrap()),
                idle_after: Mutex::new(0),
                workspace: Mutex::new(None),
                size: Mutex::new((40.0, 30.0)),
            }
        }

        fn ops(&self) -> Vec<String> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .map(|r| {
                    serde_json::to_value(r).unwrap()["op"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect()
        }
    }

    impl UiBridge for Stub {
        fn describe(&self) -> String {
            "stub".into()
        }

        fn call(&self, req: UiRequest) -> Result<Value, String> {
            self.log.lock().unwrap().push(req.clone());
            Ok(match req {
                UiRequest::Idle => {
                    let mut n = self.idle_after.lock().unwrap();
                    let idle = *n == 0;
                    *n = n.saturating_sub(1);
                    json!({"idle": idle, "generation": 7})
                }
                UiRequest::State => {
                    let (w, h) = *self.size.lock().unwrap();
                    json!({
                        "generation": 7,
                        "workspace_path": *self.workspace.lock().unwrap(),
                        "window": {"width": w, "height": h, "x": 0.0, "y": 0.0, "scale": 2.0},
                        "theme": {"name": "One Dark"},
                    })
                }
                UiRequest::Resize { width, height } => {
                    *self.size.lock().unwrap() = (width, height);
                    json!({})
                }
                UiRequest::OpenWorkspace { path } => {
                    *self.workspace.lock().unwrap() = Some(path);
                    json!({"opened": true})
                }
                UiRequest::Capture { .. } => json!({
                    "png": self.png,
                    "tier": "stub",
                    "scale": 2.0,
                    "region": {"x": 0.0, "y": 0.0, "w": 40.0, "h": 30.0, "scale": 2.0},
                    "volatile": [{"label": "timings", "x": 2.0, "y": 2.0, "w": 10.0, "h": 5.0}],
                    "state": {"generation": 7, "theme": {"name": "One Dark"}},
                }),
                _ => json!({}),
            })
        }
    }

    fn env<'a>(ui: &'a Stub, search: Vec<PathBuf>) -> Env<'a> {
        Env {
            ui,
            pace: Pace::instant(),
            search,
        }
    }

    fn fixtures() -> Vec<PathBuf> {
        vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")]
    }

    #[test]
    fn a_screenshot_returns_an_image_and_the_state_of_the_capture() {
        let ui = Stub::new();
        let v = screenshot(&env(&ui, vec![]), None, None, None).unwrap();
        assert_eq!(v["tier"], "stub");
        assert_eq!(v["size"], json!({"width": 80, "height": 60}));
        assert_eq!(v["state"]["generation"], 7);
        assert_eq!(v[IMAGES_KEY].as_array().unwrap().len(), 1);
        assert!(v.get("png").is_none() && v.get("volatile").is_none());
        let ops = ui.ops();
        let at = |n: &str| ops.iter().position(|o| o == n).unwrap();
        assert!(at("activate") < at("idle") && at("idle") < at("capture"));
    }

    #[test]
    fn volatile_regions_are_masked_so_captures_compare_equal() {
        let ui = Stub::new();
        let v = screenshot(&env(&ui, vec![]), None, None, None).unwrap();
        assert_eq!(v["masked"], 1);
        let out =
            image::decode(&image::from_base64(v[IMAGES_KEY][0]["data"].as_str().unwrap()).unwrap())
                .unwrap();
        // 2pt x 2 = pixel 4; the masked region is solid grey.
        let i = (5 * 80 + 5) * 4;
        assert_eq!(&out.data[i..i + 4], &[128, 128, 128, 255]);
    }

    #[test]
    fn explicit_masks_replace_the_defaults() {
        let ui = Stub::new();
        let v = screenshot(&env(&ui, vec![]), None, None, Some(&[])).unwrap();
        assert_eq!(v["masked"], 0);
        let rect = [Mask::Rect {
            x: 0.0,
            y: 0.0,
            w: 5.0,
            h: 5.0,
        }];
        let v = screenshot(&env(&ui, vec![]), None, None, Some(&rect)).unwrap();
        assert_eq!(v["masked"], 1);
    }

    #[test]
    fn settling_needs_two_idle_polls_with_the_same_generation() {
        let ui = Stub::new();
        *ui.idle_after.lock().unwrap() = 2;
        wait_stable(&env(&ui, vec![])).unwrap();
        let idles = ui.ops().iter().filter(|o| *o == "idle").count();
        assert_eq!(idles, 4);
    }

    #[test]
    fn a_never_idle_app_times_out() {
        let ui = Stub::new();
        *ui.idle_after.lock().unwrap() = u32::MAX;
        assert!(wait_stable(&env(&ui, vec![])).unwrap_err().contains("idle"));
    }

    #[test]
    fn steps_map_to_bridge_requests_in_order() {
        let ui = Stub::new();
        let steps = [
            Step::Open {
                key: "a.flux.md".into(),
                permanent: true,
            },
            Step::SetTheme {
                name: "one-light".into(),
            },
            Step::Invoke {
                name: "Send".into(),
            },
        ];
        run_steps(&env(&ui, vec![]), &steps).unwrap();
        assert_eq!(ui.ops(), ["open_request", "set_theme", "invoke_action"]);
    }

    #[test]
    fn a_failing_step_names_its_position_and_kind() {
        struct Fail;
        impl UiBridge for Fail {
            fn describe(&self) -> String {
                String::new()
            }
            fn call(&self, _: UiRequest) -> Result<Value, String> {
                Err("boom".into())
            }
        }
        let e = Env {
            ui: &Fail,
            pace: Pace::instant(),
            search: vec![],
        };
        let err = run_steps(&e, &[Step::WaitIdle, Step::Invoke { name: "x".into() }]).unwrap_err();
        assert!(err.contains("step 1") && err.contains("wait_idle"), "{err}");
    }

    #[test]
    fn a_named_state_opens_its_workspace_then_runs_its_steps() {
        let ui = Stub::new();
        let e = env(&ui, fixtures());
        let steps = resolve(&e, &StateSpec::Named("palette-open".into())).unwrap();
        assert!(
            steps
                .iter()
                .any(|s| matches!(s, Step::Invoke { name } if name == "CommandPalette"))
        );
        assert_eq!(ui.ops(), ["state", "open_workspace"]);
        let ws = ui.workspace.lock().unwrap().clone().unwrap();
        assert!(ws.ends_with("fixtures/payments-api"), "{ws}");
        // Already open: not reopened.
        resolve(&e, &StateSpec::Named("palette-open".into())).unwrap();
        assert_eq!(
            ui.ops().iter().filter(|o| *o == "open_workspace").count(),
            1
        );
    }

    #[test]
    fn an_unknown_state_lists_the_available_ones() {
        let ui = Stub::new();
        let e = env(&ui, fixtures());
        let err = resolve(&e, &StateSpec::Named("nope".into())).unwrap_err();
        assert!(err.contains("env-modal"), "{err}");
    }

    #[test]
    fn the_matrix_returns_labelled_images_and_restores_theme_and_size() {
        let ui = Stub::new();
        let e = env(&ui, fixtures());
        let v = screenshot_all(
            &e,
            &["env-modal".into()],
            &["one-dark".into(), "one-light".into()],
            &["compact".into()],
            None,
        )
        .unwrap();
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0]["label"], "env-modal/one-dark/compact");
        let imgs = v[IMAGES_KEY].as_array().unwrap();
        assert_eq!(imgs[1]["label"], "env-modal/one-light/compact");
        let log = ui.log.lock().unwrap();
        assert_eq!(
            log.last().unwrap(),
            &UiRequest::State,
            "ends by awaiting the restored size"
        );
        assert!(log.contains(&UiRequest::SetTheme { name: None }));
        assert!(log.contains(&UiRequest::Resize {
            width: 900.0,
            height: 600.0
        }));
        assert!(log.contains(&UiRequest::Resize {
            width: 40.0,
            height: 30.0
        }));
    }

    #[test]
    fn a_bad_size_fails_before_anything_is_changed() {
        let ui = Stub::new();
        let e = env(&ui, fixtures());
        let err = screenshot_all(&e, &[], &[], &["huge".into()], None).unwrap_err();
        assert!(err.contains("invalid size"), "{err}");
        assert!(ui.ops().is_empty());
    }
}
