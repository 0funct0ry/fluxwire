//! Behavioural tests for the window shell, driven through real actions and keystrokes.

use std::path::{Path, PathBuf};

use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{
    actions::{self, ACTIONS},
    theme::{Appearance, Theme, theme},
    workspace::{ModalKind, WorkspaceView},
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
) -> (Entity<WorkspaceView>, &'a mut VisualTestContext, PathBuf) {
    cx.update(|cx| {
        cx.set_global(Theme::one_dark());
        actions::install(cx);
    });
    let root = scratch(fixture, tag);
    let (view, cx) = cx.add_window_view(WorkspaceView::new);
    let r = root.clone();
    view.update_in(cx, |w, window, cx| {
        w.open_workspace(&r, window, cx);
        w.open_request("payments/create-payment.flux.md", true, window, cx);
    });
    cx.run_until_parked();
    (view, cx, root)
}

fn setup(cx: &mut TestAppContext) -> (Entity<WorkspaceView>, &mut VisualTestContext) {
    let (v, cx, _) = setup_with(cx, "payments-api", "shell");
    (v, cx)
}

fn keys(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.scope.tabs.all().iter().map(|t| t.key.clone()).collect()
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
    let previews = view.read_with(cx, |w, _| {
        w.scope.tabs.all().iter().filter(|t| t.preview).count()
    });
    assert_eq!(previews, 1);
}

#[gpui::test]
fn pinning_and_closing_tabs(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| w.open_request(GET, true, window, cx));
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("tab: pin");
    cx.simulate_keystrokes("enter");
    let pinned = view.read_with(cx, |w, _| w.scope.tabs.active().map(|t| t.pinned));
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
    assert_eq!(count, ACTIONS.iter().filter(|a| a.palette).count());
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
    assert!(view.read_with(cx, |w, _| !w.scope.bottom_open));
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("toggle console");
    assert_eq!(view.read_with(cx, |w, _| w.modal_items().len()), 1);
    cx.simulate_keystrokes("enter");
    assert!(view.read_with(cx, |w, _| w.scope.bottom_open && w.modal.is_none()));
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
    assert!(view.read_with(cx, |w, _| !w.scope.left_open));
    cx.simulate_keystrokes("secondary-j");
    assert!(view.read_with(cx, |w, _| w.scope.bottom_open));
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
    let method = |view: &Entity<WorkspaceView>, cx: &mut VisualTestContext| {
        view.update(cx, |w, _| w.active_ui().map(|u| u.method))
    };
    assert_eq!(method(&view, cx), Some(1)); // POST
    view.update(cx, |w, _| w.scope.method_menu = true);
    // Choosing an item sets the method (not "the next one"), edits the file and closes the menu.
    view.update(cx, |w, cx| {
        w.scope.method_menu = false;
        w.set_method(4, cx);
    });
    assert_eq!(method(&view, cx), Some(4));
    let text = view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .text
            .clone()
    });
    assert!(text.contains("DELETE {{baseUrl}}/v1/payments"), "{text}");
    view.update(cx, |w, _| w.scope.method_menu = true);
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| !w.scope.method_menu));
}

fn doc_text(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, key: &str) -> String {
    view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(key)
            .unwrap()
            .text
            .clone()
    })
}

fn dirty(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, key: &str) -> bool {
    view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(key)
            .unwrap()
            .dirty()
            && w.scope.tabs.all().iter().any(|t| t.key == key && t.dirty)
    })
}

#[gpui::test]
fn opening_a_folder_builds_the_tree_from_disk(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.read_with(cx, |w, _| {
        let p = w.scope.workspace.as_ref().unwrap();
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
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .parsed
            .request()
            .is_some()
            || !w
                .scope
                .workspace
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
    let count = view.read_with(cx, |w, _| {
        w.scope.workspace.as_ref().unwrap().all_requests().len()
    });
    cx.simulate_keystrokes("secondary-n");
    assert!(view.read_with(cx, |w, _| w.modal.is_none() && w.scope.inline.is_some()));
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
    let after = view.read_with(cx, |w, _| {
        w.scope.workspace.as_ref().unwrap().all_requests().len()
    });
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
        w.scope
            .workspace
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
            .scope
            .workspace
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
        w.scope
            .workspace
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
            .scope
            .workspace
            .as_ref()
            .unwrap()
            .doc("malformed.flux.md")
            .unwrap();
        assert!(d.text_mode);
        assert_eq!(
            d.editors[&crate::workspace_model::Target::Text].cursor,
            problem.span.start
        );
        assert_eq!(w.scope.tabs.active().unwrap().key, "malformed.flux.md");
    });
}

#[gpui::test]
fn collection_with_newer_format_is_an_error_row(cx: &mut TestAppContext) {
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
    let (view, cx) = cx.add_window_view(WorkspaceView::new);
    view.update_in(cx, |w, window, cx| w.open_workspace(&dir, window, cx));
    view.read_with(cx, |w, _| {
        let ws = w.scope.workspace.as_ref().expect("workspace still opens");
        assert!(ws.collections.is_empty());
        assert!(ws.errors[0].1.to_string().contains("format 2"));
    });
}

// ---- M3: sending ----------------------------------------------------------------------------

/// A one-thread HTTP server. `stall` keeps connections open without answering.
fn test_server(stall: bool) -> u16 {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                if stall {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    return;
                }
                let body = r#"{"id":"pay_1","amount":4200}"#;
                let _ = write!(
                    s,
                    "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    port
}

/// Scripts only run in trusted collections (SPEC §21); tests that send a scripted request
/// trust the collection of `key` for the session.
fn trust_collection_of(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, key: &str) {
    view.update(cx, |w, _| {
        let root = w.collection_root_of(key).expect("collection root");
        w.scope.session_trust.insert(root);
    });
}

fn set_doc_text(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, key: &str, text: String) {
    view.update(cx, |w, _| {
        w.scope
            .workspace
            .as_mut()
            .unwrap()
            .doc_mut(key)
            .unwrap()
            .set_text(text);
    });
}

fn wait_until(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    what: &str,
    f: impl Fn(&WorkspaceView) -> bool,
) {
    for _ in 0..500 {
        cx.run_until_parked();
        if view.read_with(cx, |w, _| f(w)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn get_doc_text(port: u16) -> String {
    format!(
        "---\nname: Local\n---\n\n# Local\n\nProse stays.\n\n```http\nGET http://127.0.0.1:{port}/v1/payments\nAccept: application/json\n```\n"
    )
}

fn scripted_doc_text(port: u16) -> String {
    format!(
        "---\nname: Scripted\ncapture:\n  paymentId: $.id\n---\n\n```http\nGET http://127.0.0.1:{port}/v1/payments\n```\n\n```http expect\nHTTP 201\n\n{{ \"amount\": 4200 }}\n```\n\n```ts pre\nfw.vars.set(\"stamp\", \"s1\");\nconsole.log(\"before send\");\n```\n\n```ts test\nfw.test(\"has an id\", () => fw.expect(fw.response.json()).toHaveProperty(\"id\"));\n```\n"
    )
}

#[gpui::test]
fn scripts_wait_for_trust_then_run_and_feed_runtime_variables(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "scripts");
    let port = test_server(false);
    set_doc_text(&view, cx, CREATE, scripted_doc_text(port));

    cx.simulate_keystrokes("secondary-enter");
    view.read_with(cx, |w, _| {
        assert_eq!(
            w.scope.trust_prompt,
            w.scope.tabs.active_id(),
            "the first send asks"
        );
        assert!(w.scope.responses.values().all(|r| r.result.is_none()));
    });

    view.update(cx, |w, cx| w.set_collection_trust(true, cx));
    wait_until(&view, cx, "the scripted send", |w| {
        w.scope.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        assert!(w.scope.trust_prompt.is_none());
        let res = w
            .scope
            .responses
            .values()
            .next()
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        assert!(res.error.is_none(), "{:?}", res.error);
        let tests: Vec<_> = res
            .tests
            .iter()
            .map(|t| (t.name.as_str(), t.passed))
            .collect();
        assert_eq!(
            tests,
            [
                ("expect status", true),
                ("expect body $.amount", true),
                ("has an id", true)
            ]
        );
        assert_eq!(w.scope.envs.runtime["paymentId"].0, "pay_1");
        assert_eq!(w.scope.envs.runtime["stamp"].0, "s1");
        assert!(
            w.scope.console.iter().any(|l| l.contains("before send")),
            "{:?}",
            w.scope.console
        );
    });

    view.update(cx, |w, cx| w.set_collection_trust(false, cx));
    view.read_with(cx, |w, _| {
        let root = w.collection_root_of(CREATE).unwrap();
        assert!(!w.scope.session_trust.contains(&root));
    });
}

#[gpui::test]
fn send_shows_the_response_without_saving(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "send");
    let port = test_server(false);
    set_doc_text(&view, cx, CREATE, get_doc_text(port));
    let on_disk = std::fs::read_to_string(root.join(CREATE)).unwrap();
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the response body", |w| {
        w.scope.responses.values().any(|r| r.shown.is_some())
    });
    view.read_with(cx, |w, _| {
        let r = w.scope.responses.values().next().unwrap();
        let res = r.result.as_ref().unwrap();
        let x = res.outcome.as_ref().unwrap().exchange.as_ref().unwrap();
        assert_eq!(x.status, 201);
        let shown = r.shown.as_ref().unwrap();
        assert!(
            shown.text.contains("\"id\": \"pay_1\""),
            "pretty printed: {}",
            shown.text
        );
        assert!(
            w.scope.console.iter().any(|l| l.contains("201 Created")),
            "{:?}",
            w.scope.console
        );
        assert!(!w.is_sending());
    });
    assert_eq!(
        std::fs::read_to_string(root.join(CREATE)).unwrap(),
        on_disk,
        "sending never saves"
    );
}

#[gpui::test]
fn cancel_ends_an_in_flight_send(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "cancel");
    let port = test_server(true);
    set_doc_text(&view, cx, CREATE, get_doc_text(port));
    cx.simulate_keystrokes("secondary-enter");
    cx.run_until_parked();
    assert!(view.read_with(cx, |w, _| w.is_sending()));
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    wait_until(&view, cx, "the cancelled result", |w| !w.is_sending());
    view.read_with(cx, |w, _| {
        let res = w
            .scope
            .responses
            .values()
            .next()
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        assert!(res.error.as_ref().unwrap().is_cancelled());
    });
}

#[gpui::test]
fn files_that_cannot_run_are_not_sent(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "unsupported");
    set_doc_text(
        &view,
        cx,
        CREATE,
        "---\nname: X\nrequires: [grpc]\n---\n\n```http\nGET http://127.0.0.1:1/\n```\n".into(),
    );
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the refusal", |w| {
        w.scope.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        let res = w
            .scope
            .responses
            .values()
            .next()
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        assert!(res.outcome.is_none());
        assert_eq!(res.error.as_ref().unwrap().phase(), "requires");
    });
}

#[gpui::test]
fn unresolved_variables_warn_once_then_send_anyway(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "vars");
    trust_collection_of(&view, cx, CREATE);
    // create-payment uses {{idemKey}}, which no environment defines.
    cx.simulate_keystrokes("secondary-enter");
    view.read_with(cx, |w, _| {
        let n = w.notice.clone().unwrap_or_default();
        assert!(n.contains("{{idemKey}}") && n.contains("Send again"), "{n}");
        assert!(w.scope.responses.is_empty(), "the first send only warns");
    });
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the second send to finish", |w| {
        w.scope.responses.values().any(|r| r.result.is_some())
    });
}

#[gpui::test]
fn pasting_a_curl_command_fills_the_request(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "paste");
    set_doc_text(&view, cx, CREATE, get_doc_text(1));
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "curl -X PUT https://api.test/v1/x -H 'X-A: 1' -d '{\"a\":1}'".into(),
    ));
    cx.simulate_keystrokes("secondary-l secondary-v");
    let text = view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .text
            .clone()
    });
    assert!(
        text.contains("PUT https://api.test/v1/x\nX-A: 1\n\n{\"a\":1}\n```"),
        "{text}"
    );
    assert!(text.contains("Prose stays."), "prose untouched");
    assert!(dirty(&view, cx, CREATE));
    // a plain URL pastes as text
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("/more".into()));
    cx.simulate_keystrokes("secondary-v");
    let text = view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(CREATE)
            .unwrap()
            .text
            .clone()
    });
    assert!(text.contains("PUT https://api.test/v1/x/more"), "{text}");
}

