//! Screenshot states (M9.6): the steps applied before a capture, named states loaded from
//! `fixtures/ui-states/*.yaml`, window sizes and the capture matrix. Pure data and logic; the
//! steps run through the same `ui.*` requests a client would call.

use crate::bridge::UiCommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The smallest window the app allows, as `(width, height)` in points.
pub const MIN_SIZE: (f32, f32) = (640.0, 400.0);

/// One step applied before a capture.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// Open a Request tab by key.
    Open {
        key: String,
        #[serde(default)]
        permanent: bool,
    },
    /// Invoke an action by name.
    Invoke { name: String },
    /// Send a keystroke such as `cmd-shift-p`.
    Key { keys: String },
    /// Type text into the focused input.
    Type { text: String },
    /// A typed `ui.command` (open_file, set_url, select_tab, choose_environment).
    Command {
        #[serde(flatten)]
        command: UiCommand,
    },
    /// Set the theme by name, without saving it to settings.
    SetTheme { name: String },
    /// Resize to a preset (`default`, `compact`, `min`) or `1280x800`.
    Resize { size: String },
    /// Wait until the JSON pointer in `ui.state` equals `equals` (or is non-null).
    WaitFor {
        path: String,
        equals: Option<serde_json::Value>,
        timeout_ms: Option<u64>,
    },
    /// Wait until the UI is idle.
    WaitIdle,
}

/// A `state` argument: a name from `fixtures/ui-states` or an explicit list of steps.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum StateSpec {
    Named(String),
    Steps(Vec<Step>),
}

/// The contents of one `fixtures/ui-states/<name>.yaml`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct StateFile {
    /// Workspace to open first, relative to the repository root or absolute.
    pub workspace: Option<String>,
    pub theme: Option<String>,
    pub size: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl StateFile {
    pub fn parse(src: &str) -> Result<Self, String> {
        serde_yaml_ng::from_str(src).map_err(|e| format!("invalid state file: {e}"))
    }

    /// Theme and size first, then the explicit steps, as one list.
    pub fn all_steps(&self) -> Vec<Step> {
        let mut v = vec![];
        if let Some(t) = &self.theme {
            v.push(Step::SetTheme { name: t.clone() });
        }
        if let Some(s) = &self.size {
            v.push(Step::Resize { size: s.clone() });
        }
        v.extend(self.steps.iter().cloned());
        v
    }
}

/// Loads the named state from `dir`. Names are plain file stems; anything with a separator or
/// a dot is rejected so a name cannot leave the directory.
pub fn load_named(dir: &Path, name: &str) -> Result<StateFile, String> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!("invalid state name `{name}`"));
    }
    let path = dir.join(format!("{name}.yaml"));
    let src = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "unknown state `{name}`; available: {}",
            list_names(dir).join(", ")
        )
    })?;
    StateFile::parse(&src)
}

/// State names in `dir`, sorted.
pub fn list_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "yaml").then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    v.sort();
    v
}

/// Finds `fixtures/ui-states` in `start` or any ancestor.
pub fn find_state_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|a| a.join("fixtures").join("ui-states"))
        .find(|p| p.is_dir())
}

/// A window size in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    /// `default` (1280x800), `compact` (900x600), `min`, or `WxH`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (w, h) = match s {
            "default" => (1280.0, 800.0),
            "compact" => (900.0, 600.0),
            "min" => MIN_SIZE,
            custom => {
                let (a, b) = custom.split_once('x').ok_or_else(|| {
                    format!("invalid size `{s}`: use default, compact, min or WxH")
                })?;
                let n = |v: &str| {
                    v.trim()
                        .parse::<f32>()
                        .map_err(|_| format!("invalid size `{s}`"))
                };
                (n(a)?, n(b)?)
            }
        };
        if w < MIN_SIZE.0 || h < MIN_SIZE.1 {
            return Err(format!(
                "size {w}x{h} is below the minimum {}x{}",
                MIN_SIZE.0, MIN_SIZE.1
            ));
        }
        Ok(Size { w, h })
    }

    pub fn label(&self) -> String {
        format!("{}x{}", self.w.round(), self.h.round())
    }
}

/// Text that changes between captures and is masked so repeated captures compare equal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum Mask {
    /// A named volatile element: `clock` or `timings`.
    Named(String),
    /// A window-coordinate rectangle in points.
    Rect { x: f32, y: f32, w: f32, h: f32 },
}

/// Masks applied when the caller gives none.
pub fn default_masks() -> Vec<Mask> {
    vec![Mask::Named("clock".into()), Mask::Named("timings".into())]
}

/// One cell of a capture matrix.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub label: String,
    pub state: String,
    pub theme: String,
    pub size: String,
}

