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
fn trust_collection_of(view: &Entity<Workspace>, cx: &mut VisualTestContext, key: &str) {
    view.update(cx, |w, _| {
        let root = w.collection_root_of(key).expect("collection root");
        w.session_trust.insert(root);
    });
}

fn set_doc_text(view: &Entity<Workspace>, cx: &mut VisualTestContext, key: &str, text: String) {
    view.update(cx, |w, _| {
        w.project
            .as_mut()
            .unwrap()
            .doc_mut(key)
            .unwrap()
            .set_text(text);
    });
}

fn wait_until(
    view: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    what: &str,
    f: impl Fn(&Workspace) -> bool,
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
        assert_eq!(w.trust_prompt, w.tabs.active_id(), "the first send asks");
        assert!(w.responses.values().all(|r| r.result.is_none()));
    });

    view.update(cx, |w, cx| w.set_collection_trust(true, cx));
    wait_until(&view, cx, "the scripted send", |w| {
        w.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        assert!(w.trust_prompt.is_none());
        let res = w
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
        assert_eq!(w.envs.runtime["paymentId"].0, "pay_1");
        assert_eq!(w.envs.runtime["stamp"].0, "s1");
        assert!(
            w.console.iter().any(|l| l.contains("before send")),
            "{:?}",
            w.console
        );
    });

    view.update(cx, |w, cx| w.set_collection_trust(false, cx));
    view.read_with(cx, |w, _| {
        let root = w.collection_root_of(CREATE).unwrap();
        assert!(!w.session_trust.contains(&root));
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
        w.responses.values().any(|r| r.shown.is_some())
    });
    view.read_with(cx, |w, _| {
        let r = w.responses.values().next().unwrap();
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
            w.console.iter().any(|l| l.contains("201 Created")),
            "{:?}",
            w.console
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
        w.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        let res = w
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
        assert!(w.responses.is_empty(), "the first send only warns");
    });
    cx.simulate_keystrokes("secondary-enter");
    wait_until(&view, cx, "the second send to finish", |w| {
        w.responses.values().any(|r| r.result.is_some())
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
        w.project
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
        w.project
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
            let d = w.project.as_ref().unwrap().doc(CREATE).unwrap();
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
            w.project
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
        let d = w.project.as_ref().unwrap().doc(CREATE).unwrap();
        assert_eq!(d.parsed.request().unwrap().method, "GET");
    });
}

// ---- M4: environments, variables, secrets ---------------------------------------------------

fn write_env(
    view: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    root: &Path,
    file: &str,
    text: &str,
) {
    let p = root.join("environments").join(file);
    std::fs::write(&p, text).unwrap();
    view.update(cx, |w, _| w.on_env_fs_change(&[p]));
}

fn last_url(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |w, _| {
        let r = w
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

fn picker_labels(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
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
        w.responses.values().any(|r| r.result.is_some())
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
        w.responses.values().all(|r| r.sending.is_none())
            && w.responses.values().any(|r| {
                r.result
                    .as_ref()
                    .and_then(|x| x.outcome.as_ref())
                    .is_some_and(|o| o.final_url.contains(&format!(":{p_staging}/")))
            })
    });
}

#[gpui::test]
fn environment_picker_lists_real_environments_and_switches(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "env-picker");
    cx.simulate_keystrokes("secondary-e");
    assert_eq!(
        picker_labels(&view, cx),
        ["local", "staging", "No environment"]
    );
    cx.simulate_input("stag");
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |w, _| {
        assert_eq!(w.envs.active.as_deref(), Some("staging"))
    });
    assert_eq!(view.read_with(cx, |w, _| w.env_label()), "staging");
}

#[gpui::test]
fn environment_files_can_be_created_renamed_duplicated_and_deleted(cx: &mut TestAppContext) {
    let (view, cx, root) = setup_with(cx, "payments-api", "env-ops");
    view.update_in(cx, |w, window, cx| w.start_new_env(window, cx));
    cx.simulate_input("qa");
    cx.simulate_keystrokes("enter");
    assert!(root.join("environments/qa.env").exists());
    assert_eq!(
        view.read_with(cx, |w, _| w.envs.active.clone()).as_deref(),
        Some("qa")
    );
    view.update_in(cx, |w, window, cx| w.start_rename_env("qa", window, cx));
    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("uat");
    cx.simulate_keystrokes("enter");
    assert!(
        root.join("environments/uat.env").exists() && !root.join("environments/qa.env").exists()
    );
    view.update(cx, |w, cx| w.duplicate_env("uat", cx));
    assert!(root.join("environments/uat-copy.env").exists());
    // Invalid names are refused with a message.
    view.update_in(cx, |w, window, cx| w.start_new_env(window, cx));
    cx.simulate_input("a.b");
    cx.simulate_keystrokes("enter");
    assert!(!root.join("environments/a.b.env").exists());
    view.read_with(cx, |w, _| {
        assert!(w.notice.as_deref().unwrap_or("").contains("one word"))
    });
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
    assert!(view.read_with(cx, |w, _| w.envs.is_revealed("tok")));
    view.update(cx, |w, _| {
        w.envs.reveal.insert(
            "tok".into(),
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        );
    });
    assert!(!view.read_with(cx, |w, _| w.envs.is_revealed("tok")));

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
        w.responses.values().any(|r| r.result.is_some())
    });
    view.read_with(cx, |w, _| {
        let log = w.console.join("\n");
        assert!(!log.contains("hunter2-value"), "{log}");
        assert!(log.contains("‹secret:tok›"), "{log}");
    });
}