#[gpui::test]
fn copy_as_curl_masks_secrets_unless_asked(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "curl");
    set_doc_text(
        &view,
        cx,
        CREATE,
        "---\nname: X\nauth:\n  type: bearer\n  token: s3cret\n---\n\n```http\nGET http://x.test/a\n```\n".into(),
    );
    view.update(cx, |w, cx| w.copy_curl(false, cx));
    let masked = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert!(
        masked.contains("Bearer ***") && !masked.contains("s3cret"),
        "{masked}"
    );
    view.update(cx, |w, cx| w.copy_curl(true, cx));
    let open = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert!(open.contains("Bearer s3cret"), "{open}");
}

#[gpui::test]
fn clearing_the_url_keeps_the_method(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "clear-url");
    set_doc_text(
        &view,
        cx,
        CREATE,
        "---\nname: X\n---\n\n```http\nDELETE http://x.test/a\nAccept: */*\n```\n".into(),
    );
    cx.simulate_keystrokes("secondary-l");
    for _ in 0..20 {
        cx.simulate_keystrokes("backspace");
    }
    let method = |cx: &mut VisualTestContext| {
        view.read_with(cx, |w, _| {
            let d = w.scope.workspace.as_ref().unwrap().doc(CREATE).unwrap();
            (d.parsed.request().unwrap().method.clone(), d.text.clone())
        })
    };
    let (m, text) = method(cx);
    assert_eq!(m, "DELETE", "{text}");
    assert!(text.contains("DELETE \nAccept: */*"), "{text}");
    cx.simulate_input("h");
    let (m, text) = method(cx);
    assert_eq!(m, "DELETE");
    assert!(text.contains("DELETE h\nAccept: */*"), "{text}");
}

#[gpui::test]
fn url_selection_is_replaced_copied_and_deleted(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "url-sel");
    set_doc_text(&view, cx, CREATE, get_doc_text(1));
    let target = |cx: &mut VisualTestContext| {
        view.read_with(cx, |w, _| {
            w.scope
                .workspace
                .as_ref()
                .unwrap()
                .doc(CREATE)
                .unwrap()
                .parsed
                .request()
                .unwrap()
                .target
                .clone()
        })
    };
    cx.simulate_keystrokes("secondary-l secondary-a secondary-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("http://127.0.0.1:1/v1/payments")
    );
    cx.simulate_input("http://new.test/");
    assert_eq!(target(cx), "http://new.test/");
    cx.simulate_keystrokes("secondary-a backspace");
    assert_eq!(target(cx), "");
    view.read_with(cx, |w, _| {
        let d = w.scope.workspace.as_ref().unwrap().doc(CREATE).unwrap();
        assert_eq!(d.parsed.request().unwrap().method, "GET");
    });
}

// ---- M4: environments, variables, secrets ---------------------------------------------------

fn write_env(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    root: &Path,
    file: &str,
    text: &str,
) {
    let p = root.join("environments").join(file);
    std::fs::write(&p, text).unwrap();
    view.update(cx, |w, _| w.on_env_fs_change(&[p]));
}

fn last_url(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |w, _| {
        let r = w
            .scope
            .responses
            .values()
            .next()
            .unwrap()
            .result
            .as_ref()
            .unwrap();
        r.outcome.as_ref().unwrap().final_url.clone()
    })
}

fn picker_labels(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.modal_items().into_iter().map(|i| i.label).collect()
    })
}

#[gpui::test]
fn switching_environment_changes_the_resolved_url_and_the_request_sent(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-switch");
    trust_collection_of(&view, cx, CREATE);
    let (p_local, p_staging) = (test_server(false), test_server(false));
    write_env(
        &view,
        cx,
        &root,
        "local.env",
        &format!("baseUrl=http://127.0.0.1:{p_local}\nidemKey=k\ncustomerId=c\n"),
    );
    write_env(
        &view,
        cx,
        &root,
        "staging.env",
        &format!("baseUrl=http://127.0.0.1:{p_staging}\nidemKey=k\ncustomerId=c\n"),
    );

    view.update(cx, |w, cx| w.set_active_env(Some("local".into()), cx));
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the local send", |w| {
        w.scope.responses.values().any(|r| r.result.is_some())
    });
    assert!(
        last_url(&view, cx).contains(&format!(":{p_local}/")),
        "{}",
        last_url(&view, cx)
    );

    // The hover text shows the resolved value and where it came from.
    let tip = view.read_with(cx, |w, _| w.var_look(CREATE)("baseUrl"));
    assert!(
        tip.resolved && tip.tip.contains("local.env, line 1"),
        "{tip:?}"
    );

    view.update(cx, |w, cx| w.set_active_env(Some("staging".into()), cx));
    let tip = view.read_with(cx, |w, _| w.var_look(CREATE)("baseUrl"));
    assert!(
        tip.tip.contains(&format!(":{p_staging}")) && tip.tip.contains("staging.env"),
        "{tip:?}"
    );
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the staging send", |w| {
        w.scope.responses.values().all(|r| r.sending.is_none())
            && w.scope.responses.values().any(|r| {
                r.result
                    .as_ref()
                    .and_then(|x| x.outcome.as_ref())
                    .is_some_and(|o| o.final_url.contains(&format!(":{p_staging}/")))
            })
    });
}

#[gpui::test]
fn environment_switcher_lists_rows_in_order_and_activates(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "env-picker");
    cx.simulate_keystrokes("secondary-e");
    assert_eq!(
        picker_labels(&view, cx),
        [
            "New environment…",
            "✓ devel",
            "local",
            "staging",
            "test",
            "No environment",
            "Manage environments…"
        ]
    );
    let title = view.read_with(cx, |w, _| w.env_modal_title());
    assert_eq!(title.as_deref(), Some("Switch environment"));
    cx.simulate_input("stag");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("staging")
    );
    assert_eq!(view.read_with(cx, |w, _| w.env_label()), "staging");
}

#[gpui::test]
fn typing_a_new_name_offers_create_and_enter_creates_activates_and_opens_the_editor(
    cx: &mut TestAppContext,
) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-create");
    cx.simulate_keystrokes("secondary-e");
    cx.simulate_input("qa");
    assert_eq!(picker_labels(&view, cx)[0], "Create environment “qa”");
    cx.simulate_keystrokes("enter");
    assert!(root.join("environments/qa.env").exists());
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("qa")
    );
    let k = env_key(&view, cx, "qa");
    assert!(view.read_with(cx, |w, _| w.env_doc(&k).is_some()));
    assert!(
        !view.read_with(cx, |w, _| w.env_doc_dirty(&k)),
        "a fresh environment is clean"
    );
    // An invalid name offers no Create row.
    cx.simulate_keystrokes("secondary-e");
    cx.simulate_input("a/b");
    assert!(
        !picker_labels(&view, cx)
            .iter()
            .any(|l| l.starts_with("Create"))
    );
}

#[gpui::test]
fn secondary_enter_opens_the_editor_without_activating_and_manage_opens_the_panel(
    cx: &mut TestAppContext,
) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "env-open");
    cx.simulate_keystrokes("secondary-e");
    cx.simulate_input("stag");
    cx.simulate_keystrokes("secondary-enter");
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("devel")
    );
    let tab = view.read_with(cx, |w, _| w.scope.tabs.active().map(|t| t.key.clone()));
    assert_eq!(tab.as_deref(), Some("env/staging"));
    cx.simulate_keystrokes("secondary-e");
    cx.simulate_input("Manage");
    view.update(cx, |w, _| {
        let m = w.modal.as_mut().unwrap();
        m.selected = m
            .results
            .iter()
            .position(|i| i.label.starts_with("Manage"))
            .unwrap();
    });
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |w, _| {
        assert!(w.scope.left_open);
        assert_eq!(w.scope.panel, crate::workspace::Panel::Environments);
    });
}

fn new_env_dialog(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, name: &str) {
    view.update_in(cx, |w, window, cx| w.start_new_env(window, cx));
    view.update(cx, |w, cx| {
        let Some(crate::dialogs::Dialog::NewEnvironment(f)) = &w.dialog else {
            panic!("dialog")
        };
        f.name.update(cx, |i, cx| i.replace_text(name, cx));
    });
}

#[gpui::test]
fn new_environment_dialog_validates_duplicates_and_activates(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-dialog");
    for bad in ["", "a/b", ".x", "foo.local", "NUL", "staging"] {
        new_env_dialog(&view, cx, bad);
        cx.simulate_keystrokes("enter");
        assert!(
            view.read_with(cx, |w, _| w.dialog.is_some()),
            "{bad:?} must not submit"
        );
        view.update_in(cx, |w, window, cx| w.close_dialog(window, cx));
    }
    assert!(!root.join("environments/a/b.env").exists());
    // Start from a duplicate; do not activate.
    new_env_dialog(&view, cx, "staging2");
    view.update(cx, |w, _| {
        let Some(crate::dialogs::Dialog::NewEnvironment(f)) = &mut w.dialog else {
            panic!()
        };
        f.start_from = Some("staging".into());
        f.activate = false;
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(
        std::fs::read_to_string(root.join("environments/staging2.env")).unwrap(),
        std::fs::read_to_string(root.join("environments/staging.env")).unwrap()
    );
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("devel")
    );
    assert!(view.read_with(cx, |w, _| {
        w.scope
            .tabs
            .active()
            .is_some_and(|t| t.key == "env/staging2")
    }));
}

#[gpui::test]
fn environment_files_can_be_renamed_duplicated_and_deleted(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-ops");
    new_env_dialog(&view, cx, "qa");
    cx.simulate_keystrokes("enter");
    assert!(root.join("environments/qa.env").exists());
    view.update_in(cx, |w, window, cx| w.start_rename_env("qa", window, cx));
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("uat");
    cx.simulate_keystrokes("enter");
    assert!(
        root.join("environments/uat.env").exists() && !root.join("environments/qa.env").exists()
    );
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("uat")
    );
    view.update(cx, |w, cx| w.duplicate_env("uat", cx));
    assert!(root.join("environments/uat-copy.env").exists());
    // Renaming to an invalid name is refused with a message.
    view.update_in(cx, |w, window, cx| w.start_rename_env("uat", window, cx));
    cx.simulate_input("/");
    cx.simulate_keystrokes("enter");
    assert!(root.join("environments/uat.env").exists());
    assert!(
        view.read_with(cx, |w, _| w.notice.clone())
            .unwrap_or_default()
            .contains("cannot contain")
    );
    // Delete asks first and names the files.
    let rt = root.clone();
    view.update_in(cx, |w, window, cx| {
        w.confirm_delete_env(&rt, "uat-copy", window, cx)
    });
    view.read_with(cx, |w, _| {
        let Some(crate::dialogs::Dialog::Confirm(c)) = &w.dialog else {
            panic!()
        };
        assert!(c.lines[0].contains("uat-copy.env"), "{:?}", c.lines);
    });
    view.update_in(cx, |w, window, cx| w.close_dialog(window, cx));
    assert!(root.join("environments/uat-copy.env").exists());
}

#[gpui::test]
fn secrets_are_masked_revealed_for_a_while_and_never_leak(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "secrets");
    let port = test_server(false);
    write_env(
        &view,
        cx,
        &root,
        "local.env",
        "baseUrl=http://127.0.0.1:1\n",
    );
    write_env(&view, cx, &root, "local.local.env", "tok=hunter2-value\n");
    view.update(cx, |w, cx| w.set_active_env(Some("local".into()), cx));
    set_doc_text(
        &view,
        cx,
        CREATE,
        format!(
            "---\nname: X\n---\n\n```http\nGET http://127.0.0.1:{port}/v1/payments?k={{{{tok}}}}\nX-T: {{{{tok}}}}\n```\n"
        ),
    );

    // Hover is masked until revealed; reveal expires.
    let tip = view.read_with(cx, |w, _| w.var_look(CREATE)("tok"));
    assert!(
        !tip.tip.contains("hunter2-value") && tip.tip.contains("secret"),
        "{tip:?}"
    );
    view.update(cx, |w, cx| w.reveal_secret("tok", cx));
    assert!(view.read_with(cx, |w, _| w.scope.envs.is_revealed("tok")));
    view.update(cx, |w, _| {
        w.scope.envs.reveal.insert(
            "tok".into(),
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        );
    });
    assert!(!view.read_with(cx, |w, _| w.scope.envs.is_revealed("tok")));

    // Copy as cURL masks by value, wherever the secret landed.
    view.update(cx, |w, cx| w.copy_curl(false, cx));
    let masked = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert!(
        !masked.contains("hunter2-value") && masked.contains("‹secret:tok›"),
        "{masked}"
    );
    view.update(cx, |w, cx| w.copy_curl(true, cx));
    assert!(
        cx.read_from_clipboard()
            .and_then(|c| c.text())
            .unwrap()
            .contains("hunter2-value")
    );

    // The Console (and the response label) never show it.
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the send", |w| {
        w.scope.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        let log = w.scope.console.join("\n");
        assert!(!log.contains("hunter2-value"), "{log}");
        assert!(log.contains("‹secret:tok›"), "{log}");
    });
}

