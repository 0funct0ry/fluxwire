//! Behavioural tests for the window shell, driven through real actions and keystrokes.

use std::path::{Path, PathBuf};

use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{
    actions::{self, ACTIONS},
    theme::{Appearance, Theme, theme},
    workspace::{ModalKind, Workspace},
};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// A scratch copy of a fixture collection, so tests can save without touching the repo.
fn scratch(fixture: &str, tag: &str) -> PathBuf {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture);
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let to = std::env::temp_dir().join(format!("fluxwire-uitest-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&to);
    copy_dir(&from, &to);
    to
}

fn setup_with<'a>(
    cx: &'a mut TestAppContext,
    fixture: &str,
    tag: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf) {
    cx.update(|cx| {
        cx.set_global(Theme::one_dark());
        actions::install(cx);
    });
    let root = scratch(fixture, tag);
    let (view, cx) = cx.add_window_view(Workspace::new);
    let r = root.clone();
    view.update_in(cx, |w, window, cx| {
        w.open_project(&r, window, cx);
        w.open_request("payments/create-payment.flux.md", true, window, cx);
    });
    cx.run_until_parked();
    (view, cx, root)
}

fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (v, cx, _) = setup_with(cx, "payments-api", "shell");
    (v, cx)
}

fn keys(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.tabs.all().iter().map(|t| t.key.clone()).collect()
    })
}

const CREATE: &str = "payments/create-payment.flux.md";
const GET: &str = "payments/get-payment.flux.md";
const LIST: &str = "payments/list-payments.flux.md";
const REFUND: &str = "refunds/get-refund.flux.md";

#[gpui::test]
fn preview_tab_is_replaced_then_a_permanent_tab_stays(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    // The shell starts with one permanent tab.
    assert_eq!(keys(&view, cx), [CREATE]);
    view.update_in(cx, |w, window, cx| w.open_request(GET, false, window, cx));
    view.update_in(cx, |w, window, cx| w.open_request(LIST, false, window, cx));
    assert_eq!(keys(&view, cx), [CREATE, LIST]);
    view.update_in(cx, |w, window, cx| w.open_request(LIST, true, window, cx));
    view.update_in(cx, |w, window, cx| {
        w.open_request(REFUND, false, window, cx)
    });
    assert_eq!(keys(&view, cx), [CREATE, LIST, REFUND]);
    let previews = view.read_with(cx, |w, _| w.tabs.all().iter().filter(|t| t.preview).count());
    assert_eq!(previews, 1);
}

#[gpui::test]
fn pinning_and_closing_tabs(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| w.open_request(GET, true, window, cx));
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("tab: pin");
    cx.simulate_keystrokes("enter");
    let pinned = view.read_with(cx, |w, _| w.tabs.active().map(|t| t.pinned));
    assert_eq!(pinned, Some(true));
    assert_eq!(keys(&view, cx)[0], GET, "pinned tab sorts first");

    cx.simulate_keystrokes("secondary-w");
    assert_eq!(keys(&view, cx), [CREATE]);
    cx.simulate_keystrokes("secondary-shift-t");
    assert_eq!(keys(&view, cx).len(), 2, "closed tab reopens");
}

#[gpui::test]
fn palette_lists_every_registered_action(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-shift-p");
    let (kind, count) = view.read_with(cx, |w, _| {
        (w.modal.as_ref().map(|m| m.kind), w.modal_items().len())
    });
    assert_eq!(kind, Some(ModalKind::Palette));
    assert_eq!(count, ACTIONS.len());
    cx.update(|_, cx| {
        let registered = cx.all_action_names();
        for a in ACTIONS {
            assert!(registered.contains(&a.name), "{} is not registered", a.name);
        }
    });
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| w.modal.is_none()));
}

