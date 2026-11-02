//! Scenarios (M9.8): repeatable UI tests written as YAML. A scenario is a list of steps (the
//! M9.4 steps of [`state::Step`]) plus assertions on `ui.state`, `ui.find` results and the audit,
//! screenshot checkpoints and event waits. [`run`] drives them through the [`UiBridge`] and
//! returns a pass/fail report; a failure carries the expected and actual value and a screenshot.
//! [`Recorder`] turns the calls of a session back into a scenario file.

use crate::bridge::UiRequest;
use crate::events::EventBuffer;
use crate::screenshot::{self, Env};
use crate::server::IMAGES_KEY;
use crate::state::{self, Step};
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Step names that are scenario-only; everything else is a [`Step`].
const EXTRA: &[&str] = &[
    "click_element",
    "assert",
    "assert_find",
    "screenshot",
    "audit",
    "wait_event",
];

/// A scenario-only step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Extra {
    /// Find an element by label/role (the first visible match) and click it.
    ClickElement {
        label: Option<String>,
        role: Option<String>,
    },
    /// Assert on `ui.state`: the JSON pointer `path` must satisfy `equals`, `contains`
    /// (string or array element), `matches` (regular expression on the string value) or, with
    /// none of them, exist and be non-null. `not: true` inverts the check.
    Assert {
        path: String,
        equals: Option<Value>,
        contains: Option<Value>,
        matches: Option<String>,
        #[serde(default)]
        not: bool,
    },
    /// Assert on `ui.find`: how many elements match and whether they are enabled.
    AssertFind {
        label: Option<String>,
        role: Option<String>,
        text: Option<String>,
        /// Exact match count; default is at least one (zero when `exists: false`).
        count: Option<usize>,
        exists: Option<bool>,
        enabled: Option<bool>,
    },
    /// A screenshot checkpoint named `name`; `element` crops to an element id.
    Screenshot {
        name: String,
        element: Option<String>,
    },
    /// Run the audit; fail on findings at or above `fail_on` (`error` by default).
    Audit {
        #[serde(default)]
        rules: Vec<String>,
        fail_on: Option<String>,
    },
    /// Wait for an event of `kind` (see `ui.events`), optionally whose JSON contains `contains`.
    WaitEvent {
        kind: String,
        contains: Option<String>,
        timeout_ms: Option<u64>,
    },
}

/// One scenario step: a plain state step or a scenario-only one.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum ScenarioStep {
    Base(Step),
    Extra(Extra),
}

impl<'de> Deserialize<'de> for ScenarioStep {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let tag = v.get("step").and_then(Value::as_str).unwrap_or_default();
        if EXTRA.contains(&tag) {
            serde_json::from_value(v)
                .map(ScenarioStep::Extra)
                .map_err(D::Error::custom)
        } else {
            serde_json::from_value(v)
                .map(ScenarioStep::Base)
                .map_err(D::Error::custom)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub description: Option<String>,
    /// Workspace to open first, relative to the repository root or absolute.
    pub workspace: Option<String>,
    pub theme: Option<String>,
    pub size: Option<String>,
    pub steps: Vec<ScenarioStep>,
}

impl Scenario {
    pub fn parse(src: &str) -> Result<Self, String> {
        serde_yaml_ng::from_str(src).map_err(|e| format!("invalid scenario: {e}"))
    }

    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml_ng::to_string(self).map_err(|e| e.to_string())
    }
}

/// Finds `fixtures/scenarios` in `start` or any ancestor.
pub fn find_scenario_dir(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|a| a.join("fixtures").join("scenarios"))
        .find(|p| p.is_dir())
}

/// Scenario names in `dir`, sorted.
pub fn list_names(dir: &Path) -> Vec<String> {
    state::list_names(dir)
}

/// Loads `fixtures/scenarios/<name>.yaml`; names cannot leave the directory.
pub fn load_named(dir: &Path, name: &str) -> Result<Scenario, String> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(format!("invalid scenario name `{name}`"));
    }
    let src = std::fs::read_to_string(dir.join(format!("{name}.yaml"))).map_err(|_| {
        format!(
            "unknown scenario `{name}`; available: {}",
            list_names(dir).join(", ")
        )
    })?;
    Scenario::parse(&src)
}

/// How a run behaves.
pub struct RunOpts<'a> {
    /// Take screenshot checkpoints and the failure screenshot. Off, `screenshot` steps are
    /// reported as skipped (CI without a display or Screen Recording permission).
    pub capture: bool,
    /// Where events are read from; the process-wide buffer in the app.
    pub events: &'a EventBuffer,
    /// Directory the scenario's workspace path is relative to.
    pub repo: Option<PathBuf>,
}