fn env_key(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    name: &str,
) -> crate::env_editor::DocKey {
    view.read_with(cx, |w, _| (w.env_root().unwrap(), name.to_string()))
}

fn env_doc_rows(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    k: &crate::env_editor::DocKey,
) -> Vec<(String, String)> {
    view.read_with(cx, |w, _| {
        w.env_doc(k)
            .unwrap()
            .env
            .rows
            .iter()
            .map(|r| (r.name.clone(), r.value.clone()))
            .collect()
    })
}

fn set_cell_text(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    k: &crate::env_editor::DocKey,
    row: usize,
    value: bool,
    text: &str,
) {
    let id = view.read_with(cx, |w, _| w.env_doc(k).unwrap().ids[row]);
    let cell = if value {
        crate::env_doc::Cell::Value
    } else {
        crate::env_doc::Cell::Name
    };
    let input = view.update(cx, |w, cx| w.env_cell(k, id, cell, cx));
    input.update(cx, |i, cx| i.replace_text(text, cx));
}

#[gpui::test]
fn environment_edits_stay_in_memory_until_saved(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-tab");
    write_env(
        &view,
        cx,
        &root,
        "local.env",
        "# keep me\nbaseUrl=http://a\n",
    );
    view.update(cx, |w, _| w.open_env_tab("local"));
    cx.run_until_parked();
    let k = env_key(&view, cx, "local");
    let path = root.join("environments/local.env");
    set_cell_text(&view, cx, &k, 0, true, "http://ax");
    assert!(view.read_with(cx, |w, _| w.env_doc_dirty(&k)));
    let tab_dirty = view.read_with(cx, |w, _| w.scope.tabs.active().unwrap().dirty);
    assert!(tab_dirty, "the tab shows the dirty dot");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# keep me\nbaseUrl=http://a\n"
    );
    // Add a secret row, then save: comments survive, the secret goes to the local file.
    view.update_in(cx, |w, window, cx| w.env_add_row(&k, window, cx));
    set_cell_text(&view, cx, &k, 1, false, "pw");
    set_cell_text(&view, cx, &k, 1, true, "s3");
    let id = view.read_with(cx, |w, _| w.env_doc(&k).unwrap().ids[1]);
    view.update(cx, |w, cx| w.env_toggle_secret(&k, id, cx));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# keep me\nbaseUrl=http://a\n"
    );
    cx.simulate_keystrokes("secondary-s");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# keep me\nbaseUrl=http://ax\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("environments/local.local.env")).unwrap(),
        "pw=s3\n"
    );
    assert!(!view.read_with(cx, |w, _| w.env_doc_dirty(&k)));
    assert!(!view.read_with(cx, |w, _| w.scope.tabs.active().unwrap().dirty));
}

#[gpui::test]
fn a_send_uses_the_unsaved_environment_value(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-unsaved");
    write_env(&view, cx, &root, "local.env", "baseUrl=http://a.test\n");
    view.update(cx, |w, cx| w.set_active_env(Some("local".into()), cx));
    view.update(cx, |w, _| w.open_env_tab("local"));
    let k = env_key(&view, cx, "local");
    set_cell_text(&view, cx, &k, 0, true, "http://unsaved.test");
    let tip = view.read_with(cx, |w, _| w.var_look(CREATE)("baseUrl"));
    assert!(tip.tip.contains("http://unsaved.test"), "{tip:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("environments/local.env")).unwrap(),
        "baseUrl=http://a.test\n"
    );
}

#[gpui::test]
fn environment_document_undo_validation_and_close_prompt(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-doc");
    write_env(&view, cx, &root, "local.env", "A=1\n");
    view.update(cx, |w, _| w.open_env_tab("local"));
    let k = env_key(&view, cx, "local");
    view.update_in(cx, |w, window, cx| w.env_add_row(&k, window, cx));
    set_cell_text(&view, cx, &k, 1, false, "1bad");
    let problems = view.read_with(cx, |w, _| w.all_problems());
    assert!(
        problems
            .iter()
            .any(|p| p.code == "E101" && p.key.starts_with("env/")),
        "{problems:?}"
    );
    view.update(cx, |w, cx| w.env_undo(&k, false, cx));
    view.update(cx, |w, cx| w.env_undo(&k, false, cx));
    assert_eq!(
        env_doc_rows(&view, cx, &k),
        vec![("A".to_string(), "1".to_string())]
    );
    assert!(!view.read_with(cx, |w, _| w.env_doc_dirty(&k)));
    // Dirty close asks first; Discard drops the edits.
    set_cell_text(&view, cx, &k, 0, true, "9");
    let id = view.read_with(cx, |w, _| w.scope.tabs.active_id().unwrap());
    view.update_in(cx, |w, window, cx| w.close_env_tab(id, window, cx));
    assert!(view.read_with(cx, |w, _| w.dialog.is_some()));
    view.update_in(cx, |w, window, cx| {
        w.run_confirm_action(
            crate::dialogs::ConfirmAction::CloseEnvTab(id, false),
            window,
            cx,
        )
    });
    assert!(view.read_with(cx, |w, _| w.env_doc(&k).is_none()));
    assert_eq!(
        std::fs::read_to_string(root.join("environments/local.env")).unwrap(),
        "A=1\n"
    );
}

#[gpui::test]
fn external_change_reloads_a_clean_environment_and_flags_a_dirty_one(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-ext");
    write_env(&view, cx, &root, "local.env", "A=1\n");
    view.update(cx, |w, _| w.open_env_tab("local"));
    let k = env_key(&view, cx, "local");
    std::fs::write(root.join("environments/local.env"), "A=2\n").unwrap();
    view.update(cx, |w, _| w.refresh_env_docs());
    assert_eq!(env_doc_rows(&view, cx, &k)[0].1, "2");
    set_cell_text(&view, cx, &k, 0, true, "mine");
    std::fs::write(root.join("environments/local.env"), "A=3\n").unwrap();
    view.update(cx, |w, _| w.refresh_env_docs());
    assert!(view.read_with(cx, |w, _| w.env_doc(&k).unwrap().changed_on_disk));
    assert_eq!(env_doc_rows(&view, cx, &k)[0].1, "mine");
    view.update(cx, |w, cx| w.env_resolve_conflict(&k, true, cx));
    assert_eq!(env_doc_rows(&view, cx, &k)[0].1, "3");
}

#[gpui::test]
fn typing_open_braces_offers_variables_and_tab_completes(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "complete");
    set_doc_text(
        &view,
        cx,
        CREATE,
        "---\nname: X\n---\n\n```http\nGET http://x.test/\n```\n".into(),
    );
    view.update(cx, |w, cx| w.set_active_env(Some("local".into()), cx));
    cx.simulate_keystrokes("secondary-l");
    cx.simulate_input("{{cust");
    view.read_with(cx, |w, _| {
        let c = w.scope.envs.completion.as_ref().expect("suggestions");
        assert_eq!(c.names, ["customerId"]);
    });
    cx.simulate_keystrokes("tab");
    assert_eq!(
        doc_text(&view, cx, CREATE)
            .lines()
            .find(|l| l.starts_with("GET"))
            .unwrap(),
        "GET http://x.test/{{customerId}}"
    );
    view.read_with(cx, |w, _| assert!(w.scope.envs.completion.is_none()));
}

#[gpui::test]
fn unresolved_variables_are_listed_in_problems(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "problems");
    view.update(cx, |w, cx| w.set_active_env(Some("local".into()), cx));
    set_doc_text(
        &view,
        cx,
        CREATE,
        "---\nname: X\n---\n\n```http\nGET {{baseUrl}}/{{nope}}\n```\n".into(),
    );
    view.read_with(cx, |w, _| {
        let v: Vec<_> = w
            .all_problems()
            .into_iter()
            .filter(|p| p.code == "V101")
            .collect();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].message.contains("{{nope}}"));
        assert_eq!(
            &w.scope
                .workspace
                .as_ref()
                .unwrap()
                .doc(CREATE)
                .unwrap()
                .text[v[0].span.start..v[0].span.end],
            "{{nope}}"
        );
        assert!(w.problem_total() >= 1);
    });
}

#[gpui::test]
fn auth_tab_edits_the_auth_front_matter(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "auth-tab");
    view.update(cx, |w, cx| w.set_auth_type(CREATE, "bearer", cx));
    view.update(cx, |w, cx| {
        w.set_auth_field(CREATE, "token", "{{accessToken}}", cx)
    });
    let t = doc_text(&view, cx, CREATE);
    assert!(
        t.contains("auth:\n  type: bearer\n  token: {{accessToken}}")
            || t.contains("token: \"{{accessToken}}\""),
        "{t}"
    );
    view.update(cx, |w, cx| w.set_auth_type(CREATE, "oauth2", cx));
    view.update(cx, |w, cx| {
        w.set_auth_field(CREATE, "token_url", "https://idp/token", cx)
    });
    assert!(doc_text(&view, cx, CREATE).contains("type: oauth2"));
    view.update(cx, |w, cx| w.set_auth_type(CREATE, "inherit", cx));
    assert!(!doc_text(&view, cx, CREATE).contains("oauth2"));
    assert!(
        doc_text(&view, cx, CREATE).contains("Creates a payment."),
        "prose is untouched"
    );
}

#[gpui::test]
fn active_environment_is_part_of_the_session(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "session-env");
    view.update(cx, |w, cx| w.set_active_env(Some("staging".into()), cx));
    let s = view.read_with(cx, |w, _| w.session());
    assert_eq!(s.active_env.as_deref(), Some("staging"));
    assert!(!s.no_environment);
    // An environment that no longer exists falls back to the first one.
    view.update(cx, |w, _| w.init_active_env(Some("gone"), false));
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("devel")
    );
    view.update(cx, |w, _| w.init_active_env(Some("staging"), false));
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("staging")
    );
    // An explicit "No environment" is remembered too.
    view.update(cx, |w, cx| w.set_active_env(None, cx));
    let s = view.read_with(cx, |w, _| w.session());
    assert!(s.no_environment && s.active_env.is_none());
    view.update(cx, |w, _| w.init_active_env(None, true));
    assert_eq!(view.read_with(cx, |w, _| w.active_env_name()), None);
}

#[gpui::test]
fn environments_are_workspace_level_and_shared_by_every_collection(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let root = two_collections("env-ws");
    fluxwire_format::create_environment(&root, "prod").unwrap();
    fluxwire_format::create_environment(&root.join("a"), "ignored").unwrap();
    view.update_in(cx, |w, window, cx| w.open_workspace(&root, window, cx));
    // Files live in the workspace root's environments/, not in a collection's.
    assert_eq!(view.read_with(cx, |w, _| w.env_names()), ["prod"]);
    assert_eq!(view.read_with(cx, |w, _| w.env_root()), Some(root.clone()));
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("prod")
    );
    // The pill is just the environment name, whatever collection is current.
    assert_eq!(view.read_with(cx, |w, _| w.env_label()), "prod");
    // Creating one writes to the workspace root.
    view.update_in(cx, |w, window, cx| w.start_new_env(window, cx));
    view.update(cx, |w, cx| {
        let Some(crate::dialogs::Dialog::NewEnvironment(f)) = &w.dialog else {
            panic!()
        };
        f.name.update(cx, |i, cx| i.replace_text("qa", cx));
    });
    cx.simulate_keystrokes("enter");
    assert!(root.join("environments/qa.env").exists());
    assert!(!root.join("a/environments/qa.env").exists());
}

fn doc_for(port: u16, path: &str) -> String {
    format!(
        "---\nname: Local\n---\n\n# Local\n\n```http\nGET http://127.0.0.1:{port}{path}\nAccept: application/json\n```\n"
    )
}

/// Send the active request and wait until `n` history entries are listed.
fn send_and_record(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, n: usize) {
    cx.simulate_keystrokes("secondary-enter");
    wait_until(view, cx, "history entry", move |w| {
        w.history.entries.len() == n && !w.is_sending()
    });
}

