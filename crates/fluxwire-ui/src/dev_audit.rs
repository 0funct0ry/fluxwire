//! The UI audit (`dev-mcp`, M9.7, ADR-009): pure rules over the semantic tree
//! ([`crate::dev_tree`]) and theme tokens. Findings carry a rule id, severity, element ids,
//! bounds and a suggested fix, and are sorted so the same input always gives the same output.
//! No GPUI types here, so every rule is tested with hand-built trees.

use crate::dev_tree::{Node, Rect, Role};
use fluxwire_format::theme::{Rgba, ThemeTokens};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Every rule id, in report order.
pub const RULES: &[&str] = &[
    "UX101", "UX102", "UX103", "UX110", "UX120", "UX130", "UX131", "UX140", "UX150",
];

/// Smallest comfortable pointer target, in dips. Desktop toolbars are denser than WCAG 2.5.8's
/// 24 CSS px (ADR-009 M9.7 notes); 20 flags the genuinely small controls.
pub const MIN_TARGET: f32 = 20.;
/// WCAG AA for normal text.
pub const MIN_CONTRAST: f32 = 4.5;
/// Frame and input-to-paint budgets in milliseconds.
pub const FRAME_BUDGET_MS: f32 = 16.7;
pub const INPUT_BUDGET_MS: f32 = 100.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    pub message: String,
    pub ids: Vec<String>,
    pub bounds: Option<Rect>,
    pub fix: String,
}

/// One action as `ui.list_actions` reports it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ActionRow {
    pub name: String,
    pub label: String,
    pub binding: Option<String>,
    pub palette: bool,
}

/// Measured responsiveness, when the host has samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    pub frame_ms: Option<f32>,
    pub input_to_paint_ms: Option<f32>,
}

/// What the rules look at besides the tree.
#[derive(Default)]
pub struct Inputs<'a> {
    pub actions: &'a [ActionRow],
    /// Names of the state fixtures that exist.
    pub fixtures: &'a [String],
    pub latency: Latency,
    /// `(path, source)` pairs for the hard-coded colour scan.
    pub sources: &'a [(String, String)],
}

/// Fixtures a view needs to prove it has an empty, loading or error state: `(view, state, fixture)`.
pub const STATE_TABLE: &[(&str, &str, &str)] = &[
    ("workspace", "empty", "empty-workspace"),
    ("response", "error", "response-error"),
    ("response", "loading", "response-loading"),
    ("folder run", "error", "folder-run-failed"),
    ("environments", "empty", "env-empty"),
    ("history", "empty", "history-empty"),
];

fn interactive(role: Role) -> bool {
    matches!(
        role,
        Role::Button | Role::Tab | Role::Input | Role::Checkbox
    )
}

fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
    for n in nodes {
        out.push(n);
        walk(&n.children, out);
    }
}

fn finding(rule: &str, sev: Severity, msg: String, ns: &[&Node], fix: &str) -> Finding {
    Finding {
        rule: rule.into(),
        severity: sev,
        message: msg,
        ids: ns.iter().map(|n| n.id.clone()).collect(),
        bounds: ns.first().map(|n| n.bounds),
        fix: fix.into(),
    }
}

/// Resolves a theme token name (`text_muted`, `surface`…).
pub fn token(t: &ThemeTokens, name: &str) -> Option<Rgba> {
    Some(match name {
        "title_bar" => t.title_bar,
        "surface" => t.surface,
        "editor" => t.editor,
        "elevated" => t.elevated,
        "gutter" => t.gutter,
        "text" => t.text,
        "text_muted" => t.text_muted,
        "text_placeholder" => t.text_placeholder,
        "accent" => t.accent,
        "created" => t.created,
        "modified" => t.modified,
        "deleted" => t.deleted,
        _ => return None,
    })
}

/// Text/background pairs the app draws, checked for every theme regardless of the tree.
/// `(text, background, minimum)`: body text needs 4.5, status and accent colours that mark
/// state rather than carry reading text need the 3:1 of WCAG 1.4.11.
pub const THEME_PAIRS: &[(&str, &str, f32)] = &[
    ("text", "surface", MIN_CONTRAST),
    ("text", "editor", MIN_CONTRAST),
    ("text", "elevated", MIN_CONTRAST),
    ("text", "title_bar", MIN_CONTRAST),
    ("text_muted", "surface", 3.0),
    ("text_muted", "editor", 3.0),
    ("text_muted", "elevated", 3.0),
    ("accent", "surface", 3.0),
    ("created", "surface", 3.0),
    ("modified", "surface", 3.0),
    ("deleted", "surface", 3.0),
];