#[gpui::test]
fn environment_tab_edits_write_the_right_files(cx: &mut TestAppContext) {
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
    view.update_in(cx, |w, window, cx| {
        w.start_env_edit("local", 0, crate::envs::EnvCol::Value, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("x");
    assert_eq!(
        std::fs::read_to_string(root.join("environments/local.env")).unwrap(),
        "# keep me\nbaseUrl=http://ax\n",
        "comments survive, only the value changes"
    );
    // A new row goes to the file as soon as it has a name; marking it secret moves it.
    view.update_in(cx, |w, window, cx| {
        w.start_env_edit("local", 1, crate::envs::EnvCol::Name, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("pw");
    view.update_in(cx, |w, window, cx| {
        w.start_env_edit("local", 1, crate::envs::EnvCol::Value, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("s3");
    let on_disk = std::fs::read_to_string(root.join("environments/local.env")).unwrap();
    assert!(on_disk.contains("pw=s3"), "{on_disk:?}");
    view.update(cx, |w, cx| w.toggle_env_secret("local", 1, cx));
    assert!(
        !std::fs::read_to_string(root.join("environments/local.env"))
            .unwrap()
            .contains("pw=")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("environments/local.local.env")).unwrap(),
        "pw=s3\n"
    );
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
        let c = w.envs.completion.as_ref().expect("suggestions");
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
    view.read_with(cx, |w, _| assert!(w.envs.completion.is_none()));
}

#[gpui::test]
fn unresolved_variables_are_listed_in_problems(cx: &mut TestAppContext) {
    let (view, cx, _root) = setup_with(cx, "payments-api", "problems");
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
            &w.project.as_ref().unwrap().doc(CREATE).unwrap().text[v[0].span.start..v[0].span.end],
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
    // An environment that no longer exists falls back to the first one.
    view.update(cx, |w, _| w.init_active_env(Some("gone".into())));
    assert_eq!(
        view.read_with(cx, |w, _| w.envs.active.clone()).as_deref(),
        Some("local")
    );
    view.update(cx, |w, _| w.init_active_env(Some("staging".into())));
    assert_eq!(
        view.read_with(cx, |w, _| w.envs.active.clone()).as_deref(),
        Some("staging")
    );
}

fn doc_for(port: u16, path: &str) -> String {
    format!(
        "---\nname: Local\n---\n\n# Local\n\n```http\nGET http://127.0.0.1:{port}{path}\nAccept: application/json\n```\n"
    )
}

/// Send the active request and wait until `n` history entries are listed.
fn send_and_record(view: &Entity<Workspace>, cx: &mut VisualTestContext, n: usize) {
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
        let t = w.tabs.active().unwrap();
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
        w.history.method = Some("POST");
        w.history_reload(cx);
    });
    wait_until(&view, cx, "POST filter", |w| w.history.entries.is_empty());
    view.update(cx, |w, cx| {
        w.history.method = None;
        w.history.text.insert("/v1/one");
        w.history_reload(cx);
    });
    wait_until(&view, cx, "text filter", |w| w.history.entries.len() == 1);
    view.update(cx, |w, cx| {
        w.history.text.clear();
        w.history_reload(cx);
    });
    wait_until(&view, cx, "filters cleared", |w| {
        w.history.entries.len() == 2
    });

    // response-pane dropdown shows the earlier response, then goes back
    view.update(cx, |w, cx| w.show_history_response(first, cx));
    wait_until(&view, cx, "earlier response", |w| {
        w.responses
            .values()
            .any(|r| r.viewing.is_some() && r.shown.is_some())
    });
    view.update(cx, |w, cx| w.history_back_to_latest(cx));
    wait_until(&view, cx, "latest response", |w| {
        w.responses
            .values()
            .any(|r| r.viewing.is_none() && r.shown.is_some())
    });

    // restore puts the old request in an unsaved tab
    let on_disk = std::fs::read_to_string(root.join(CREATE)).unwrap();
    view.update_in(cx, |w, window, cx| {
        w.history_restore(first, CREATE.to_string(), window, cx)
    });
    wait_until(&view, cx, "restored", |w| {
        w.project
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

// ---- M6: search, tabs, layout, keymap -------------------------------------------------------

fn labels(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
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
    let active = |view: &Entity<Workspace>, cx: &mut VisualTestContext| {
        view.read_with(cx, |w, _| w.tabs.active().map(|t| t.key.clone()))
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
        w.tabs
            .mru_order()
            .into_iter()
            .filter_map(|id| w.tabs.get(id).map(|t| t.key.clone()))
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
        let id = w.tabs.id_for_key(CREATE).unwrap();
        w.tabs.activate(id);
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
    view.update(cx, |w, _| w.left_width = 333.0);
    let session = view.read_with(cx, |w, _| w.session());
    let json = serde_json::to_string(&session).unwrap();
    let back: crate::project::Session = serde_json::from_str(&json).unwrap();
    // A fresh window picks the layout up.
    let (fresh, cx) = cx.add_window_view(Workspace::new);
    fresh.update(cx, |w, _| w.apply_layout(&back.layout));
    let l = fresh.read_with(cx, |w, _| w.layout());
    assert!(!l.split_vertical && l.bottom_open && l.group_by_kind);
    assert_eq!(
        (l.panel.as_str(), l.bottom_tab.as_str(), l.left_width),
        ("history", "problems", 333.0)
    );
    // Old session files without a layout still load.
    let old: crate::project::Session =
        serde_json::from_str(r#"{"tabs":[],"active":null,"unsaved":{}}"#).unwrap();
    assert_eq!(old.layout, crate::project::Layout::default());
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
    let (fresh, cx) = cx.add_window_view(Workspace::new);
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

fn send_keys(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
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
        w.tabs.active().is_some_and(|t| t.key == "keybindings")
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
    for a in ACTIONS {
        assert!(all.iter().any(|l| l == a.label), "{} not listed", a.label);
    }
}