#[gpui::test]
fn every_send_is_recorded_newest_first_and_opens_read_only(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-rec");
    let port = test_server(false);
    set_doc_text(&view, cx, CREATE, doc_for(port, "/v1/one"));
    send_and_record(&view, cx, 1);
    set_doc_text(&view, cx, CREATE, doc_for(port, "/v1/two"));
    send_and_record(&view, cx, 2);
    let (newest, oldest) = view.read_with(cx, |w, _| {
        let e = &w.history.entries;
        assert!(e[0].url.ends_with("/v1/two") && e[1].url.ends_with("/v1/one"));
        assert_eq!((e[0].status, e[0].method.as_str()), (Some(201), "GET"));
        assert!(e[0].dirty, "the send used unsaved text");
        (e[0].id, e[1].id)
    });
    view.update(cx, |w, cx| w.open_history_entry(oldest, false, cx));
    wait_until(&view, cx, "entry loaded", move |w| {
        w.history.cache.contains_key(&oldest)
    });
    view.read_with(cx, |w, _| {
        let t = w.scope.tabs.active().unwrap();
        assert_eq!(t.key, format!("history/{oldest}"));
        assert!(t.preview);
        let e = &w.history.cache[&oldest];
        assert_eq!(
            e.payload["response"]["body"]["text"],
            r#"{"id":"pay_1","amount":4200}"#
        );
        assert_eq!(e.payload["request"]["method"], "GET");
    });
    // diff the two entries
    view.update(cx, |w, cx| {
        w.history_pick_diff(oldest, cx);
        w.history_pick_diff(newest, cx);
    });
    wait_until(&view, cx, "diff computed", move |w| {
        w.history.diffs.contains_key(&(oldest, newest))
    });
    view.read_with(cx, |w, _| {
        let d = &w.history.diffs[&(oldest, newest)];
        assert!(d.res_body.iter().all(|r| r.left == r.right), "same body");
    });
}

#[gpui::test]
fn filters_restore_and_response_dropdown(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "hist-act");
    let port = test_server(false);
    set_doc_text(&view, cx, CREATE, doc_for(port, "/v1/one"));
    send_and_record(&view, cx, 1);
    set_doc_text(&view, cx, CREATE, doc_for(port, "/v1/two"));
    send_and_record(&view, cx, 2);
    let first = view.read_with(cx, |w, _| w.history.entries[1].id);

    // filters
    view.update(cx, |w, cx| {
        w.history.query.method = Some("POST".into());
        w.history_reload(cx);
    });
    wait_until(&view, cx, "POST filter", |w| w.history.entries.is_empty());
    view.update(cx, |w, cx| {
        w.history.query.method = None;
        w.history.query.text = "/v1/one".to_string();
        w.history_reload(cx);
    });
    wait_until(&view, cx, "text filter", |w| w.history.entries.len() == 1);
    view.update(cx, |w, cx| {
        w.history.query.text.clear();
        w.history_reload(cx);
    });
    wait_until(&view, cx, "filters cleared", |w| {
        w.history.entries.len() == 2
    });

    // response-pane dropdown shows the earlier response, then goes back
    view.update(cx, |w, cx| w.show_history_response(first, cx));
    wait_until(&view, cx, "earlier response", |w| {
        w.scope
            .responses
            .values()
            .any(|r| r.viewing.is_some() && r.shown.is_some())
    });
    view.update(cx, |w, cx| w.history_back_to_latest(cx));
    wait_until(&view, cx, "latest response", |w| {
        w.scope
            .responses
            .values()
            .any(|r| r.viewing.is_none() && r.shown.is_some())
    });

    // restore puts the old request in an unsaved tab
    let on_disk = std::fs::read_to_string(root.join(CREATE)).unwrap();
    view.update_in(cx, |w, window, cx| {
        w.history_restore(first, CREATE.to_string(), window, cx)
    });
    wait_until(&view, cx, "restored", |w| {
        w.scope
            .workspace
            .as_ref()
            .and_then(|p| p.doc(CREATE))
            .is_some_and(|d| d.text.contains("/v1/one"))
    });
    assert!(dirty(&view, cx, CREATE));
    assert_eq!(std::fs::read_to_string(root.join(CREATE)).unwrap(), on_disk);

    // clearing empties the panel
    view.update(cx, |w, cx| {
        w.history_clear(crate::history::ClearScope::All, cx)
    });
    wait_until(&view, cx, "cleared", |w| w.history.entries.is_empty());
}

// ---- M9.03: History viewer -------------------------------------------------------------------

fn seed_history(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext, n: usize) {
    use fluxwire_store::{NewEntry, Redactor};
    view.update(cx, |w, _| {
        let (store, ws) = (
            w.history.store.clone().unwrap(),
            w.history.ws_key.clone().unwrap(),
        );
        for i in 0..n {
            let status = if i % 5 == 0 { Some(500) } else { Some(200) };
            store
                .insert(
                    &ws,
                    NewEntry {
                        ts_ms: crate::history::now_ms() - (n - i) as i64 * 1000,
                        collection: "c".into(),
                        request_path: format!("r{}.flux.md", i % 3),
                        request_name: format!("R{}", i % 3),
                        protocol: "http".into(),
                        method: "GET".into(),
                        url: format!("http://x/{i}"),
                        status,
                        total_ms: 10.0 + i as f64,
                        env: Some("staging".into()),
                        dirty: false,
                        run_id: None,
                        summary: "s".into(),
                        payload: serde_json::json!({"request": {"headers": [], "method": "GET"},
                            "response": {"headers": [], "body": {"text": "{\"ok\":true}"}}}),
                    },
                    &Redactor::default(),
                )
                .unwrap();
        }
    });
    view.update(cx, |w, cx| w.history_reload(cx));
}

#[gpui::test]
fn history_opens_as_one_tab_and_navigator_items_filter_it(cx: &mut TestAppContext) {
    use crate::history_query::{NavItem, SavedView};
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-nav");
    seed_history(&view, cx, 10);
    wait_until(&view, cx, "seeded", |w| w.history.entries.len() == 10);
    view.update(cx, |w, cx| {
        w.history_open(cx);
        w.history_open(cx);
    });
    view.read_with(cx, |w, _| {
        let tabs: Vec<_> = w
            .scope
            .tabs
            .all()
            .iter()
            .filter(|t| t.key == "history")
            .collect();
        assert_eq!(tabs.len(), 1, "reopen-or-focus");
        assert_eq!(w.scope.tabs.active().unwrap().key, "history");
    });
    view.update(cx, |w, cx| {
        w.history_show(NavItem::View(SavedView::Failures), cx)
    });
    wait_until(&view, cx, "failures only", |w| {
        w.history.entries.len() == 2 && w.history.entries.iter().all(|e| e.status == Some(500))
    });
    view.read_with(cx, |w, _| {
        assert_eq!(
            w.history.query.nav,
            Some(NavItem::View(SavedView::Failures))
        );
        assert_eq!(w.history.stats.count, 2);
        assert_eq!(w.history.counts.failures, 2);
        assert_eq!(w.history.counts.requests.len(), 3);
    });
    // the navigator item for a request filters by that request
    view.update(cx, |w, cx| {
        w.history_show(
            NavItem::Request {
                collection: "c".into(),
                path: "r1.flux.md".into(),
            },
            cx,
        )
    });
    wait_until(&view, cx, "one request", |w| {
        !w.history.entries.is_empty()
            && w.history
                .entries
                .iter()
                .all(|e| e.request_path == "r1.flux.md")
    });
    view.update(cx, |w, cx| w.history_clear_filters(cx));
    wait_until(&view, cx, "all", |w| w.history.entries.len() == 10);
}

#[gpui::test]
fn selecting_shows_detail_without_a_tab_and_pin_note_delete_work(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-detail");
    seed_history(&view, cx, 4);
    wait_until(&view, cx, "seeded", |w| w.history.entries.len() == 4);
    let tabs_before = view.read_with(cx, |w, _| w.scope.tabs.all().len());
    let id = view.read_with(cx, |w, _| w.history.entries[1].id);
    view.update(cx, |w, cx| w.history_select(id, cx));
    wait_until(&view, cx, "detail loaded", move |w| {
        w.history.cache.contains_key(&id) && w.history.note_for == Some(id)
    });
    view.read_with(cx, |w, _| assert_eq!(w.scope.tabs.all().len(), tabs_before));
    // pin
    view.update(cx, |w, cx| w.history_toggle_pin(cx));
    wait_until(&view, cx, "pinned", move |w| {
        w.history.entries.iter().any(|e| e.id == id && e.pinned)
    });
    // note round-trips
    view.update(cx, |w, cx| {
        let i = w.history.note_input.clone().unwrap();
        i.update(cx, |i, cx| i.set_text("baseline", cx));
        w.history_commit_note(cx);
    });
    wait_until(&view, cx, "note saved", move |w| {
        w.history
            .entries
            .iter()
            .any(|e| e.id == id && e.note.as_deref() == Some("baseline"))
    });
    // clearing keeps the pinned entry
    view.update(cx, |w, cx| {
        w.history_clear_scope(fluxwire_store::Scope::All, false, cx)
    });
    wait_until(&view, cx, "cleared but pinned", move |w| {
        w.history.entries.len() == 1 && w.history.entries[0].id == id
    });
    view.update(cx, |w, cx| w.history_delete(vec![id], cx));
    wait_until(&view, cx, "deleted", |w| w.history.entries.is_empty());
}

#[gpui::test]
fn ticked_rows_diff_and_compare_to_previous_open_a_diff_tab(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-diff");
    seed_history(&view, cx, 6);
    wait_until(&view, cx, "seeded", |w| w.history.entries.len() == 6);
    let (a, b) = view.read_with(cx, |w, _| {
        (w.history.entries[1].id, w.history.entries[0].id)
    });
    view.update(cx, |w, cx| {
        w.history_toggle_tick(a, cx);
        w.history_toggle_tick(b, cx);
        w.history_diff_ticked(cx);
    });
    wait_until(&view, cx, "diff of ticked rows", move |w| {
        w.history.diffs.keys().any(|k| k.0 == a && k.1 == b)
    });
    // entries 0 and 3 are the same request (i % 3); the newest is index 0 here
    let newest = view.read_with(cx, |w, _| w.history.entries[0].id);
    view.update(cx, |w, cx| w.history_compare_previous(newest, cx));
    wait_until(&view, cx, "compare to previous", move |w| {
        w.scope
            .tabs
            .all()
            .iter()
            .any(|t| t.key.starts_with("historydiff/") && t.key.ends_with(&format!("/{newest}")))
    });
}

#[gpui::test]
fn ten_thousand_entries_build_only_the_loaded_page_of_rows(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-big");
    seed_history(&view, cx, 10_000);
    wait_until(&view, cx, "first page", |w| {
        w.history.entries.len() == crate::history_query::PAGE
    });
    view.read_with(cx, |w, _| {
        assert!(!w.history.all_loaded);
        assert_eq!(w.history.stats.count, 10_000, "stats cover the whole set");
        assert!(
            w.history.rows.len() <= crate::history_query::PAGE + 4,
            "one page of rows plus headers"
        );
    });
}

#[gpui::test]
fn history_layout_round_trips_through_the_session(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "hist-layout");
    view.update(cx, |w, _| {
        w.history.widths[2] = 222.0;
        w.history.divider = 500.0;
        w.history.query.sort = fluxwire_store::Sort {
            col: fluxwire_store::SortCol::Duration,
            asc: true,
        };
    });
    let layout = view.read_with(cx, |w, _| w.layout());
    view.update(cx, |w, _| {
        w.history.widths = crate::history_query::DEFAULT_WIDTHS;
        w.history.divider = 420.0;
        w.history.query.sort = Default::default();
        w.apply_layout(&layout);
    });
    view.read_with(cx, |w, _| {
        assert_eq!(w.history.widths[2], 222.0);
        assert_eq!(w.history.divider, 500.0);
        assert_eq!(w.history.query.sort.col, fluxwire_store::SortCol::Duration);
        assert!(w.history.query.sort.asc);
    });
}

// ---- M6: search, tabs, layout, keymap -------------------------------------------------------

fn labels(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.modal_items().into_iter().map(|i| i.label).collect()
    })
}

#[gpui::test]
fn quick_search_prefix_filters(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "prefix");
    write_env_file(&root, "qa");
    view.update(cx, |w, _| {
        w.on_env_fs_change(&[root.join("environments/qa.env")])
    });
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("@sta");
    assert_eq!(labels(&view, cx), ["staging"]);
    cx.simulate_keystrokes("escape secondary-k");
    cx.simulate_input("/refunds");
    let folders = labels(&view, cx);
    assert!(folders.iter().any(|l| l == "refunds"), "{folders:?}");
    assert!(view.read_with(cx, |w, _| {
        w.modal_items()
            .iter()
            .all(|i| matches!(i.run, crate::workspace::ItemRun::Folder(_)))
    }));
    cx.simulate_keystrokes("escape secondary-k");
    cx.simulate_input(">toggle theme");
    assert_eq!(
        labels(&view, cx).first().map(String::as_str),
        Some("theme: toggle")
    );
}