/// Runs the selected `rules` (all when empty) over `roots`.
pub fn audit(roots: &[Node], theme: &ThemeTokens, inp: &Inputs, rules: &[String]) -> Vec<Finding> {
    let on = |r: &str| rules.is_empty() || rules.iter().any(|x| x.eq_ignore_ascii_case(r));
    let mut all = vec![];
    walk(roots, &mut all);
    let vis: Vec<&Node> = all.iter().copied().filter(|n| n.visible).collect();
    let mut out = vec![];

    if on("UX101") {
        for n in &vis {
            // Only text-bearing controls: rows and panels are clipped by scrolling by design.
            let text_bearing = matches!(
                n.role,
                Role::Text | Role::Button | Role::Tab | Role::Input | Role::Checkbox
            );
            if ((n.clipped && text_bearing) || n.meta.overflow) && !n.meta.tooltip {
                out.push(finding(
                    "UX101",
                    Severity::Warning,
                    format!("{:?} is clipped or truncated and has no tooltip", n.label),
                    &[n],
                    "show the full text in a tooltip or give the element room",
                ));
            }
        }
    }
    if on("UX102") {
        for p in &all {
            let kids: Vec<&Node> = p
                .children
                .iter()
                .filter(|c| c.visible && !matches!(c.role, Role::Modal | Role::Menu))
                .collect();
            for (i, a) in kids.iter().enumerate() {
                for b in &kids[i + 1..] {
                    let o = a.bounds.intersect(&b.bounds);
                    if o.w * o.h > 1. {
                        out.push(finding(
                            "UX102",
                            Severity::Warning,
                            format!("{} overlaps {}", a.id, b.id),
                            &[a, b],
                            "lay siblings out in a flex container so they cannot overlap",
                        ));
                    }
                }
            }
        }
    }
    if on("UX103") {
        for n in vis.iter().filter(|n| interactive(n.role)) {
            if n.bounds.w < MIN_TARGET || n.bounds.h < MIN_TARGET {
                out.push(finding(
                    "UX103",
                    Severity::Warning,
                    format!(
                        "{} is {:.0}x{:.0}, below the {MIN_TARGET:.0}x{MIN_TARGET:.0} minimum target",
                        n.id, n.bounds.w, n.bounds.h
                    ),
                    &[n],
                    "add padding or a min size so the target reaches the minimum",
                ));
            }
        }
    }
    if on("UX110") {
        for n in &vis {
            let (Some(f), Some(b)) = (&n.meta.fg, &n.meta.bg) else {
                continue;
            };
            if let (Some(fg), Some(bg)) = (token(theme, f), token(theme, b)) {
                let c = fg.contrast(bg);
                if c < MIN_CONTRAST {
                    out.push(finding(
                        "UX110",
                        Severity::Error,
                        format!(
                            "{} text {f} on {b} has contrast {c:.2} (< {MIN_CONTRAST})",
                            n.id
                        ),
                        &[n],
                        "use a stronger text token or a different surface",
                    ));
                }
            }
        }
        for (f, b, min) in THEME_PAIRS {
            if let (Some(fg), Some(bg)) = (token(theme, f), token(theme, b)) {
                let c = fg.contrast(bg);
                if c < *min {
                    out.push(Finding {
                        rule: "UX110".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "theme {:?}: {f} on {b} has contrast {c:.2} (< {min})",
                            theme.name
                        ),
                        ids: vec![format!("theme:{f}/{b}")],
                        bounds: None,
                        fix: "adjust the token or avoid this pair".into(),
                    });
                }
            }
        }
    }
    if on("UX120") {
        for (path, src) in inp.sources {
            for (line, text) in hard_coded_colors(src) {
                out.push(Finding {
                    rule: "UX120".into(),
                    severity: Severity::Warning,
                    message: format!("{path}:{line} hard-codes a colour: {text}"),
                    ids: vec![format!("{path}:{line}")],
                    bounds: None,
                    fix: "read the colour from a theme token".into(),
                });
            }
        }
    }
    if on("UX130") {
        for a in inp.actions {
            if a.binding.is_none() && !a.palette {
                out.push(Finding {
                    rule: "UX130".into(),
                    severity: Severity::Info,
                    message: format!("action {} ({}) has no binding, palette entry or menu path", a.name, a.label),
                    ids: vec![a.name.clone()],
                    bounds: None,
                    fix: "give it a binding or show it in the command palette (hidden actions may be reachable from a UI control)".into(),
                });
            }
        }
    }
    if on("UX131") {
        for n in vis.iter().filter(|n| interactive(n.role) && n.enabled) {
            if n.meta.focusable == Some(false) {
                out.push(finding(
                    "UX131",
                    Severity::Warning,
                    format!("{} is not reachable by keyboard", n.id),
                    &[n],
                    "track a focus handle and make it a tab stop",
                ));
            } else if n.meta.focusable == Some(true) && n.meta.focus_visible == Some(false) {
                out.push(finding(
                    "UX131",
                    Severity::Warning,
                    format!("{} has no visible focus indicator", n.id),
                    &[n],
                    "style the focused state with the accent border",
                ));
            }
        }
    }
    if on("UX140") {
        for (view, state, fixture) in STATE_TABLE {
            if !inp.fixtures.iter().any(|f| f == fixture) {
                out.push(Finding {
                    rule: "UX140".into(),
                    severity: Severity::Info,
                    message: format!("no {state} state fixture for {view} (expected {fixture})"),
                    ids: vec![format!("{view}:{state}")],
                    bounds: None,
                    fix: format!("add fixtures/ui-states/{fixture}.yaml and the {state} state"),
                });
            }
        }
    }
    if on("UX150") {
        let l = inp.latency;
        if let Some(ms) = l.frame_ms.filter(|m| *m > FRAME_BUDGET_MS) {
            out.push(budget("frame time", ms, FRAME_BUDGET_MS));
        }
        if let Some(ms) = l.input_to_paint_ms.filter(|m| *m > INPUT_BUDGET_MS) {
            out.push(budget("input-to-paint latency", ms, INPUT_BUDGET_MS));
        }
    }

    out.sort_by(|a, b| (&a.rule, &a.ids, &a.message).cmp(&(&b.rule, &b.ids, &b.message)));
    out.dedup();
    out
}

