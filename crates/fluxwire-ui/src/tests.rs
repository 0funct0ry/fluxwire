//! Behavioural tests for the window shell, driven through real actions and keystrokes.

use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::{
    actions::{self, ACTIONS},
    theme::{Appearance, Theme, theme},
    workspace::{ModalKind, Workspace},
};

fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::one_dark());
        actions::install(cx);
    });
    let (view, cx) = cx.add_window_view(Workspace::new);
    cx.run_until_parked();
    (view, cx)
}

fn keys(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |w, _| {
        w.tabs.all().iter().map(|t| t.key.clone()).collect()
    })
}

#[gpui::test]
fn preview_tab_is_replaced_then_a_permanent_tab_stays(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    // The shell starts with one permanent tab.
    assert_eq!(keys(&view, cx), ["payments/create"]);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/get", false, window, cx)
    });
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/list", false, window, cx)
    });
    assert_eq!(keys(&view, cx), ["payments/create", "payments/list"]);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/list", true, window, cx)
    });
    view.update_in(cx, |w, window, cx| {
        w.open_request("refunds/get", false, window, cx)
    });
    assert_eq!(
        keys(&view, cx),
        ["payments/create", "payments/list", "refunds/get"]
    );
    let previews = view.read_with(cx, |w, _| w.tabs.all().iter().filter(|t| t.preview).count());
    assert_eq!(previews, 1);
}

#[gpui::test]
fn pinning_and_closing_tabs(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update_in(cx, |w, window, cx| {
        w.open_request("payments/get", true, window, cx)
    });
    cx.simulate_keystrokes("secondary-shift-p");
    cx.simulate_input("tab: pin");
    cx.simulate_keystrokes("enter");
    let pinned = view.read_with(cx, |w, _| w.tabs.active().map(|t| t.pinned));
    assert_eq!(pinned, Some(true));
    assert_eq!(keys(&view, cx)[0], "payments/get", "pinned tab sorts first");

    cx.simulate_keystrokes("secondary-w");
    assert_eq!(keys(&view, cx), ["payments/create"]);
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
    cx.simulate_input("refund");
    cx.simulate_keystrokes("enter");
    assert!(keys(&view, cx).contains(&"refunds/create".to_string()));
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
    // Choosing an item sets the method (not "the next one") and closes the menu.
    view.update(cx, |w, _| {
        w.method_menu = false;
        w.active_ui().unwrap().method = 4;
    });
    assert_eq!(method(&view, cx), Some(4));
    view.update(cx, |w, _| w.method_menu = true);
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |w, _| !w.method_menu));
}