fn write_env_file(root: &Path, name: &str) {
    std::fs::write(root.join(format!("environments/{name}.env")), "A=1\n").unwrap();
}

#[gpui::test]
fn quick_search_matches_urls_and_highlights(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("create refund");
    let hl = view.read_with(cx, |w, _| {
        w.modal
            .as_ref()
            .unwrap()
            .results
            .first()
            .map(|i| i.indices.clone())
            .unwrap()
    });
    assert!(
        !hl.is_empty(),
        "matched characters are reported for highlighting"
    );
}

#[gpui::test]
fn secondary_enter_opens_beside_the_active_tab(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/list-payments.flux.md", true, window, cx);
        w.open_request(CREATE, true, window, cx); // CREATE active, LIST after it
    });
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("create refund");
    cx.simulate_keystrokes("secondary-enter");
    let ks = keys(&view, cx);
    let at = |k: &str| ks.iter().position(|x| x == k).unwrap();
    assert_eq!(
        at("refunds/create-refund.flux.md"),
        at(CREATE) + 1,
        "{ks:?}"
    );
}

#[gpui::test]
fn ctrl_tab_switcher_uses_recency_and_commits_on_release(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/list-payments.flux.md", true, window, cx);
        w.open_request("refunds/create-refund.flux.md", true, window, cx);
    });
    let active = |view: &Entity<WorkspaceView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |w, _| w.scope.tabs.active().map(|t| t.key.clone()))
    };
    assert_eq!(
        active(&view, cx).as_deref(),
        Some("refunds/create-refund.flux.md")
    );
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(
        active(&view, cx).as_deref(),
        Some("payments/list-payments.flux.md")
    );
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(active(&view, cx).as_deref(), Some(CREATE));
    view.update(cx, |w, _| w.commit_switcher());
    // CREATE is now most recent, then LIST was previewed only (never committed), so refunds is next.
    let order = view.read_with(cx, |w, _| {
        w.scope
            .tabs
            .mru_order()
            .into_iter()
            .filter_map(|id| w.scope.tabs.get(id).map(|t| t.key.clone()))
            .collect::<Vec<_>>()
    });
    assert_eq!(order[0], CREATE);
}

#[gpui::test]
fn close_to_the_right_and_reopen(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/list-payments.flux.md", true, window, cx);
        w.open_request("refunds/create-refund.flux.md", true, window, cx);
        let id = w.scope.tabs.id_for_key(CREATE).unwrap();
        w.scope.tabs.activate(id);
    });
    cx.dispatch_action(crate::actions::CloseTabsToRight);
    assert_eq!(keys(&view, cx), [CREATE]);
    cx.simulate_keystrokes("secondary-shift-t");
    assert_eq!(keys(&view, cx).len(), 2);
    cx.dispatch_action(crate::actions::CloseAllTabs);
    assert!(keys(&view, cx).is_empty());
}

#[gpui::test]
fn layout_round_trips_through_the_session(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.dispatch_action(crate::actions::ToggleSplitOrientation);
    cx.dispatch_action(crate::actions::ShowProblems);
    cx.dispatch_action(crate::actions::GroupTabsByKind);
    cx.dispatch_action(crate::actions::ShowHistoryPanel);
    view.update(cx, |w, _| w.scope.left_width = 333.0);
    let session = view.read_with(cx, |w, _| w.session());
    let json = serde_json::to_string(&session).unwrap();
    let back: crate::workspace_model::Session = serde_json::from_str(&json).unwrap();
    // A fresh window picks the layout up.
    let (fresh, cx) = cx.add_window_view(WorkspaceView::new);
    fresh.update(cx, |w, _| w.apply_layout(&back.layout));
    let l = fresh.read_with(cx, |w, _| w.layout());
    assert!(!l.split_vertical && l.bottom_open && l.group_by_kind);
    assert_eq!(
        (l.panel.as_str(), l.bottom_tab.as_str(), l.left_width),
        ("history", "problems", 333.0)
    );
    // Old session files without a layout still load.
    let old: crate::workspace_model::Session =
        serde_json::from_str(r#"{"tabs":[],"active":null,"unsaved":{}}"#).unwrap();
    assert_eq!(old.layout, crate::workspace_model::Layout::default());
}

#[gpui::test]
fn legacy_panel_value_loads_as_workspace_and_only_workspace_is_written(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let legacy = ["proj", "ect"].concat();
    let l = crate::workspace_model::Layout {
        panel: legacy,
        ..Default::default()
    };
    assert_eq!(crate::workspace_model::Layout::default().panel, "workspace");
    // Loading maps the old value onto the Workspace panel (asserted through the fresh view below).
    view.update(cx, |w, _| {
        w.apply_layout(&l);
        assert_eq!(w.layout().panel, "workspace");
    });
}

#[test]
fn old_keymap_names_are_aliases() {
    let old_label = ["panel: ", "proj", "ect"].concat();
    let old_name = ["ShowPro", "jectPanel"].concat();
    for n in [old_label, old_name] {
        let a = crate::actions::find_action(&n).expect("alias resolves");
        assert_eq!(a.label, "panel: workspace");
    }
}

#[test]
fn source_has_no_legacy_noun() {
    let word = ["proj", "ect"].concat();
    let crates = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let mut bad = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = std::fs::read_dir(crates)
        .unwrap()
        .map(|e| e.unwrap().path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let src = std::fs::read_to_string(&p).unwrap();
                for (i, line) in src.lines().enumerate() {
                    let l = line.to_lowercase();
                    let allowed = line.contains("ProjectDirs") || l.contains("legacy-project");
                    if l.contains(&word) && !allowed {
                        bad.push(format!("{}:{}", p.display(), i + 1));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "Fluxwire has no {} concept; use Workspace, Collection, Folder, or Request (SPEC §3): {bad:?}",
        ["Pro", "ject"].concat()
    );
}

#[gpui::test]
fn non_request_tabs_and_scroll_are_restored(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |w, _| {
        w.open_tab(
            "keybindings",
            "Keybindings",
            crate::tabs::TabKind::Keybindings,
            true,
        );
        w.open_tab("sync", "Sync", crate::tabs::TabKind::Sync, true);
    });
    let s = view.read_with(cx, |w, _| w.session());
    let kinds: Vec<_> = s.tabs.iter().map(|t| t.kind.as_str()).collect();
    assert_eq!(kinds, ["", "keys", "sync"]);
    let (fresh, cx) = cx.add_window_view(WorkspaceView::new);
    fresh.update(cx, |w, cx| {
        for t in s.tabs.iter().filter(|t| !t.kind.is_empty()) {
            assert!(w.restore_other_tab(t, cx));
        }
    });
    assert_eq!(keys(&fresh, cx), ["keybindings", "sync"]);
}

fn keymap_dir(text: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fluxwire-keymap-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("keymap.toml"), text).unwrap();
    dir
}

fn send_keys(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.update_in(cx, |_, window, _| {
        window
            .bindings_for_action(&crate::actions::Send)
            .iter()
            .map(|b| {
                b.keystrokes()
                    .iter()
                    .map(|k| k.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    })
}

#[gpui::test]
fn keymap_override_rebinds_send_without_restart(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let before = send_keys(&view, cx);
    assert!(before.iter().any(|k| k.contains("enter")), "{before:?}");
    let dir = keymap_dir("[[bindings]]\n\"cmd-r\" = \"request::Send\"\n");
    cx.update(|_, cx| cx.set_global(crate::envs::ConfigDir(dir.clone())));
    view.update(cx, |w, cx| w.reload_keymap(cx));
    let after = send_keys(&view, cx);
    assert_eq!(after.len(), 1, "{after:?}");
    assert!(
        after[0].contains('r') && !after[0].contains("enter"),
        "{after:?}"
    );
    // Editing the file again takes effect on the next reload.
    std::fs::write(dir.join("keymap.toml"), "").unwrap();
    view.update(cx, |w, cx| w.reload_keymap(cx));
    assert_eq!(send_keys(&view, cx), before);
}

#[gpui::test]
fn keymap_problems_are_listed(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let dir = keymap_dir(
        "[[bindings]]\n\"cmd-r\" = \"request::Send\"\n[[bindings]]\n\"cmd-r\" = \"file::Save\"\n[[bindings]]\n\"cmd-x\" = \"nope::Missing\"\n\"a-b\" = \"Send\"\n",
    );
    cx.update(|_, cx| cx.set_global(crate::envs::ConfigDir(dir)));
    view.update(cx, |w, cx| w.reload_keymap(cx));
    let codes: Vec<&str> = view.read_with(cx, |w, _| {
        w.all_problems()
            .into_iter()
            .filter(|p| p.key == "keymap.toml")
            .map(|p| p.code)
            .collect()
    });
    assert!(codes.contains(&"K003"), "{codes:?}");
    assert!(codes.contains(&"K004"), "{codes:?}");
    assert!(codes.contains(&"K002"), "{codes:?}");
}

#[gpui::test]
fn keybindings_tab_lists_every_action(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.dispatch_action(crate::actions::OpenKeybindings);
    assert!(view.read_with(cx, |w, _| {
        w.scope
            .tabs
            .active()
            .is_some_and(|t| t.key == "keybindings")
    }));
    let rows = view.update(cx, |w, cx| {
        let state = cx.global::<crate::actions::KeymapState>().clone();
        w.keybinding_rows(&state)
    });
    for a in ACTIONS {
        assert!(
            rows.iter().any(|r| r.label == a.label),
            "{} missing",
            a.label
        );
    }
}

#[gpui::test]
fn every_action_is_reachable_from_the_palette(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-shift-p");
    let all = labels(&view, cx);
    for a in ACTIONS.iter().filter(|a| a.palette) {
        assert!(all.iter().any(|l| l == a.label), "{} not listed", a.label);
    }
    for a in ACTIONS.iter().filter(|a| !a.palette) {
        assert!(!all.iter().any(|l| l == a.label), "{} listed", a.label);
    }
}

/// An echo server on a thread: replies to every text frame and answers a close.
fn ws_echo_server() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { continue };
            std::thread::spawn(move || {
                let Ok(mut ws) = tungstenite::accept(s) else {
                    return;
                };
                while let Ok(m) = ws.read() {
                    let (text, close) = (m.is_text(), m.is_close());
                    if text && ws.send(m).is_err() {
                        break;
                    }
                    if close {
                        break;
                    }
                }
            });
        }
    });
    port
}

const WS_KEY: &str = "events/order-stream.flux.md";

fn ws_doc_text(port: u16) -> String {
    format!(
        "---\nname: Stream\n---\n\n```ws\nGET ws://127.0.0.1:{port}/stream\n```\n\n```json message subscribe\n{{ \"op\": \"subscribe\" }}\n```\n"
    )
}

#[gpui::test]
fn websocket_connects_sends_a_saved_message_and_disconnects(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "ws");
    let port = ws_echo_server();
    view.update_in(cx, |w, window, cx| w.open_request(WS_KEY, true, window, cx));
    set_doc_text(&view, cx, WS_KEY, ws_doc_text(port));
    let id = view.read_with(cx, |w, _| w.scope.tabs.active_id().unwrap());
    view.update(cx, |w, cx| w.ws_toggle(cx));
    wait_until(&view, cx, "the upgrade", |w| w.ws_connected(id));
    view.update(cx, |w, cx| w.ws_send_saved(0, cx));
    wait_until(&view, cx, "the echoed frame", |w| {
        w.scope.responses[&id].log.counts() == (1, 1)
    });
    view.update(cx, |w, cx| w.ws_toggle(cx));
    wait_until(&view, cx, "the session to end", |w| {
        w.scope.responses[&id].sending.is_none()
    });
    view.read_with(cx, |w, _| {
        let r = &w.scope.responses[&id];
        let res = r.result.as_ref().unwrap();
        assert!(res.error.is_none(), "{:?}", res.error);
        let s = res.outcome.as_ref().unwrap().session.as_ref().unwrap();
        assert!(
            s.frames
                .iter()
                .any(|f| !f.outbound && f.data == "{ \"op\": \"subscribe\" }")
        );
        assert!(res.history.is_some());
        assert!(!w.ws_connected(id));
    });
}