fn name_of(s: &ScenarioStep) -> String {
    match s {
        ScenarioStep::Base(b) => screenshot::step_name(b).to_string(),
        ScenarioStep::Extra(e) => match e {
            Extra::ClickElement { .. } => "click_element",
            Extra::Assert { .. } => "assert",
            Extra::AssertFind { .. } => "assert_find",
            Extra::Screenshot { .. } => "screenshot",
            Extra::Audit { .. } => "audit",
            Extra::WaitEvent { .. } => "wait_event",
        }
        .to_string(),
    }
}

/// Outcome of one step before it is put in the report.
enum Done {
    Ok(Option<Value>),
    Skipped(String),
}

fn show(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn check_assert(state: &Value, a: &Extra) -> Result<(), String> {
    let Extra::Assert {
        path,
        equals,
        contains,
        matches,
        not,
    } = a
    else {
        unreachable!()
    };
    let got = state.pointer(path);
    let pass = if let Some(w) = equals {
        got == Some(w)
    } else if let Some(w) = contains {
        match got {
            Some(Value::String(s)) => w.as_str().is_some_and(|w| s.contains(w)),
            Some(Value::Array(a)) => a.contains(w),
            _ => false,
        }
    } else if let Some(re) = matches {
        let re = regex_lite(re)?;
        got.and_then(Value::as_str).is_some_and(re)
    } else {
        got.is_some_and(|v| !v.is_null())
    };
    if pass != *not {
        return Ok(());
    }
    let want = match (equals, contains, matches) {
        (Some(w), _, _) => format!("equal {}", show(w)),
        (_, Some(w), _) => format!("contain {}", show(w)),
        (_, _, Some(r)) => format!("match /{r}/"),
        _ => "exist and be non-null".to_string(),
    };
    Err(format!(
        "expected {path} to {}{want}, got {}",
        if *not { "not " } else { "" },
        got.map(show).unwrap_or_else(|| "(missing)".into())
    ))
}

/// A minimal matcher for `matches`: `^`/`$` anchors and `.*` wildcards over literal text. A
/// scenario needs substring and prefix checks, not a full regex engine.
fn regex_lite(p: &str) -> Result<impl Fn(&str) -> bool + use<>, String> {
    let (anch_s, p) = p.strip_prefix('^').map_or((false, p), |r| (true, r));
    let (anch_e, p) = p.strip_suffix('$').map_or((false, p), |r| (true, r));
    if p.contains(['[', '(', '+', '?', '|', '\\']) {
        return Err(format!(
            "unsupported pattern `{p}`: `matches` takes literals, `.*`, `^` and `$`"
        ));
    }
    let parts: Vec<String> = p.split(".*").map(String::from).collect();
    Ok(move |s: &str| {
        let mut at = 0usize;
        for (i, part) in parts.iter().enumerate() {
            if i == 0 && anch_s {
                if !s.starts_with(part.as_str()) {
                    return false;
                }
                at = part.len();
                continue;
            }
            match s[at..].find(part.as_str()) {
                Some(j) => at += j + part.len(),
                None => return false,
            }
        }
        !anch_e || parts.last().is_some_and(|l| s.ends_with(l.as_str()))
    })
}

fn find_req(label: &Option<String>, role: &Option<String>, text: &Option<String>) -> UiRequest {
    UiRequest::Find {
        label: label.clone(),
        role: role.clone(),
        text: text.clone(),
    }
}

fn run_extra(env: &Env, opts: &RunOpts, e: &Extra, last_seq: &mut u64) -> Result<Done, String> {
    match e {
        Extra::ClickElement { label, role } => {
            let v = env.ui.call(find_req(label, role, &None))?;
            let id = v["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|n| n["visible"] != false)
                .and_then(|n| n["id"].as_str())
                .ok_or_else(|| format!("no visible element matches label={label:?} role={role:?}"))?
                .to_string();
            env.ui.call(UiRequest::ClickElement { id })?;
            Ok(Done::Ok(None))
        }
        Extra::Assert { .. } => {
            let st = env.ui.call(UiRequest::State)?;
            check_assert(&st, e)?;
            Ok(Done::Ok(None))
        }
        Extra::AssertFind {
            label,
            role,
            text,
            count,
            exists,
            enabled,
        } => {
            let v = env.ui.call(find_req(label, role, text))?;
            let found: Vec<&Value> = v["elements"].as_array().into_iter().flatten().collect();
            let q = format!("label={label:?} role={role:?} text={text:?}");
            match (count, exists) {
                (Some(n), _) if found.len() != *n => {
                    return Err(format!(
                        "expected {n} elements for {q}, found {}",
                        found.len()
                    ));
                }
                (None, Some(false)) if !found.is_empty() => {
                    return Err(format!(
                        "expected no element for {q}, found {}",
                        found.len()
                    ));
                }
                (None, Some(false)) => {}
                (None, _) if found.is_empty() => {
                    return Err(format!("expected an element for {q}, found none"));
                }
                _ => {}
            }
            if let Some(want) = enabled
                && let Some(n) = found.iter().find(|n| n["enabled"] != *want)
            {
                return Err(format!(
                    "expected {} to be enabled={want}, got {}",
                    n["id"], n["enabled"]
                ));
            }
            Ok(Done::Ok(None))
        }
        Extra::Screenshot { name, element } => {
            if !opts.capture {
                return Ok(Done::Skipped("capture disabled".into()));
            }
            let masks = state::default_masks();
            let v = screenshot::capture_now(env, element.as_deref(), &masks, Some(name))?;
            Ok(Done::Ok(v.get(IMAGES_KEY).cloned()))
        }
        Extra::Audit { rules, fail_on } => {
            let level = |s: &str| match s {
                "info" => 0,
                "warning" => 1,
                _ => 2,
            };
            let min = level(fail_on.as_deref().unwrap_or("error"));
            let v = crate::audit::audit(
                env,
                &crate::audit::AuditArgs {
                    rules: rules.clone(),
                    subtree: None,
                    states: vec![],
                    themes: vec![],
                    sizes: vec![],
                },
            )?;
            let bad: Vec<String> = v["cells"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|c| c["findings"].as_array().into_iter().flatten())
                .filter(|f| level(f["severity"].as_str().unwrap_or("error")) >= min)
                .map(|f| {
                    format!(
                        "{} {}: {}",
                        f["rule"].as_str().unwrap_or("?"),
                        f["severity"].as_str().unwrap_or("?"),
                        f["message"].as_str().unwrap_or("")
                    )
                })
                .collect();
            if bad.is_empty() {
                Ok(Done::Ok(None))
            } else {
                Err(format!("audit found {}: {}", bad.len(), bad.join("; ")))
            }
        }
        Extra::WaitEvent {
            kind,
            contains,
            timeout_ms,
        } => {
            let end = Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(5000));
            let kinds = [kind.clone()];
            loop {
                let left = end.saturating_duration_since(Instant::now());
                let hits = opts.events.wait(*last_seq, &kinds, left);
                if let Some(m) = hits.last() {
                    *last_seq = m.seq.max(*last_seq);
                }
                if hits
                    .iter()
                    .any(|h| contains.as_ref().is_none_or(|c| show(&h.data).contains(c)))
                {
                    return Ok(Done::Ok(None));
                }
                if Instant::now() >= end {
                    return Err(format!(
                        "timed out waiting for a `{kind}` event{}",
                        contains
                            .as_ref()
                            .map(|c| format!(" containing {c:?}"))
                            .unwrap_or_default()
                    ));
                }
            }
        }
    }
}

/// Runs a scenario and returns the report: `{name, passed, steps: [{n, step, status, ms,
/// error?}], failure?: {n, step, error, state}, _images}`.
pub fn run(env: &Env, opts: &RunOpts, sc: &Scenario) -> Value {
    let t0 = Instant::now();
    let mut rows: Vec<Value> = vec![];
    let mut images: Vec<Value> = vec![];
    let mut failure = None;
    let mut last_seq = opts.events.latest();

    let setup = (|| -> Result<(), String> {
        if let Some(ws) = &sc.workspace {
            let repo = opts.repo.clone().unwrap_or_default();
            let p = Path::new(ws);
            let path = if p.is_absolute() {
                p.into()
            } else {
                repo.join(p)
            };
            let path = path.canonicalize().unwrap_or(path);
            env.ui.call(UiRequest::OpenWorkspace {
                path: path.to_string_lossy().into_owned(),
            })?;
        }
        if let Some(t) = &sc.theme {
            screenshot::run_step(env, &Step::SetTheme { name: t.clone() })?;
        }
        if let Some(s) = &sc.size {
            screenshot::run_step(env, &Step::Resize { size: s.clone() })?;
        }
        Ok(())
    })();
    if let Err(e) = setup {
        failure = Some((0, "setup".to_string(), e));
    }

    if failure.is_none() {
        for (i, s) in sc.steps.iter().enumerate() {
            let t = Instant::now();
            let r = match s {
                ScenarioStep::Base(b) => screenshot::run_step(env, b).map(|_| Done::Ok(None)),
                ScenarioStep::Extra(e) => run_extra(env, opts, e, &mut last_seq),
            };
            let ms = t.elapsed().as_millis() as u64;
            let mut row = json!({"n": i + 1, "step": name_of(s), "ms": ms});
            match r {
                Ok(Done::Ok(img)) => {
                    row["status"] = "pass".into();
                    if let Some(Value::Array(a)) = img {
                        images.extend(a);
                    }
                }
                Ok(Done::Skipped(why)) => {
                    row["status"] = "skipped".into();
                    row["note"] = why.into();
                }
                Err(e) => {
                    row["status"] = "fail".into();
                    row["error"] = e.clone().into();
                    failure = Some((i + 1, name_of(s), e));
                    rows.push(row);
                    break;
                }
            }
            rows.push(row);
        }
    }

    let mut report = json!({
        "name": sc.name,
        "passed": failure.is_none(),
        "steps": rows,
        "wall_ms": t0.elapsed().as_millis() as u64,
    });
    if let Some((n, step, error)) = failure {
        let state = env.ui.call(UiRequest::State).unwrap_or(Value::Null);
        report["failure"] = json!({"n": n, "step": step, "error": error, "state": state});
        if opts.capture {
            let label = format!("failure: {} step {n} ({step})", sc.name);
            match screenshot::capture_now(env, None, &state::default_masks(), Some(&label)) {
                Ok(v) => {
                    if let Some(Value::Array(a)) = v.get(IMAGES_KEY) {
                        images.extend(a.iter().cloned());
                    }
                }
                Err(e) => report["failure"]["screenshot_error"] = e.into(),
            }
        }
    }
    if !images.is_empty() {
        report[IMAGES_KEY] = Value::Array(images);
    }
    report
}

/// Records the `ui.*` calls of a session as scenario steps.
#[derive(Default)]
pub struct Recorder {
    steps: Vec<ScenarioStep>,
    active: bool,
}

impl Recorder {
    pub fn start(&mut self) {
        self.steps.clear();
        self.active = true;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Notes a request that changes the app; reads and waits are not recorded.
    pub fn note(&mut self, req: &UiRequest) {
        if !self.active {
            return;
        }
        let step = match req {
            UiRequest::OpenRequest { key, permanent } => Step::Open {
                key: key.clone(),
                permanent: *permanent,
            },
            UiRequest::InvokeAction { name } => Step::Invoke { name: name.clone() },
            UiRequest::Key { keys } => Step::Key { keys: keys.clone() },
            UiRequest::TypeText { text } => Step::Type { text: text.clone() },
            UiRequest::ClickElement { id } => Step::Click { id: id.clone() },
            UiRequest::Command { command } => Step::Command {
                command: command.clone(),
            },
            UiRequest::SetTheme { name: Some(n) } => Step::SetTheme { name: n.clone() },
            UiRequest::Resize { width, height } => Step::Resize {
                size: format!("{}x{}", width.round(), height.round()),
            },
            _ => return,
        };
        self.steps.push(ScenarioStep::Base(step));
        self.steps.push(ScenarioStep::Base(Step::WaitIdle));
    }

    /// Stops and returns the scenario; `state` (the final `ui.state`) adds assertions for the
    /// stable facts worth checking.
    pub fn stop(
        &mut self,
        name: &str,
        workspace: Option<String>,
        state: Option<&Value>,
    ) -> Scenario {
        self.active = false;
        let mut steps = std::mem::take(&mut self.steps);
        if let Some(st) = state {
            for p in ["/response/status", "/modal", "/bottom_tab"] {
                if let Some(v) = st.pointer(p).filter(|v| !v.is_null()) {
                    steps.push(ScenarioStep::Extra(Extra::Assert {
                        path: p.into(),
                        equals: Some(v.clone()),
                        contains: None,
                        matches: None,
                        not: false,
                    }));
                }
            }
        }
        Scenario {
            name: name.into(),
            description: Some("Recorded with ui.record.".into()),
            workspace,
            theme: None,
            size: None,
            steps,
        }
    }
}

/// The recorder shared by `ui.record` and the `ui.*` tools.
pub fn recorder() -> &'static std::sync::Mutex<Recorder> {
    static R: std::sync::OnceLock<std::sync::Mutex<Recorder>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UiBridge;
    use crate::screenshot::Pace;
    use std::sync::Mutex;

    /// Answers state/find/idle from fixed JSON and records the rest.
    struct Stub {
        state: Mutex<Value>,
        elements: Value,
        log: Mutex<Vec<UiRequest>>,
        findings: Value,
    }

    impl Stub {
        fn new() -> Self {
            Stub {
                state: Mutex::new(
                    json!({"generation": 1, "response": {"status": 200, "body": "hello world"}, "tabs": ["a", "b"]}),
                ),
                elements: json!([{"id": "btn.send", "label": "Send", "enabled": true, "visible": true}]),
                log: Mutex::new(vec![]),
                findings: json!([]),
            }
        }
    }

    impl UiBridge for Stub {
        fn describe(&self) -> String {
            "stub".into()
        }
        fn call(&self, req: UiRequest) -> Result<Value, String> {
            self.log.lock().unwrap().push(req.clone());
            Ok(match req {
                UiRequest::State => self.state.lock().unwrap().clone(),
                UiRequest::Idle => json!({"idle": true, "generation": 1}),
                UiRequest::Find { .. } => json!({"elements": self.elements}),
                UiRequest::Audit { .. } => json!({"findings": self.findings}),
                _ => json!({"ok": true}),
            })
        }
    }

    fn env(s: &Stub) -> Env<'_> {
        Env {
            ui: s,
            pace: Pace::instant(),
            search: vec![],
        }
    }

    fn go(s: &Stub, yaml: &str, ev: &EventBuffer) -> Value {
        let sc = Scenario::parse(yaml).unwrap();
        run(
            &env(s),
            &RunOpts {
                capture: false,
                events: ev,
                repo: None,
            },
            &sc,
        )
    }

    #[test]
    fn parses_base_and_extra_steps() {
        let sc = Scenario::parse(
            "name: t\nsteps:\n  - step: invoke\n    name: Send\n  - step: assert\n    path: /response/status\n    equals: 200\n  - step: screenshot\n    name: done\n",
        )
        .unwrap();
        assert!(matches!(
            &sc.steps[0],
            ScenarioStep::Base(Step::Invoke { .. })
        ));
        assert!(matches!(
            &sc.steps[1],
            ScenarioStep::Extra(Extra::Assert { .. })
        ));
        assert!(matches!(
            &sc.steps[2],
            ScenarioStep::Extra(Extra::Screenshot { .. })
        ));
        assert!(Scenario::parse("name: t\nsteps:\n  - step: explode\n").is_err());
    }

    #[test]
    fn yaml_round_trips() {
        let sc = Scenario::parse(
            "name: t\nsteps:\n  - step: open\n    key: a.flux.md\n  - step: wait_idle\n  - step: assert_find\n    label: Send\n    enabled: true\n",
        )
        .unwrap();
        assert_eq!(Scenario::parse(&sc.to_yaml().unwrap()).unwrap(), sc);
    }

    #[test]
    fn a_passing_scenario_reports_every_step() {
        let s = Stub::new();
        let r = go(
            &s,
            "name: ok\nsteps:\n  - step: invoke\n    name: Send\n  - step: wait_idle\n  - step: assert\n    path: /response/status\n    equals: 200\n  - step: assert\n    path: /response/body\n    contains: world\n  - step: assert\n    path: /response/body\n    matches: '^hello.*world$'\n  - step: assert\n    path: /missing\n    not: true\n  - step: assert_find\n    label: Send\n    count: 1\n    enabled: true\n  - step: click_element\n    label: Send\n  - step: audit\n  - step: screenshot\n    name: x\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], true, "{r}");
        assert_eq!(r["steps"].as_array().unwrap().len(), 10);
        assert_eq!(r["steps"][9]["status"], "skipped");
        assert!(s.log.lock().unwrap().contains(&UiRequest::ClickElement {
            id: "btn.send".into()
        }));
    }

    #[test]
    fn a_broken_assertion_fails_with_expected_and_actual() {
        let s = Stub::new();
        let r = go(
            &s,
            "name: bad\nsteps:\n  - step: assert\n    path: /response/status\n    equals: 404\n  - step: wait_idle\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], false);
        assert_eq!(
            r["steps"].as_array().unwrap().len(),
            1,
            "stops at the failure"
        );
        let e = r["failure"]["error"].as_str().unwrap();
        assert!(
            e.contains("/response/status") && e.contains("404") && e.contains("200"),
            "{e}"
        );
        assert_eq!(r["failure"]["n"], 1);
        assert_eq!(r["failure"]["state"]["response"]["status"], 200);
    }

    #[test]
    fn find_assertions_check_count_and_enabled() {
        let mut s = Stub::new();
        s.elements = json!([{"id": "a", "enabled": false, "visible": true}]);
        let r = go(
            &s,
            "name: f\nsteps:\n  - step: assert_find\n    label: Send\n    enabled: true\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], false);
        let r = go(
            &s,
            "name: f\nsteps:\n  - step: assert_find\n    label: Nope\n    count: 2\n",
            &EventBuffer::new(8),
        );
        assert!(
            r["failure"]["error"]
                .as_str()
                .unwrap()
                .contains("expected 2")
        );
        s.elements = json!([]);
        let r = go(
            &s,
            "name: f\nsteps:\n  - step: assert_find\n    label: Send\n    exists: false\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], true);
    }

    #[test]
    fn audit_errors_fail_but_warnings_pass() {
        let mut s = Stub::new();
        s.findings = json!([{"rule": "UX103", "severity": "warning", "message": "small"}]);
        let r = go(
            &s,
            "name: a\nsteps:\n  - step: audit\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], true);
        s.findings = json!([{"rule": "UX110", "severity": "error", "message": "contrast"}]);
        let r = go(
            &s,
            "name: a\nsteps:\n  - step: audit\n",
            &EventBuffer::new(8),
        );
        assert_eq!(r["passed"], false);
        assert!(r["failure"]["error"].as_str().unwrap().contains("UX110"));
    }

    #[test]
    fn wait_event_sees_new_events_only() {
        let s = Stub::new();
        let ev = EventBuffer::new(8);
        ev.push("run", json!({"phase": "old"}));
        let sc = Scenario::parse("name: w\nsteps:\n  - step: wait_event\n    kind: run\n    contains: done\n    timeout_ms: 2000\n").unwrap();
        let ev = std::sync::Arc::new(ev);
        let e2 = ev.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            e2.push("run", json!({"phase": "done"}));
        });
        let r = run(
            &env(&s),
            &RunOpts {
                capture: false,
                events: &ev,
                repo: None,
            },
            &sc,
        );
        t.join().unwrap();
        assert_eq!(r["passed"], true, "{r}");
        let r = go(
            &s,
            "name: w\nsteps:\n  - step: wait_event\n    kind: frame\n    timeout_ms: 30\n",
            &ev,
        );
        assert!(
            r["failure"]["error"]
                .as_str()
                .unwrap()
                .contains("timed out")
        );
    }

    #[test]
    fn matches_supports_anchors_and_wildcards_only() {
        let m = regex_lite("^ab.*ef$").unwrap();
        assert!(m("abcdef") && !m("xabcdef") && !m("abcdeg"));
        assert!(regex_lite("a(b)").is_err());
    }

    #[test]
    fn recorder_captures_changes_and_skips_reads() {
        let mut r = Recorder::default();
        r.note(&UiRequest::State);
        assert!(r.stop("x", None, None).steps.is_empty());
        r.start();
        r.note(&UiRequest::State);
        r.note(&UiRequest::InvokeAction {
            name: "Send".into(),
        });
        r.note(&UiRequest::Resize {
            width: 900.0,
            height: 600.0,
        });
        let sc = r.stop(
            "rec",
            Some("fixtures/payments-api".into()),
            Some(&json!({"response": {"status": 200}})),
        );
        assert_eq!(sc.steps.len(), 5);
        assert!(
            matches!(&sc.steps[4], ScenarioStep::Extra(Extra::Assert { path, .. }) if path == "/response/status")
        );
        assert_eq!(Scenario::parse(&sc.to_yaml().unwrap()).unwrap(), sc);
    }

    #[test]
    fn names_cannot_escape_the_directory() {
        let d = tempfile::tempdir().unwrap();
        assert!(
            load_named(d.path(), "../x")
                .unwrap_err()
                .contains("invalid")
        );
        std::fs::write(d.path().join("a.yaml"), "name: a\nsteps: []\n").unwrap();
        assert_eq!(load_named(d.path(), "a").unwrap().name, "a");
        assert!(
            load_named(d.path(), "b")
                .unwrap_err()
                .contains("available: a")
        );
    }
}
