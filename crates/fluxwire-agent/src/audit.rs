//! `ui.audit` and `ui.walkthrough` orchestration (M9.7). The rules live in `fluxwire-ui`
//! (`dev_audit`) because they need the semantic tree; this layer applies states, themes and
//! sizes through the [`UiBridge`] exactly as `ui.screenshot_all` does, asks the app to audit each
//! cell, and aggregates. Walkthrough times a list of steps and counts the user effort in them.

use crate::bridge::UiRequest;
use crate::screenshot::{Env, resize, resolve, run_step, step_name, wait_stable};
use crate::state::{self, Size, StateSpec, Step};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

/// Rules about the app as a whole rather than one frame; run once per audit, not per cell.
const GLOBAL_RULES: &[&str] = &["UX120", "UX130", "UX140", "UX150"];

pub struct AuditArgs {
    pub rules: Vec<String>,
    pub subtree: Option<String>,
    pub states: Vec<String>,
    pub themes: Vec<String>,
    pub sizes: Vec<String>,
}

fn fixture_names(env: &Env) -> Vec<String> {
    env.search
        .iter()
        .find_map(|d| state::find_state_dir(d))
        .map(|d| state::list_names(&d))
        .unwrap_or_default()
}

fn counts(findings: &[Value]) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    for f in findings {
        *m.entry(f["rule"].as_str().unwrap_or("?").to_string())
            .or_insert(0) += 1;
    }
    m
}

pub fn audit(env: &Env, a: &AuditArgs) -> Result<Value, String> {
    let fixtures = fixture_names(env);
    let cells = state::matrix(&a.states, &a.themes, &a.sizes);
    for c in &cells {
        if c.size != "-" {
            Size::parse(&c.size)?;
        }
    }
    let start = env.ui.call(UiRequest::State)?;
    let (w0, h0) = (
        start["window"]["width"].as_f64().unwrap_or(0.0) as f32,
        start["window"]["height"].as_f64().unwrap_or(0.0) as f32,
    );
    let swept = cells
        .iter()
        .any(|c| c.state != "-" || c.theme != "-" || c.size != "-");
    let mut rows = vec![];
    let mut all: Vec<Value> = vec![];
    let result = (|| -> Result<(), String> {
        for (i, c) in cells.iter().enumerate() {
            if c.state != "-" {
                let steps = resolve(env, &StateSpec::Named(c.state.clone()))?;
                for s in steps {
                    let skip = matches!(s, Step::SetTheme { .. }) && c.theme != "-"
                        || matches!(s, Step::Resize { .. }) && c.size != "-";
                    if !skip {
                        run_step(env, &s)?;
                    }
                }
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
                resize(env, Size::parse(&c.size)?)?;
            }
            wait_stable(env)?;
            let rules: Vec<String> = if i == 0 {
                a.rules.clone()
            } else if a.rules.is_empty() {
                crate::audit::per_frame_rules()
            } else {
                a.rules
                    .iter()
                    .filter(|r| !GLOBAL_RULES.contains(&r.to_ascii_uppercase().as_str()))
                    .cloned()
                    .collect()
            };
            if i > 0 && rules.is_empty() {
                rows.push(json!({"label": c.label, "findings": []}));
                continue;
            }
            let v = env.ui.call(UiRequest::Audit {
                rules,
                subtree: a.subtree.clone(),
                theme: (c.theme != "-").then(|| c.theme.clone()),
                fixtures: fixtures.clone(),
            })?;
            let mut fs = v["findings"].as_array().cloned().unwrap_or_default();
            for f in &mut fs {
                f["cell"] = json!(c.label);
            }
            all.extend(fs.clone());
            rows.push(json!({"label": c.label, "findings": fs}));
        }
        Ok(())
    })();
    if swept {
        env.ui.call(UiRequest::SetTheme { name: None })?;
        if w0 > 0.0 && h0 > 0.0 {
            resize(env, Size { w: w0, h: h0 }).ok();
        }
    }
    result?;
    Ok(json!({
        "cells": rows,
        "total": all.len(),
        "by_rule": counts(&all),
    }))
}

/// The rules that depend on the frame rather than the app, for cells after the first.
fn per_frame_rules() -> Vec<String> {
    ["UX101", "UX102", "UX103", "UX110", "UX131"]
        .map(String::from)
        .to_vec()
}

/// Effort counted in one step: keystrokes typed or pressed, and element clicks.
fn effort(s: &Step) -> (u64, u64) {
    match s {
        Step::Key { keys } => (keys.split_whitespace().count() as u64, 0),
        Step::Type { text } => (text.chars().count() as u64, 0),
        Step::Click { .. } => (0, 1),
        _ => (0, 0),
    }
}