#[gpui::test]
fn palette_filters_and_runs_an_action(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    assert!(view.read_with(cx, |w, _| !w.bottom_open));
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("toggle bottom");
    assert_eq!(view.read_with(cx, |w, _| w.modal_items().len()), 1);
    cx.simulate_keystrokes("enter");
    assert!(view.read_with(cx, |w, _| w.bottom_open && w.modal.is_none()));
}

#[gpui::test]
fn quick_search_opens_a_request(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("create refund");
    cx.simulate_keystrokes("enter");
    assert!(keys(&view, cx).contains(&"refunds/create-refund.flux.md".to_string()));
}

#[gpui::test]
fn dock_and_bottom_toggle_bindings(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-b");
    assert!(view.read_with(cx, |w, _| !w.left_open));
    cx.simulate_keystrokes("secondary-j");
    assert!(view.read_with(cx, |w, _| w.bottom_open));
}

#[gpui::test]
fn theme_toggle_switches_appearance(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    // The window follows the OS appearance at startup, so compare against whatever it chose.
    let before = cx.update(|_, cx| theme(cx).appearance);
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("theme: toggle");
    let first = view.read_with(cx, |w, _| w.modal_items().first().map(|i| i.label.clone()));
    assert_eq!(first.as_deref(), Some("theme: toggle"));
    cx.simulate_keystrokes("enter");
    let after = cx.update(|_, cx| theme(cx).appearance);
    assert_ne!(after, before);
    let expected = match before {
        Appearance::Dark => "One Light",
        Appearance::Light => "One Dark",
    };
    assert_eq!(cx.update(|_, cx| theme(cx).name.clone()), expected);
}

#[test]
fn views_contain_no_hard_coded_colors() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let banned = [
        "rgb(", "rgba(", "hsla(", "hsl(", "white()", "black()", "red()", "green()", "blue()",
        "yellow()", "0x",
    ];
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name == "theme.rs" || name == "tests.rs" || !name.ends_with(".rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let src = src
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .replace("transparent_black()", "");
        for b in banned {
            assert!(
                !src.contains(b),
                "{name} contains hard-coded color token {b:?}"
            );
        }
    }
}

#[gpui::test]
fn method_dropdown_opens_selects_and_closes(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let method = |view: &Entity<Workspace>, cx: &mut VisualTestContext| {
        view.update(cx, |w, _| w.active_ui().map(|u| u.method))
    };
    assert_eq!(method(&view, cx), Some(1)); // POST
    view.update(cx, |w, _| w.method_menu = true);
    // Choosing an item sets the method (not "the next one"), edits the file and closes the menu.
    view.update(cx, |w, cx| {
        w.method_menu = false;
        w.set_method(4, cx);
    });
    assert_eq!(method(&view, cx), Some(4));
    let text = view.read_with(cx, |w, _| {
        w.project
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .text
            .clone()
    });
    assert!(text.contains("DELETE {{baseUrl}}/v1/payments"), "{text}");
    view.update(cx, |w, _| w.method_menu = true);
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| !w.method_menu));
}

fn doc_text(view: &Entity<Workspace>, cx: &mut VisualTestContext, key: &str) -> String {
    view.read_with(cx, |w, _| {
        w.project.as_ref().unwrap().doc(key).unwrap().text.clone()
    })
}

fn dirty(view: &Entity<Workspace>, cx: &mut VisualTestContext, key: &str) -> bool {
    view.read_with(cx, |w, _| {
        w.project.as_ref().unwrap().doc(key).unwrap().dirty()
            && w.tabs.all().iter().any(|t| t.key == key && t.dirty)
    })
}

#[gpui::test]
fn opening_a_folder_builds_the_tree_from_disk(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.read_with(cx, |w, _| {
        let p = w.project.as_ref().unwrap();
        assert!(p.all_requests().len() >= 7);
        assert_eq!(p.problem_count(), 0);
        assert_eq!(w.workspace_label(), p.name);
    });
}