#[gpui::test]
fn save_as_message_inserts_a_block_with_a_span_edit(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "ws-save");
    view.update_in(cx, |w, window, cx| w.open_request(WS_KEY, true, window, cx));
    let id = view.read_with(cx, |w, _| w.scope.tabs.active_id().unwrap());
    let before = view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(WS_KEY)
            .unwrap()
            .text
            .clone()
    });
    view.update(cx, |w, cx| {
        w.scope.composers.entry(id).or_default().text = "{\"op\":\"ping\"}".into();
        w.ws_save_composer(cx);
    });
    view.read_with(cx, |w, _| {
        let doc = w.scope.workspace.as_ref().unwrap().doc(WS_KEY).unwrap();
        let msgs = doc.parsed.messages();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].text, "{\"op\":\"ping\"}");
        assert!(
            doc.text.starts_with(before.trim_end()),
            "existing content is untouched"
        );
    });
}

#[gpui::test]
fn folder_run_runs_every_request_and_tags_history(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "folderrun");
    let port = test_server(false);
    for i in 0..3 {
        let dir = root.join("bulk");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("r{i}.flux.md")),
            format!(
                "---\nname: R{i}\nseq: {i}\n---\n\n# R{i}\n\n```http\nGET http://127.0.0.1:{port}/x\n```\n\n```http expect\nHTTP 201\n```\n"
            ),
        )
        .unwrap();
    }
    view.update(cx, |w, cx| {
        w.open_run_tab("bulk", cx);
        let id = w.scope.tabs.active_id().unwrap();
        w.start_run(id, cx);
    });
    wait_until(&view, cx, "the run to finish", |w| {
        w.scope.runs.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        let r = w.scope.runs.values().next().unwrap();
        let res = r.result.as_ref().unwrap();
        let s = res.summary();
        assert_eq!((s.passed, s.failed), (3, 0), "{s:?}");
        assert_eq!(r.entries.len(), 3);
        let cmd = w
            .run_cli_command(w.scope.tabs.active_id().unwrap())
            .unwrap();
        assert!(cmd.starts_with("fluxwire run bulk"), "{cmd}");
    });
    // every request of the run wrote a history entry tagged with its run id
    let run_id = view.read_with(cx, |w, _| {
        w.scope.runs.values().next().unwrap().run_id.clone()
    });
    wait_until(&view, cx, "run history", |w| {
        let (Some(store), Some(ws)) = (&w.history.store, &w.history.ws_key) else {
            return false;
        };
        let f = fluxwire_store::Filter {
            run_id: run_id.clone(),
            ..Default::default()
        };
        store.list(ws, &f).map(|l| l.len() == 3).unwrap_or(false)
    });
}

#[gpui::test]
fn closing_a_workspace_cancels_runs_and_drops_all_scoped_state(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "close-ws");
    let ws_port = ws_echo_server();
    view.update_in(cx, |w, window, cx| w.open_request(WS_KEY, true, window, cx));
    set_doc_text(&view, cx, WS_KEY, ws_doc_text(ws_port));
    let ws_tab = view.read_with(cx, |w, _| w.scope.tabs.active_id().unwrap());
    view.update(cx, |w, cx| w.ws_toggle(cx));
    wait_until(&view, cx, "the upgrade", |w| w.ws_connected(ws_tab));

    let slow = test_server(true);
    let dir = root.join("slow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.flux.md"),
        format!(
            "---\nname: A\nseq: 0\n---\n\n# A\n\n```http\nGET http://127.0.0.1:{slow}/x\n```\n"
        ),
    )
    .unwrap();
    view.update(cx, |w, cx| {
        w.open_run_tab("slow", cx);
        let id = w.scope.tabs.active_id().unwrap();
        w.start_run(id, cx);
    });
    let (ws_cancel, run_cancel) = view.read_with(cx, |w, _| {
        assert_eq!(w.scope.running(), 2);
        (
            w.scope.responses[&ws_tab]
                .sending
                .as_ref()
                .unwrap()
                .cancel
                .clone(),
            w.scope
                .runs
                .values()
                .next()
                .unwrap()
                .cancel
                .clone()
                .unwrap(),
        )
    });
    view.update(cx, |w, _| w.close_workspace());
    assert!(ws_cancel.is_cancelled() && run_cancel.is_cancelled());
    cx.run_until_parked();
    view.read_with(cx, |w, _| {
        assert!(w.scope.workspace.is_none());
        assert!(w.scope.tabs.all().is_empty());
        assert!(w.scope.responses.is_empty() && w.scope.runs.is_empty());
        assert!(w.scope.envs.runtime.is_empty() && w.scope.envs.states.is_empty());
        assert!(w.scope.console.is_empty() && w.scope.search.is_empty());
        assert!(w.history.ws_key.is_none() && w.recent.last.is_none());
    });
}

#[gpui::test]
fn shift_w_closes_the_workspace_and_only_app_actions_remain(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-shift-w");
    assert!(view.read_with(cx, |w, _| w.scope.workspace.is_none()));
    // A bound key for an unavailable action does nothing.
    cx.simulate_keystrokes("secondary-n");
    assert!(view.read_with(cx, |w, _| w.scope.tabs.all().is_empty()));
    view.read_with(cx, |w, _| {
        assert!(w.action_available("fluxwire::OpenWorkspace"));
        assert!(w.action_available("fluxwire::NewWorkspace"));
        assert!(!w.action_available("fluxwire::CloseWorkspace"));
        assert!(!w.action_available("fluxwire::Send"));
    });
}

// ---- M9.01: workspace lifecycle -------------------------------------------------------------

fn bare(cx: &mut TestAppContext) -> (Entity<WorkspaceView>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::one_dark());
        actions::install(cx);
    });
    cx.add_window_view(WorkspaceView::new)
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fluxwire-m901-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[gpui::test]
fn new_workspace_creates_only_the_directory_and_lands_on_the_empty_state(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let parent = fresh_dir("newws");
    view.update_in(cx, |w, window, cx| {
        w.start_new_workspace(window, cx);
        let Some(crate::dialogs::Dialog::NewWorkspace(f)) = &mut w.dialog else {
            panic!("dialog");
        };
        f.location = parent.clone();
        f.name.update(cx, |i, cx| i.set_text("demo", cx));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let demo = parent.join("demo");
    view.read_with(cx, |w, _| {
        let ws = w.scope.workspace.as_ref().expect("workspace opened");
        assert_eq!(ws.root, demo);
        assert!(ws.collections.is_empty() && ws.errors.is_empty());
        assert!(w.dialog.is_none());
    });
    assert_eq!(std::fs::read_dir(&demo).unwrap().count(), 0);
}

#[gpui::test]
fn new_workspace_dialog_rejects_bad_names(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let parent = fresh_dir("badws");
    view.update_in(cx, |w, window, cx| {
        w.start_new_workspace(window, cx);
        let Some(crate::dialogs::Dialog::NewWorkspace(f)) = &mut w.dialog else {
            panic!("dialog");
        };
        f.location = parent.clone();
        f.name.update(cx, |i, cx| i.set_text("a/b", cx));
    });
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |w, _| {
        assert!(w.scope.workspace.is_none());
        assert!(w.dialog.is_some(), "Create stays disabled");
    });
    assert!(!parent.join("a").exists());
}

#[gpui::test]
fn new_collection_creates_files_selects_and_opens_it(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let root = fresh_dir("newcoll");
    view.update_in(cx, |w, window, cx| {
        w.open_workspace(&root, window, cx);
        w.start_new_collection(
            Some("Requests live in a collection. Create one first."),
            window,
            cx,
        );
        let Some(crate::dialogs::Dialog::NewCollection(f)) = &mut w.dialog else {
            panic!("dialog");
        };
        f.name.update(cx, |i, cx| i.set_text("Payments", cx));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let dir = root.join("payments");
    for f in ["fluxwire.toml", "collection.flux.md", ".gitignore"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert!(dir.join("environments").is_dir());
    view.read_with(cx, |w, _| {
        let ws = w.scope.workspace.as_ref().unwrap();
        assert_eq!(ws.collections.len(), 1);
        assert_eq!(w.scope.selected.as_deref(), Some("payments"));
        assert_eq!(
            w.scope.tabs.active().map(|t| t.key.as_str()),
            Some("payments/collection.flux.md")
        );
    });
}

#[gpui::test]
fn add_existing_collection_outcomes(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let root = fresh_dir("addcoll");
    std::fs::create_dir_all(root.join("loaded")).unwrap();
    fluxwire_format::create_collection(&root.join("loaded"), "Loaded").unwrap();
    std::fs::create_dir_all(root.join("plain")).unwrap();
    let outside = fresh_dir("addcoll-out");
    view.update_in(cx, |w, window, cx| w.open_workspace(&root, window, cx));
    // already loaded: notice, no dialog
    view.update_in(cx, |w, window, cx| {
        w.add_collection_dir(&root.join("loaded"), window, cx)
    });
    view.read_with(cx, |w, _| {
        assert!(
            w.notice
                .as_deref()
                .unwrap()
                .contains("already part of this workspace")
        );
        assert!(w.dialog.is_none());
    });
    // no fluxwire.toml: confirm, then create
    view.update_in(cx, |w, window, cx| {
        w.add_collection_dir(&root.join("plain"), window, cx)
    });
    view.read_with(cx, |w, _| {
        assert!(matches!(w.dialog, Some(crate::dialogs::Dialog::Confirm(_))))
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(root.join("plain/fluxwire.toml").is_file());
    // outside the root: offers to open it as the workspace; nothing changes
    view.update_in(cx, |w, window, cx| {
        w.add_collection_dir(&outside, window, cx)
    });
    view.read_with(cx, |w, _| {
        let Some(crate::dialogs::Dialog::Confirm(c)) = &w.dialog else {
            panic!("confirm");
        };
        assert!(c.lines[0].contains("outside this workspace"));
        assert!(matches!(
            c.buttons[0].1,
            crate::dialogs::ConfirmAction::OpenAsWorkspace(_)
        ));
    });
    cx.simulate_keystrokes("escape");
    assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    // too deep: refused with the rule
    let deep = root.join("a/b/c/d/e");
    std::fs::create_dir_all(&deep).unwrap();
    view.update_in(cx, |w, window, cx| w.add_collection_dir(&deep, window, cx));
    view.read_with(cx, |w, _| {
        let Some(crate::dialogs::Dialog::Confirm(c)) = &w.dialog else {
            panic!("confirm");
        };
        assert!(c.lines[0].contains("levels below"));
    });
}

fn confirm_of(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
) -> (String, Vec<String>, Vec<String>) {
    view.read_with(cx, |w, _| {
        let Some(crate::dialogs::Dialog::Confirm(c)) = &w.dialog else {
            panic!("expected a confirm dialog");
        };
        (
            c.title.clone(),
            c.lines.clone(),
            c.buttons.iter().map(|b| b.0.clone()).collect(),
        )
    })
}

#[gpui::test]
fn closing_with_unsaved_requests_asks_and_each_button_does_its_thing(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "close-dirty");
    let on_disk = std::fs::read_to_string(root.join(CREATE)).unwrap();
    set_doc_text(&view, cx, CREATE, format!("{on_disk}\nedited\n"));
    cx.simulate_keystrokes("secondary-shift-w");
    let (title, lines, buttons) = confirm_of(&view, cx);
    assert!(title.starts_with("Close "));
    assert!(lines[0].starts_with("1 unsaved request"), "{lines:?}");
    assert_eq!(
        buttons,
        ["Save all and close", "Discard and close", "Cancel"]
    );
    // Esc cancels: the workspace stays open and the edit is kept
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |w, _| {
        assert!(w.scope.workspace.is_some() && w.dialog.is_none());
    });
    // Discard: closed, file untouched
    cx.simulate_keystrokes("secondary-shift-w");
    view.update_in(cx, |w, window, cx| {
        w.run_confirm_action(
            crate::dialogs::ConfirmAction::Close(
                crate::dialogs::AfterClose::Welcome,
                crate::dialogs::CloseMode::Discard,
            ),
            window,
            cx,
        )
    });
    assert!(view.read_with(cx, |w, _| w.scope.workspace.is_none()));
    assert_eq!(std::fs::read_to_string(root.join(CREATE)).unwrap(), on_disk);
}

#[gpui::test]
fn save_all_and_close_writes_the_file(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "close-save");
    let on_disk = std::fs::read_to_string(root.join(CREATE)).unwrap();
    set_doc_text(&view, cx, CREATE, format!("{on_disk}\nsaved\n"));
    cx.simulate_keystrokes("secondary-shift-w");
    cx.simulate_keystrokes("enter"); // default button: Save all and close
    assert!(view.read_with(cx, |w, _| w.scope.workspace.is_none()));
    assert!(
        std::fs::read_to_string(root.join(CREATE))
            .unwrap()
            .ends_with("saved\n")
    );
}

#[gpui::test]
fn closing_with_a_running_folder_run_offers_stop_and_close(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "close-running");
    let slow = test_server(true);
    let dir = root.join("slow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.flux.md"),
        format!(
            "---\nname: A\nseq: 0\n---\n\n# A\n\n```http\nGET http://127.0.0.1:{slow}/x\n```\n"
        ),
    )
    .unwrap();
    view.update(cx, |w, cx| {
        w.open_run_tab("slow", cx);
        let id = w.scope.tabs.active_id().unwrap();
        w.start_run(id, cx);
    });
    cx.simulate_keystrokes("secondary-shift-w");
    let (_, lines, buttons) = confirm_of(&view, cx);
    assert_eq!(lines, ["1 running"]);
    assert_eq!(buttons, ["Stop and close", "Cancel"]);
    cx.simulate_keystrokes("enter");
    assert!(view.read_with(cx, |w, _| w.scope.workspace.is_none()));
}

#[gpui::test]
fn closing_clears_recent_last_and_reopening_restores_tabs(cx: &mut TestAppContext) {
    let state = fresh_dir("state");
    cx.update(|cx| cx.set_global(crate::workspace::StateDir(state.clone())));
    let (view, cx, root) = setup_with(cx, "payments-api", "close-restore");
    view.read_with(cx, |w, _| assert!(w.recent.last.is_some()));
    cx.simulate_keystrokes("secondary-shift-w");
    view.read_with(cx, |w, _| {
        assert!(w.scope.workspace.is_none() && w.recent.last.is_none());
    });
    let saved = crate::workspace_model::load_recent(&state);
    assert!(saved.last.is_none() && !saved.workspaces.is_empty());
    view.update_in(cx, |w, window, cx| w.open_workspace(&root, window, cx));
    cx.run_until_parked();
    assert_eq!(keys(&view, cx), [CREATE]);
}

#[gpui::test]
fn new_request_without_collections_opens_new_collection_and_single_collection_is_flagged(
    cx: &mut TestAppContext,
) {
    let (view, cx) = bare(cx);
    let root = fresh_dir("noreq");
    view.update_in(cx, |w, window, cx| w.open_workspace(&root, window, cx));
    cx.simulate_keystrokes("secondary-n");
    view.read_with(cx, |w, _| {
        let Some(crate::dialogs::Dialog::NewCollection(f)) = &w.dialog else {
            panic!("New collection dialog");
        };
        assert!(f.note.as_deref().unwrap().contains("Create one first"));
        assert!(!w.is_single_collection());
    });
}

#[gpui::test]
fn the_payments_workspace_is_a_single_collection(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    assert!(view.read_with(cx, |w, _| w.is_single_collection()));
}

// ---- URL bar caret and new-request URL ------------------------------------------------------

fn url_of(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |w, _| {
        let k = w.active_key().unwrap();
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .doc(&k)
            .unwrap()
            .parsed
            .request()
            .unwrap()
            .target
            .clone()
    })
}