/// Runs `steps`, stopping at the first failure, and reports per-step and total cost.
pub fn walkthrough(env: &Env, steps: &[Step]) -> Result<Value, String> {
    let t0 = Instant::now();
    let (mut keys, mut clicks, mut notices) = (0, 0, 0);
    let mut rows = vec![];
    let mut error = None;
    for (i, s) in steps.iter().enumerate() {
        let (k, c) = effort(s);
        let t = Instant::now();
        let r = run_step(env, s);
        let ms = t.elapsed().as_millis() as u64;
        let notice = env
            .ui
            .call(UiRequest::State)
            .ok()
            .and_then(|v| v["notice"].as_str().map(String::from));
        keys += k;
        clicks += c;
        notices += u64::from(notice.is_some());
        let mut row = json!({
            "n": i + 1, "step": step_name(s), "ms": ms,
            "keystrokes": k, "clicks": c, "notice": notice,
        });
        if let Err(e) = r {
            row["error"] = json!(e);
            error = Some(format!("step {} ({}): {e}", i + 1, step_name(s)));
            rows.push(row);
            break;
        }
        rows.push(row);
    }
    Ok(json!({
        "ok": error.is_none(),
        "error": error,
        "steps": rows,
        "totals": {
            "steps": rows.len(),
            "keystrokes": keys,
            "clicks": clicks,
            "notices": notices,
            "wall_ms": t0.elapsed().as_millis() as u64,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UiBridge;
    use crate::screenshot::Pace;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Stub {
        log: Mutex<Vec<UiRequest>>,
    }

    impl UiBridge for Stub {
        fn describe(&self) -> String {
            "stub".into()
        }
        fn call(&self, req: UiRequest) -> Result<Value, String> {
            self.log.lock().unwrap().push(req.clone());
            Ok(match req {
                UiRequest::State => {
                    json!({"window": {"width": 1280.0, "height": 800.0}, "notice": null, "generation": 1})
                }
                UiRequest::Idle => json!({"idle": true, "generation": 1}),
                UiRequest::Audit { rules, .. } => json!({
                    "findings": [{"rule": rules.first().cloned().unwrap_or("UX101".into())}]
                }),
                UiRequest::ClickElement { id } if id == "bad" => return Err("nope".into()),
                _ => json!({}),
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

    #[test]
    fn audit_runs_global_rules_once_across_a_sweep() {
        let s = Stub::default();
        let a = AuditArgs {
            rules: vec![],
            subtree: None,
            states: vec![],
            themes: vec!["one-dark".into(), "one-light".into()],
            sizes: vec![],
        };
        let v = audit(&env(&s), &a).unwrap();
        assert_eq!(v["cells"].as_array().unwrap().len(), 2);
        let log = s.log.lock().unwrap();
        let rules: Vec<_> = log
            .iter()
            .filter_map(|r| match r {
                UiRequest::Audit { rules, .. } => Some(rules.clone()),
                _ => None,
            })
            .collect();
        assert!(rules[0].is_empty());
        assert_eq!(rules[1], per_frame_rules());
        // The theme is restored after a sweep.
        assert!(log.contains(&UiRequest::SetTheme { name: None }));
    }

    #[test]
    fn audit_of_the_current_view_changes_nothing() {
        let s = Stub::default();
        let a = AuditArgs {
            rules: vec!["UX103".into()],
            subtree: None,
            states: vec![],
            themes: vec![],
            sizes: vec![],
        };
        let v = audit(&env(&s), &a).unwrap();
        assert_eq!(v["total"], 1);
        assert!(
            !s.log
                .lock()
                .unwrap()
                .contains(&UiRequest::SetTheme { name: None })
        );
    }

    #[test]
    fn walkthrough_counts_effort_and_stops_on_failure() {
        let s = Stub::default();
        let steps = vec![
            Step::Key {
                keys: "cmd-shift-p".into(),
            },
            Step::Type {
                text: "send".into(),
            },
            Step::Click { id: "ok".into() },
            Step::Click { id: "bad".into() },
            Step::Key {
                keys: "enter".into(),
            },
        ];
        let v = walkthrough(&env(&s), &steps).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["totals"]["steps"], 4);
        assert_eq!(v["totals"]["keystrokes"], 5);
        assert_eq!(v["totals"]["clicks"], 2);
        assert!(v["error"].as_str().unwrap().contains("step 4 (click)"));
    }
}