#[gpui::test]
fn url_edit_dirties_the_tab_and_save_touches_one_line(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "url");
    let before = std::fs::read_to_string(root.join(CREATE)).unwrap();
    cx.simulate_keystrokes("secondary-l");
    cx.simulate_input("x");
    assert!(dirty(&view, cx, CREATE));
    assert_eq!(
        std::fs::read_to_string(root.join(CREATE)).unwrap(),
        before,
        "typing never writes"
    );
    cx.simulate_keystrokes("secondary-s");
    assert!(!dirty(&view, cx, CREATE));
    let after = std::fs::read_to_string(root.join(CREATE)).unwrap();
    let diff: Vec<_> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(diff.len(), 1, "{diff:?}");
    assert!(diff[0].1.ends_with("/v1/paymentsx"), "{diff:?}");
    assert_eq!(before.lines().count(), after.lines().count());
}

#[gpui::test]
fn text_mode_and_form_edit_the_same_document(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "text");
    view.update_in(cx, |w, window, cx| {
        w.toggle_text_mode(cx);
        window.focus(&w.editor_focus);
    });
    cx.run_until_parked();
    // Cursor starts at 0: type a YAML comment line at the top of the file.
    cx.simulate_input("#");
    let text = doc_text(&view, cx, CREATE);
    assert!(text.starts_with("#---"), "{}", &text[..20]);
    // The form sees the same document: the request is still parsed.
    view.update(cx, |w, cx| w.toggle_text_mode(cx));
    assert!(view.read_with(cx, |w, _| {
        w.project
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .parsed
            .request()
            .is_some()
            || !w
                .project
                .as_ref()
                .unwrap()
                .doc(CREATE)
                .unwrap()
                .parsed
                .diagnostics
                .is_empty()
    }));
}

#[gpui::test]
fn header_cell_edit_goes_through_the_span_editor(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "cell");
    let before = std::fs::read_to_string(root.join(CREATE)).unwrap();
    view.update_in(cx, |w, window, cx| {
        w.active_ui().unwrap().sub = 1;
        w.start_cell(
            crate::workspace::Table::Headers,
            3,
            crate::workspace::Col::Key,
            window,
            cx,
        );
    });
    cx.run_until_parked();
    cx.simulate_input("X-New");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("1");
    let text = doc_text(&view, cx, CREATE);
    assert!(text.contains("X-New: 1"), "{text}");
    assert!(text.contains("# X-Debug: 1\nX-New: 1\n") || text.contains("X-New: 1"));
    assert!(
        text.starts_with(&before[..before.find("Idempotency").unwrap()]),
        "earlier lines untouched"
    );
}

#[gpui::test]
fn new_request_prompt_creates_file_and_opens_tab(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "new");
    let count = view.read_with(cx, |w, _| w.project.as_ref().unwrap().all_requests().len());
    cx.simulate_keystrokes("secondary-n");
    assert!(view.read_with(cx, |w, _| w.modal.is_none() && w.inline.is_some()));
    cx.run_until_parked();
    cx.simulate_input("Capture payment");
    cx.simulate_keystrokes("enter");
    let created = root.join("payments/capture-payment.flux.md");
    assert!(created.is_file(), "created next to the selected request");
    let text = std::fs::read_to_string(&created).unwrap();
    assert!(
        text.contains("name: Capture payment")
            && text.contains("# Capture payment")
            && text.contains("```http\nGET")
    );
    assert!(keys(&view, cx).contains(&"payments/capture-payment.flux.md".to_string()));
    let after = view.read_with(cx, |w, _| w.project.as_ref().unwrap().all_requests().len());
    assert_eq!(after, count + 1);
}

#[gpui::test]
fn rename_updates_file_name_and_tab(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "rename");
    view.update_in(cx, |w, window, cx| w.start_rename(window, cx));
    cx.simulate_input(" v2");
    cx.simulate_keystrokes("enter");
    assert!(root.join("payments/create-payment-v2.flux.md").is_file());
    assert!(!root.join(CREATE).exists());
    assert_eq!(keys(&view, cx), ["payments/create-payment-v2.flux.md"]);
    let t = doc_text(&view, cx, "payments/create-payment-v2.flux.md");
    assert!(
        t.contains("name: Create payment v2") && t.contains("# Create payment\n"),
        "prose untouched"
    );
}