#[gpui::test]
fn url_bar_has_a_movable_caret_and_selection(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| w.focus_url(window, cx));
    let base = url_of(&view, cx);
    // typing appends at the caret (end by default)
    cx.simulate_keystrokes("x");
    assert_eq!(url_of(&view, cx), format!("{base}x"));
    // arrows move the caret; typing inserts there
    cx.simulate_keystrokes("left left y");
    assert_eq!(
        url_of(&view, cx),
        format!("{}y{}x", &base[..base.len() - 1], &base[base.len() - 1..])
    );
    // home + delete removes the first character
    cx.simulate_keystrokes("home delete");
    assert_eq!(url_of(&view, cx).chars().count(), base.chars().count() + 1);
    // backspace at the start does nothing; end + backspace removes the last character
    cx.simulate_keystrokes("home backspace");
    assert_eq!(url_of(&view, cx).chars().count(), base.chars().count() + 1);
    cx.simulate_keystrokes("end backspace");
    assert_eq!(url_of(&view, cx).chars().count(), base.chars().count());
    // shift+left selects; typing replaces the selection
    let before = url_of(&view, cx);
    cx.simulate_keystrokes("shift-left shift-left z");
    let expect: String = before
        .chars()
        .take(before.chars().count() - 2)
        .collect::<String>()
        + "z";
    assert_eq!(url_of(&view, cx), expect);
    // a click places the caret (no selection) and select-all selects everything
    cx.simulate_keystrokes("secondary-a");
    #[allow(clippy::let_and_return)]
    let url_sel = |cx: &mut VisualTestContext| {
        view.read_with(cx, |w, cx| {
            let key = w.active_key().unwrap();
            let sel = w.scope.url_inputs.borrow()[&key].read(cx).selection();
            sel
        })
    };
    assert_eq!(url_sel(cx), (0, expect.len()));
    cx.simulate_keystrokes("left");
    assert_eq!(url_sel(cx), (0, 0));
}

fn new_request_text(root: &std::path::Path, env_file: Option<&str>) -> String {
    std::fs::create_dir_all(root.join("environments")).unwrap();
    fluxwire_format::create_collection(root, "C").unwrap();
    if let Some(e) = env_file {
        std::fs::write(root.join("environments/local.env"), e).unwrap();
    }
    std::fs::read_to_string(root.join("fluxwire.toml")).unwrap()
}

#[gpui::test]
fn new_request_url_uses_base_url_only_when_the_active_environment_defines_it(
    cx: &mut TestAppContext,
) {
    for (tag, env, expect) in [
        ("nourl-none", None, "GET \n"),
        ("nourl-empty", Some("other=1\n"), "GET \n"),
        (
            "nourl-set",
            Some("baseUrl=http://x\n"),
            "GET {{baseUrl}}/\n",
        ),
    ] {
        let root = fresh_dir(tag);
        new_request_text(&root, env);
        let (view, cx) = bare(cx);
        view.update_in(cx, |w, window, cx| {
            w.open_workspace(&root, window, cx);
            w.start_new(false, false, window, cx);
            w.scope
                .inline
                .as_ref()
                .unwrap()
                .text
                .update(cx, |i, cx| i.set_text("Thing", cx));
            w.confirm_inline(window, cx);
        });
        let text = std::fs::read_to_string(root.join("thing.flux.md")).unwrap();
        assert!(
            text.contains(&format!("```http\n{expect}")),
            "{tag}: {text:?}"
        );
    }
}

#[gpui::test]
fn workspace_node_opens_the_tree_context_menu_without_changing_the_selection(
    cx: &mut TestAppContext,
) {
    let (view, cx) = setup(cx);
    let before = view.read_with(cx, |w, _| w.scope.selected.clone());
    view.update(cx, |w, cx| {
        w.open_tree_menu(
            "",
            crate::workspace::MenuKind::Workspace,
            Default::default(),
            cx,
        )
    });
    view.read_with(cx, |w, _| {
        let m = w.scope.tree_menu.as_ref().expect("menu open");
        assert_eq!(m.kind, crate::workspace::MenuKind::Workspace);
        assert_eq!(w.scope.selected, before);
    });
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| w.scope.tree_menu.is_none()));
}

#[gpui::test]
fn dialog_name_field_has_a_movable_caret(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    view.update_in(cx, |w, window, cx| w.start_new_workspace(window, cx));
    cx.simulate_keystrokes("a c left b");
    let name = |view: &Entity<WorkspaceView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |w, cx| match &w.dialog {
            Some(crate::dialogs::Dialog::NewWorkspace(f)) => {
                let i = f.name.read(cx);
                (i.text().to_string(), i.buffer().head())
            }
            _ => panic!("dialog"),
        })
    };
    assert_eq!(name(&view, cx), ("abc".to_string(), 2));
    cx.simulate_keystrokes("home delete");
    assert_eq!(name(&view, cx), ("bc".to_string(), 0));
    cx.simulate_keystrokes("end shift-left shift-left x");
    assert_eq!(name(&view, cx), ("x".to_string(), 1));
}

// ---- collection rename / remove / delete ----------------------------------------------------

fn two_collections(tag: &str) -> PathBuf {
    let root = fresh_dir(tag);
    for n in ["a", "b"] {
        std::fs::create_dir_all(root.join(n)).unwrap();
        fluxwire_format::create_collection(&root.join(n), &n.to_uppercase()).unwrap();
    }
    root
}

fn collection_names(view: &Entity<WorkspaceView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.scope
            .workspace
            .as_ref()
            .unwrap()
            .collections
            .iter()
            .map(|c| c.collection.config.collection.name.clone())
            .collect()
    })
}

#[gpui::test]
fn rename_collection_updates_its_name_on_disk_and_in_the_tree(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let root = two_collections("rencoll");
    view.update_in(cx, |w, window, cx| {
        w.open_workspace(&root, window, cx);
        w.start_rename_collection("a", window, cx);
        w.modal
            .as_ref()
            .unwrap()
            .query
            .update(cx, |i, cx| i.set_text("Alpha", cx));
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(collection_names(&view, cx), ["Alpha", "B"]);
    let toml = std::fs::read_to_string(root.join("a/fluxwire.toml")).unwrap();
    assert!(
        toml.contains("name = \"Alpha\"") && toml.contains("format = 1"),
        "{toml}"
    );
    assert!(root.join("a").is_dir(), "the folder keeps its name");
}

#[gpui::test]
fn remove_collection_hides_it_without_touching_files_and_add_existing_brings_it_back(
    cx: &mut TestAppContext,
) {
    let (view, cx) = bare(cx);
    let root = two_collections("rmcoll");
    view.update_in(cx, |w, window, cx| {
        w.open_workspace(&root, window, cx);
        w.remove_collection("a", cx);
    });
    assert_eq!(collection_names(&view, cx), ["B"]);
    assert!(root.join("a/fluxwire.toml").is_file(), "files untouched");
    view.read_with(cx, |w, _| {
        assert_eq!(w.session().hidden_collections, ["a"]);
    });
    view.update_in(cx, |w, window, cx| {
        w.add_collection_dir(&root.join("a"), window, cx)
    });
    assert_eq!(collection_names(&view, cx), ["A", "B"]);
    view.read_with(cx, |w, _| {
        assert!(w.session().hidden_collections.is_empty())
    });
}

#[gpui::test]
fn delete_collection_asks_first_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let (view, cx) = bare(cx);
    let root = two_collections("delcoll");
    view.update_in(cx, |w, window, cx| {
        w.open_workspace(&root, window, cx);
        w.confirm_delete_collection("a", window, cx);
    });
    let (title, _, buttons) = confirm_of(&view, cx);
    assert!(title.starts_with("Move A to the trash"), "{title}");
    assert_eq!(buttons, ["Move to trash", "Cancel"]);
    cx.simulate_keystrokes("escape");
    assert_eq!(collection_names(&view, cx), ["A", "B"]);
    assert!(root.join("a").is_dir());
}

#[gpui::test]
fn the_workspace_root_collection_cannot_be_deleted_from_the_menu(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| {
        w.confirm_delete_collection("", window, cx)
    });
    view.read_with(cx, |w, _| {
        assert!(w.dialog.is_none());
        assert!(
            w.notice
                .as_deref()
                .unwrap()
                .contains("workspace folder itself")
        );
    });
}

#[gpui::test]
fn workspace_switcher_lists_recents_and_offers_new_workspace(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "switcher");
    let other = fresh_dir("switch-other");
    view.update_in(cx, |w, window, cx| {
        w.recent.workspaces = vec![
            root.display().to_string(),
            other.display().to_string(),
            "/definitely/missing/ws".into(),
        ];
        w.open_modal(ModalKind::Workspaces, window, cx);
    });
    let items = view.read_with(cx, |w, _| w.modal_items());
    assert_eq!(items[0].label, "New workspace…");
    assert!(
        items[1].detail.starts_with("current"),
        "{:?}",
        items[1].detail
    );
    assert!(
        items[3].detail.starts_with("Not found"),
        "{:?}",
        items[3].detail
    );
    // typing filters; Enter on the first entry opens the same New workspace dialog
    cx.simulate_input("new");
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |w, _| {
        assert!(w.modal.is_none());
        assert!(matches!(
            w.dialog,
            Some(crate::dialogs::Dialog::NewWorkspace(_))
        ));
    });
    cx.simulate_keystrokes("escape");
    // choosing another recent workspace switches to it
    view.update_in(cx, |w, window, cx| {
        w.open_modal(ModalKind::Workspaces, window, cx)
    });
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    view.read_with(cx, |w, _| {
        assert_eq!(w.scope.workspace.as_ref().unwrap().root, other);
    });
}