/// The cross product of states, themes and sizes in a stable order (state, then theme, then
/// size). An empty axis contributes one entry that leaves that setting unchanged (`-`).
pub fn matrix(states: &[String], themes: &[String], sizes: &[String]) -> Vec<Cell> {
    let axis = |v: &[String]| -> Vec<String> {
        if v.is_empty() {
            vec!["-".into()]
        } else {
            v.to_vec()
        }
    };
    let mut out = vec![];
    for s in axis(states) {
        for t in axis(themes) {
            for z in axis(sizes) {
                out.push(Cell {
                    label: format!("{s}/{t}/{z}"),
                    state: s.clone(),
                    theme: t.clone(),
                    size: z.clone(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_state_file_with_steps() {
        let f = StateFile::parse(
            "workspace: fixtures/payments-api\ntheme: one-light\nsize: compact\nsteps:\n  - step: open\n    key: payments/get-payment.flux.md\n  - step: command\n    command: choose_environment\n    name: local\n  - step: invoke\n    name: Send\n  - step: wait_for\n    path: /response/status\n    equals: 200\n",
        )
        .unwrap();
        assert_eq!(f.workspace.as_deref(), Some("fixtures/payments-api"));
        assert_eq!(f.steps.len(), 4);
        assert!(matches!(
            &f.steps[1],
            Step::Command {
                command: UiCommand::ChooseEnvironment { name: Some(n) }
            } if n == "local"
        ));
        let all = f.all_steps();
        assert_eq!(all.len(), 6);
        assert_eq!(
            all[0],
            Step::SetTheme {
                name: "one-light".into()
            }
        );
        assert_eq!(
            all[1],
            Step::Resize {
                size: "compact".into()
            }
        );
    }

    #[test]
    fn rejects_unknown_steps() {
        assert!(StateFile::parse("steps:\n  - step: explode\n").is_err());
    }

    #[test]
    fn state_spec_accepts_a_name_or_steps() {
        let n: StateSpec = serde_json::from_str("\"env-modal\"").unwrap();
        assert_eq!(n, StateSpec::Named("env-modal".into()));
        let s: StateSpec =
            serde_json::from_str(r#"[{"step":"invoke","name":"Send"},{"step":"wait_idle"}]"#)
                .unwrap();
        assert!(matches!(s, StateSpec::Steps(v) if v.len() == 2));
    }

    #[test]
    fn names_cannot_escape_the_directory() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["../x", "a/b", "", "a.b", "..", "a\\b"] {
            assert!(load_named(d.path(), bad).unwrap_err().contains("invalid"));
        }
    }

    #[test]
    fn unknown_names_list_the_available_ones() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("alpha.yaml"), "steps: []\n").unwrap();
        assert_eq!(load_named(d.path(), "alpha").unwrap(), StateFile::default());
        let e = load_named(d.path(), "beta").unwrap_err();
        assert!(e.contains("alpha"), "{e}");
    }

    #[test]
    fn sizes_parse_presets_and_custom() {
        assert_eq!(Size::parse("default").unwrap().label(), "1280x800");
        assert_eq!(Size::parse("compact").unwrap().label(), "900x600");
        assert_eq!(Size::parse("min").unwrap().w, MIN_SIZE.0);
        assert_eq!(Size::parse("1000x700").unwrap().h, 700.0);
        assert!(Size::parse("10x10").unwrap_err().contains("minimum"));
        assert!(Size::parse("wide").is_err());
    }

    #[test]
    fn matrix_is_a_stable_cross_product() {
        let m = matrix(
            &["a".into(), "b".into()],
            &["one-dark".into(), "one-light".into()],
            &[],
        );
        let labels: Vec<_> = m.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "a/one-dark/-",
                "a/one-light/-",
                "b/one-dark/-",
                "b/one-light/-"
            ]
        );
    }

    #[test]
    fn masks_deserialize_from_names_and_rects() {
        let m: Vec<Mask> = serde_json::from_str(r#"["clock",{"x":1,"y":2,"w":3,"h":4}]"#).unwrap();
        assert_eq!(m[0], Mask::Named("clock".into()));
        assert!(matches!(m[1], Mask::Rect { w, .. } if w == 3.0));
    }

    #[test]
    fn the_shipped_fixtures_all_parse() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ui-states");
        let names = list_names(&root);
        for want in [
            "response-200-json",
            "response-error",
            "folder-run-failed",
            "ws-session-open",
            "env-modal",
            "trust-prompt",
            "palette-open",
            "empty-workspace",
            "problems-open",
        ] {
            assert!(names.iter().any(|n| n == want), "missing fixture {want}");
        }
        for n in names {
            let f = load_named(&root, &n).unwrap_or_else(|e| panic!("{n}: {e}"));
            for s in f.all_steps() {
                if let Step::Resize { size } = s {
                    Size::parse(&size).unwrap_or_else(|e| panic!("{n}: {e}"));
                }
            }
        }
    }
}