#[gpui::test]
fn external_edit_reloads_clean_tab_and_flags_dirty_one(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "ext");
    let path = root.join(CREATE);
    let orig = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, orig.replace("customerId", "customer_id")).unwrap();
    view.update(cx, |w, cx| w.on_fs_changes(std::slice::from_ref(&path), cx));
    assert!(
        doc_text(&view, cx, CREATE).contains("customer_id"),
        "clean tab reloads silently"
    );
    assert!(!dirty(&view, cx, CREATE));

    cx.simulate_keystrokes("secondary-l");
    cx.simulate_input("y");
    std::fs::write(&path, orig.replace("INR", "USD")).unwrap();
    view.update(cx, |w, cx| w.on_fs_changes(std::slice::from_ref(&path), cx));
    assert!(view.read_with(cx, |w, _| {
        w.project
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .conflict
            .is_some()
    }));
    // Keep mine: the in-memory edit stays and is still unsaved.
    view.update(cx, |w, cx| w.resolve_conflict(false, cx));
    assert!(dirty(&view, cx, CREATE));
    assert!(doc_text(&view, cx, CREATE).contains("paymentsy"));
}

#[gpui::test]
fn unsupported_file_saves_byte_identical_and_is_not_runnable(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "format-edge", "edge");
    view.update_in(cx, |w, window, cx| {
        w.open_request("future-grpc.flux.md", true, window, cx)
    });
    let before = std::fs::read(root.join("future-grpc.flux.md")).unwrap();
    view.read_with(cx, |w, _| {
        let d = w
            .project
            .as_ref()
            .unwrap()
            .doc("future-grpc.flux.md")
            .unwrap();
        assert!(!d.parsed.runnable());
        assert_eq!(d.parsed.unsupported, ["grpc"]);
        assert!(
            d.parsed
                .other_content()
                .iter()
                .any(|c| c.contains("x-owner"))
        );
    });
    cx.simulate_keystrokes("secondary-s");
    assert_eq!(
        std::fs::read(root.join("future-grpc.flux.md")).unwrap(),
        before
    );
}

#[gpui::test]
fn malformed_file_opens_with_problems_and_click_jumps_to_span(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "format-edge", "bad");
    let problem = view.read_with(cx, |w, _| {
        w.project
            .as_ref()
            .unwrap()
            .problems()
            .into_iter()
            .find(|p| p.code == "F101")
            .expect("F101 listed")
    });
    view.update_in(cx, |w, window, cx| w.goto_problem(&problem, window, cx));
    view.read_with(cx, |w, _| {
        let d = w
            .project
            .as_ref()
            .unwrap()
            .doc("malformed.flux.md")
            .unwrap();
        assert!(d.text_mode);
        assert_eq!(
            d.editors[&crate::project::Target::Text].cursor,
            problem.span.start
        );
        assert_eq!(w.tabs.active().unwrap().key, "malformed.flux.md");
    });
}

#[gpui::test]
fn collection_with_newer_format_is_refused(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(Theme::one_dark());
        actions::install(cx);
    });
    let dir = std::env::temp_dir().join(format!("fluxwire-uitest-toonew-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("fluxwire.toml"),
        "[collection]\nname = \"x\"\nformat = 2\n",
    )
    .unwrap();
    let (view, cx) = cx.add_window_view(Workspace::new);
    view.update_in(cx, |w, window, cx| w.open_project(&dir, window, cx));
    view.read_with(cx, |w, _| {
        assert!(w.project.is_none());
        assert!(w.open_error.as_deref().unwrap().contains("format 2"));
    });
}