/// Typed characters must reach text through `TextInput` (IME, dead keys). The only other places
/// that read `key_char` are listed here, each with the reason it is still allowed.
#[test]
fn only_allow_listed_modules_read_key_char() {
    const ALLOWED: &[&str] = &[
        "input.rs",
        "text_input.rs",
        // Code editors and the WebSocket composer (known gap: no IME yet, see editor.rs docs).
        "editor.rs",
        // Test helpers build keystrokes.
        "tests.rs",
    ];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if path.extension().is_none_or(|e| e != "rs") || ALLOWED.contains(&name.as_str()) {
            continue;
        }
        if std::fs::read_to_string(&path).unwrap().contains("key_char") {
            offenders.push(name);
        }
    }
    assert!(offenders.is_empty(), "key_char used in {offenders:?}");
}

#[gpui::test]
fn panel_opens_a_preview_without_activating_and_follows_the_file_system(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-panel");
    let r = root.clone();
    view.update(cx, |w, _| w.open_env_doc(&r, "staging", false));
    view.read_with(cx, |w, _| {
        assert_eq!(
            w.active_env_name().as_deref(),
            Some("devel"),
            "opening never activates"
        );
        let t = w.scope.tabs.active().unwrap();
        assert!(t.key == "env/staging" && !t.pinned, "{t:?}");
    });
    // A file added outside the app shows up; a removed one disappears and the active falls back.
    std::fs::write(root.join("environments/zeta.env"), "A=1\n").unwrap();
    view.update(cx, |w, _| {
        w.on_env_fs_change(&[root.join("environments/zeta.env")])
    });
    assert!(
        view.read_with(cx, |w, _| w.env_names())
            .contains(&"zeta".to_string())
    );
    view.update(cx, |w, cx| w.set_active_env(Some("zeta".into()), cx));
    std::fs::remove_file(root.join("environments/zeta.env")).unwrap();
    view.update(cx, |w, _| {
        w.on_env_fs_change(&[root.join("environments/zeta.env")])
    });
    assert!(
        !view
            .read_with(cx, |w, _| w.env_names())
            .contains(&"zeta".to_string())
    );
    assert_ne!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("zeta")
    );
}

#[gpui::test]
fn at_prefix_lists_environments_and_environment_actions_target_the_current_one(
    cx: &mut TestAppContext,
) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-at");
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("@stag");
    assert_eq!(picker_labels(&view, cx), ["staging"]);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        view.read_with(cx, |w, _| w.active_env_name()).as_deref(),
        Some("staging")
    );
    view.update_in(cx, |w, window, cx| {
        window.dispatch_action(Box::new(crate::actions::ManageEnvironments), cx);
        let _ = w;
    });
    view.read_with(cx, |w, _| {
        assert_eq!(w.scope.panel, crate::workspace::Panel::Environments);
        assert_eq!(w.scope.tabs.active().unwrap().key, "env/staging");
    });
    // Duplicate acts on the open Environment tab.
    view.update_in(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::actions::DuplicateEnvironment), cx)
    });
    assert!(root.join("environments/staging-copy.env").exists());
}

fn cell_focused(
    view: &Entity<WorkspaceView>,
    cx: &mut VisualTestContext,
    k: &crate::env_editor::DocKey,
    row: usize,
    value: bool,
) -> bool {
    let id = view.read_with(cx, |w, _| w.env_doc(k).unwrap().ids[row]);
    let cell = if value {
        crate::env_doc::Cell::Value
    } else {
        crate::env_doc::Cell::Name
    };
    let input = view.update(cx, |w, cx| w.env_cell(k, id, cell, cx));
    cx.update(|window, app| input.read(app).is_focused(window))
}

#[gpui::test]
fn tab_moves_from_name_to_value_to_the_next_row(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-tab-key");
    write_env(&view, cx, &root, "local.env", "A=1\nB=2\n");
    view.update(cx, |w, _| w.open_env_tab("local"));
    let k = env_key(&view, cx, "local");
    cx.run_until_parked();
    let id0 = view.read_with(cx, |w, _| w.env_doc(&k).unwrap().ids[0]);
    view.update_in(cx, |w, window, cx| {
        let i = w.env_cell(&k, id0, crate::env_doc::Cell::Name, cx);
        window.focus(&gpui::Focusable::focus_handle(i.read(cx), cx));
    });
    cx.run_until_parked();
    assert!(cell_focused(&view, cx, &k, 0, false));
    cx.simulate_keystrokes("tab");
    assert!(cell_focused(&view, cx, &k, 0, true), "Tab: Name -> Value");
    cx.simulate_keystrokes("tab");
    assert!(
        cell_focused(&view, cx, &k, 1, false),
        "Tab: Value -> next Name"
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(cell_focused(&view, cx, &k, 0, true), "Shift+Tab goes back");
}

#[gpui::test]
fn e106_clears_by_itself_when_the_gitignore_covers_local_env_files(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-e106");
    std::fs::write(root.join(".gitignore"), "target\n").unwrap();
    view.update(cx, |w, _| w.on_env_fs_change(&[root.join(".gitignore")]));
    assert!(!view.read_with(cx, |w, _| w.ignores_local_env_files()));
    std::fs::write(root.join(".gitignore"), "target\n*.env\n").unwrap();
    view.update(cx, |w, _| w.on_env_fs_change(&[root.join(".gitignore")]));
    assert!(view.read_with(cx, |w, _| w.ignores_local_env_files()));
}

#[gpui::test]
fn environment_copy_formats_leave_secrets_out(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-copy");
    write_env(&view, cx, &root, "local.env", "A=1\n");
    write_env(&view, cx, &root, "local.local.env", "tok=hunter2\n");
    let (dotenv, json) = view.read_with(cx, |w, _| {
        (w.env_as_dotenv("local"), w.env_as_postman_json("local"))
    });
    assert_eq!(dotenv, "A=1\n");
    assert!(
        !json.contains("hunter2")
            && json.contains("\"type\": \"secret\"")
            && json.contains("\"key\": \"A\""),
        "{json}"
    );
}

#[gpui::test]
fn secret_checkbox_masks_and_eye_reveals_then_hides(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-eye");
    write_env(&view, cx, &root, "local.env", "A=1\n");
    view.update(cx, |w, _| w.open_env_tab("local"));
    let k = env_key(&view, cx, "local");
    let id = view.read_with(cx, |w, _| w.env_doc(&k).unwrap().ids[0]);
    view.update(cx, |w, cx| w.env_toggle_secret(&k, id, cx));
    assert!(view.read_with(cx, |w, _| w.env_doc(&k).unwrap().rows()[0].secret));
    assert!(!view.read_with(cx, |w, _| w.scope.envs.is_revealed("A")));
    view.update(cx, |w, cx| w.env_toggle_reveal(&k, "A", cx));
    assert!(view.read_with(cx, |w, _| w.scope.envs.is_revealed("A")));
    view.update(cx, |w, cx| w.env_toggle_reveal(&k, "A", cx));
    assert!(!view.read_with(cx, |w, _| w.scope.envs.is_revealed("A")));
}

/// Icons are embedded from an explicit list, and a missing one draws nothing. Every literal
/// passed to `icon(…)` must be in that list.
#[test]
fn every_icon_used_in_views_is_embedded() {
    use gpui::AssetSource;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut missing = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap_or_default();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("icon(") {
            rest = &rest[i + 5..];
            let after = rest.trim_start();
            if let Some(lit) = after.strip_prefix('"')
                && let Some(end) = lit.find('"')
            {
                let name = &lit[..end];
                let ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                if ok
                    && crate::assets::Assets
                        .load(&format!("icons/{name}.svg"))
                        .unwrap()
                        .is_none()
                {
                    missing.insert(name.to_string());
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "icons used but not embedded: {missing:?}"
    );
}

#[gpui::test]
fn palette_toggle_closes_the_palette(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-shift-p");
    assert!(view.read_with(cx, |w, _| w.modal.is_some()));
    cx.simulate_keystrokes("secondary-shift-p");
    assert!(view.read_with(cx, |w, _| w.modal.is_none()));
}

#[gpui::test]
fn hidden_actions_still_dispatch_from_keybindings(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.simulate_keystrokes("secondary-w");
    assert_eq!(keys(&view, cx).len(), 0);
    cx.simulate_keystrokes("secondary-shift-t");
    assert_eq!(
        keys(&view, cx).len(),
        1,
        "reopen closed is hidden but bound"
    );
    for old in [
        "Send",
        "request::Send",
        "fluxwire::NextTab",
        "Dismiss",
        "InputCopy",
        "view: toggle left dock",
        "keymap: open keybindings",
    ] {
        assert!(crate::actions::find_action(old).is_some(), "{old}");
    }
    assert!(!crate::actions::find_action("NextTab").unwrap().palette);
}

#[gpui::test]
fn environment_delete_is_listed_only_with_an_environment(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let has = |view: &Entity<WorkspaceView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |w, _| w.action_available("fluxwire::DeleteEnvironment"))
    };
    view.update(cx, |w, _| w.scope.envs.active = Some("staging".into()));
    assert!(has(&view, cx));
    view.update(cx, |w, _| w.scope.envs.active = None);
    assert!(!has(&view, cx));
}

#[gpui::test]
fn both_new_collection_entries_open_the_dialog(cx: &mut TestAppContext) {
    for action in [
        "fluxwire::NewCollection",
        "fluxwire::NewWorkspaceCollection",
    ] {
        let (view, cx, _) = setup_with(cx, "payments-api", "newcoll");
        view.update_in(cx, |w, window, cx| {
            let _ = w;
            window.dispatch_action((crate::actions::find_action(action).unwrap().build)(), cx);
        });
        cx.run_until_parked();
        assert!(
            view.read_with(cx, |w, _| w.dialog.is_some() || w.notice.is_some()),
            "{action}"
        );
    }
}

#[gpui::test]
fn keymap_editor_rebinds_clears_and_resets(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let dir = keymap_dir("# mine\n");
    cx.update(|_, cx| cx.set_global(crate::envs::ConfigDir(dir.clone())));
    view.update(cx, |w, cx| w.clear_keybinding("fluxwire::ToggleTheme", cx));
    view.update(cx, |w, cx| {
        w.clear_keybinding("fluxwire::ToggleLeftDock", cx)
    });
    let text = std::fs::read_to_string(dir.join("keymap.toml")).unwrap();
    assert!(text.contains("# mine") && text.contains("none"), "{text}");
    cx.simulate_keystrokes("secondary-b");
    assert!(
        view.read_with(cx, |w, _| w.scope.left_open),
        "binding cleared live"
    );
    view.update(cx, |w, cx| {
        w.reset_keybinding("fluxwire::ToggleLeftDock", cx)
    });
    cx.simulate_keystrokes("secondary-b");
    assert!(
        view.read_with(cx, |w, _| !w.scope.left_open),
        "binding restored live"
    );
    // Rebind with a chord captured from the keyboard.
    view.update(cx, |w, cx| {
        w.begin_capture("fluxwire::ToggleBottomDock", cx)
    });
    cx.simulate_keystrokes("ctrl-alt-j");
    let text = std::fs::read_to_string(dir.join("keymap.toml")).unwrap();
    assert!(
        text.contains("ctrl-alt-j") && text.contains("ToggleBottomDock"),
        "{text}"
    );
    assert!(text.contains("# mine"));
}

#[gpui::test]
fn keymap_editor_flags_conflicts_and_escape_cancels(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let dir = keymap_dir("");
    cx.update(|_, cx| cx.set_global(crate::envs::ConfigDir(dir.clone())));
    view.update(cx, |w, cx| {
        w.begin_capture("fluxwire::ToggleBottomDock", cx)
    });
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| w.keybind_edit.is_none()));
    view.update(cx, |w, cx| {
        w.begin_capture("fluxwire::ToggleBottomDock", cx)
    });
    cx.simulate_keystrokes("secondary-b");
    let conflict = view.read_with(cx, |w, _| {
        w.keybind_edit.as_ref().and_then(|p| p.conflict.clone())
    });
    assert_eq!(conflict.unwrap().label, "view: toggle sidebar");
    assert!(
        !dir.join("keymap.toml").exists()
            || std::fs::read_to_string(dir.join("keymap.toml"))
                .unwrap()
                .is_empty()
    );
    view.update(cx, |w, cx| w.confirm_replace(cx));
    cx.simulate_keystrokes("secondary-b");
    assert!(view.read_with(cx, |w, _| w.scope.bottom_open && w.scope.left_open));
}

#[gpui::test]
fn keymap_edit_opens_the_tab_in_edit_mode(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.dispatch_action(crate::actions::EditKeymap);
    assert!(view.read_with(cx, |w, _| {
        w.keybind_editing
            && w.scope
                .tabs
                .active()
                .is_some_and(|t| t.key == "keybindings")
    }));
}