fn budget(what: &str, ms: f32, limit: f32) -> Finding {
    Finding {
        rule: "UX150".into(),
        severity: Severity::Warning,
        message: format!("{what} {ms:.1} ms exceeds the {limit} ms budget"),
        ids: vec![],
        bounds: None,
        fix: "profile the render path; avoid per-frame allocation and layout work".into(),
    }
}

/// Rule ids present, for summaries.
pub fn rule_set(f: &[Finding]) -> BTreeSet<&str> {
    f.iter().map(|f| f.rule.as_str()).collect()
}

/// Lines of `src` that build a colour from a literal (an rgb, rgba, hsl or hsla constructor given
/// a number), skipping comments. Test modules end the scan of a file. The constructor names are
/// assembled at run time so this file passes the app's own no-literal-colours test.
pub fn hard_coded_colors(src: &str) -> Vec<(usize, String)> {
    let pats: Vec<String> = [("rg", "b(0"), ("rg", "ba(0"), ("hs", "la("), ("hs", "l(")]
        .iter()
        .map(|(a, b)| format!("{a}{b}"))
        .collect();
    let mut out = vec![];
    for (i, line) in src.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") {
            break;
        }
        if t.starts_with("//") {
            continue;
        }
        if pats.iter().any(|p| line.contains(p.as_str())) {
            out.push((i + 1, t.trim_end().to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_tree::Meta;

    fn theme(name: &str) -> ThemeTokens {
        crate::theme::registry().get(name).unwrap().clone()
    }

    fn node(id: &str, role: Role, b: Rect) -> Node {
        Node {
            id: id.into(),
            role,
            label: id.into(),
            enabled: true,
            bounds: b,
            visible: true,
            clipped: false,
            focused: false,
            meta: Meta {
                focusable: Some(true),
                focus_visible: Some(true),
                ..Meta::default()
            },
            z: 0,
            children: vec![],
        }
    }

    fn all_fixtures() -> Vec<String> {
        STATE_TABLE.iter().map(|t| t.2.to_string()).collect()
    }

    fn run(roots: &[Node], rule: &str) -> Vec<Finding> {
        let fx = all_fixtures();
        let acts = [ActionRow {
            name: "a".into(),
            label: "A".into(),
            binding: Some("cmd-a".into()),
            palette: true,
        }];
        audit(
            roots,
            &theme("One Dark"),
            &Inputs {
                actions: &acts,
                fixtures: &fx,
                ..Default::default()
            },
            &[rule.into()],
        )
    }

    fn clean() -> Vec<Node> {
        let mut root = node("root", Role::Panel, Rect::new(0., 0., 800., 600.));
        root.children = vec![
            node("a", Role::Button, Rect::new(0., 0., 80., 30.)),
            node("b", Role::Button, Rect::new(100., 0., 80., 30.)),
        ];
        vec![root]
    }

    #[test]
    fn a_clean_tree_has_no_findings() {
        let fx = all_fixtures();
        for name in ["One Dark", "One Light"] {
            let f = audit(
                &clean(),
                &theme(name),
                &Inputs {
                    fixtures: &fx,
                    ..Default::default()
                },
                &RULES
                    .iter()
                    .filter(|r| **r != "UX110")
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>(),
            );
            assert_eq!(f, vec![], "{name}");
        }
    }

    #[test]
    fn bundled_theme_pairs_known_gaps() {
        let pairs = |n: &str| -> Vec<String> {
            audit(&[], &theme(n), &Inputs::default(), &["UX110".into()])
                .into_iter()
                .flat_map(|f| f.ids)
                .collect()
        };
        assert!(pairs("One Dark").is_empty());
        // Triage (M9.7): One Light's created/modified colours sit below 3:1 on the surface.
        assert_eq!(
            pairs("One Light"),
            ["theme:created/surface", "theme:modified/surface"]
        );
    }

    #[test]
    fn ux101_clipped_text_without_tooltip() {
        let mut t = clean();
        t[0].children[0].clipped = true;
        let f = run(&t, "UX101");
        assert_eq!(f.len(), 1);
        t[0].children[0].meta.tooltip = true;
        assert!(run(&t, "UX101").is_empty());
        t[0].children[0].meta.tooltip = false;
        t[0].children[0].clipped = false;
        t[0].children[0].meta.overflow = true;
        assert_eq!(run(&t, "UX101").len(), 1);
    }

    #[test]
    fn ux102_overlapping_siblings() {
        let mut t = clean();
        t[0].children[1].bounds = Rect::new(40., 0., 80., 30.);
        let f = run(&t, "UX102");
        assert_eq!(f[0].ids, ["a", "b"]);
    }

    #[test]
    fn ux103_small_target() {
        let mut t = clean();
        t[0].children[0].bounds = Rect::new(0., 0., 16., 16.);
        t[0].children[1].bounds = Rect::new(100., 0., 20., 20.);
        assert_eq!(run(&t, "UX103")[0].ids, ["a"]);
    }

    #[test]
    fn ux110_low_contrast_in_both_themes() {
        for name in ["One Dark", "One Light"] {
            let th = theme(name);
            let mut t = clean();
            // The surface against itself is the lowest contrast possible.
            t[0].children[0].meta.fg = Some("surface".into());
            t[0].children[0].meta.bg = Some("surface".into());
            let f = audit(&t, &th, &Inputs::default(), &["UX110".into()]);
            assert!(f.iter().any(|f| f.ids == ["a"]), "{name}");
        }
    }

    #[test]
    fn ux120_hard_coded_colour() {
        let src =
            "let a = theme.text;\n// rgb(0x000000)\nlet b = rgb(0xff0000);\n#[cfg(test)]\nrgb(0x1)";
        assert_eq!(hard_coded_colors(src).len(), 1);
        let s = [("x.rs".to_string(), src.to_string())];
        let f = audit(
            &[],
            &theme("One Dark"),
            &Inputs {
                sources: &s,
                ..Default::default()
            },
            &["UX120".into()],
        );
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn ux130_unreachable_action() {
        let acts = [ActionRow {
            name: "x".into(),
            label: "X".into(),
            binding: None,
            palette: false,
        }];
        let f = audit(
            &[],
            &theme("One Dark"),
            &Inputs {
                actions: &acts,
                ..Default::default()
            },
            &["UX130".into()],
        );
        assert_eq!(f[0].ids, ["x"]);
    }

    #[test]
    fn ux131_keyboard_and_focus() {
        let mut t = clean();
        t[0].children[0].meta.focusable = Some(false);
        t[0].children[1].meta.focus_visible = Some(false);
        let f = run(&t, "UX131");
        assert_eq!(f.len(), 2);
        // Disabled elements are exempt.
        t[0].children[0].enabled = false;
        assert_eq!(run(&t, "UX131").len(), 1);
    }

    #[test]
    fn ux140_missing_state_fixture() {
        let fx: Vec<String> = vec!["empty-workspace".into()];
        let f = audit(
            &[],
            &theme("One Dark"),
            &Inputs {
                fixtures: &fx,
                ..Default::default()
            },
            &["UX140".into()],
        );
        assert_eq!(f.len(), STATE_TABLE.len() - 1);
    }

    #[test]
    fn ux150_latency_over_budget() {
        let f = audit(
            &[],
            &theme("One Dark"),
            &Inputs {
                latency: Latency {
                    frame_ms: Some(40.),
                    input_to_paint_ms: Some(10.),
                },
                ..Default::default()
            },
            &["UX150".into()],
        );
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("frame time"));
    }

    #[test]
    fn output_is_sorted_and_deterministic() {
        let mut t = clean();
        t[0].children[0].clipped = true;
        t[0].children[1].clipped = true;
        assert_eq!(run(&t, "UX101"), run(&t, "UX101"));
    }
}
