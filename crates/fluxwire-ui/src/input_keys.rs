//! Key bindings and GPUI actions for text input (M9.02, SPEC §17.11, PROMPTS A4).
//!
//! Every action is bound in the `TextInput` key context, so `keymap.toml` can override them and
//! they are only live while an input is focused. The platform is a parameter of `input_keymap`
//! so both tables are tested on any host. Enter, Escape and Tab are deliberately absent: the
//! owner of the field decides what they do.

use gpui::actions;

pub const CONTEXT: &str = "TextInput";

actions!(
    fluxwire,
    [
        InputLeft,
        InputRight,
        InputWordLeft,
        InputWordRight,
        InputLineStart,
        InputLineEnd,
        InputUp,
        InputDown,
        InputDocStart,
        InputDocEnd,
        InputSelectLeft,
        InputSelectRight,
        InputSelectWordLeft,
        InputSelectWordRight,
        InputSelectLineStart,
        InputSelectLineEnd,
        InputSelectUp,
        InputSelectDown,
        InputSelectDocStart,
        InputSelectDocEnd,
        InputSelectAll,
        InputCopy,
        InputCut,
        InputPaste,
        InputUndo,
        InputRedo,
        InputBackspace,
        InputDelete,
        InputDeleteWordBack,
        InputDeleteWordForward,
        InputDeleteToLineStart,
        InputContextMenu,
    ]
);

/// (action short name, [macOS keys], [Linux/Windows keys]).
type Row = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const TABLE: &[Row] = &[
    ("InputLeft", &["left"], &["left"]),
    ("InputRight", &["right"], &["right"]),
    ("InputWordLeft", &["alt-left"], &["ctrl-left"]),
    ("InputWordRight", &["alt-right"], &["ctrl-right"]),
    ("InputLineStart", &["cmd-left", "home", "ctrl-a"], &["home"]),
    ("InputLineEnd", &["cmd-right", "end", "ctrl-e"], &["end"]),
    ("InputUp", &["up"], &["up"]),
    ("InputDown", &["down"], &["down"]),
    ("InputDocStart", &["cmd-up"], &["ctrl-home"]),
    ("InputDocEnd", &["cmd-down"], &["ctrl-end"]),
    ("InputSelectLeft", &["shift-left"], &["shift-left"]),
    ("InputSelectRight", &["shift-right"], &["shift-right"]),
    (
        "InputSelectWordLeft",
        &["alt-shift-left"],
        &["ctrl-shift-left"],
    ),
    (
        "InputSelectWordRight",
        &["alt-shift-right"],
        &["ctrl-shift-right"],
    ),
    (
        "InputSelectLineStart",
        &["cmd-shift-left", "shift-home"],
        &["shift-home"],
    ),
    (
        "InputSelectLineEnd",
        &["cmd-shift-right", "shift-end"],
        &["shift-end"],
    ),
    ("InputSelectUp", &["shift-up"], &["shift-up"]),
    ("InputSelectDown", &["shift-down"], &["shift-down"]),
    (
        "InputSelectDocStart",
        &["cmd-shift-up"],
        &["ctrl-shift-home"],
    ),
    (
        "InputSelectDocEnd",
        &["cmd-shift-down"],
        &["ctrl-shift-end"],
    ),
    ("InputSelectAll", &["cmd-a"], &["ctrl-a"]),
    ("InputCopy", &["cmd-c"], &["ctrl-c", "ctrl-insert"]),
    ("InputCut", &["cmd-x"], &["ctrl-x", "shift-delete"]),
    ("InputPaste", &["cmd-v"], &["ctrl-v", "shift-insert"]),
    ("InputUndo", &["cmd-z"], &["ctrl-z"]),
    ("InputRedo", &["cmd-shift-z"], &["ctrl-y", "ctrl-shift-z"]),
    (
        "InputBackspace",
        &["backspace", "shift-backspace", "ctrl-h"],
        &["backspace", "shift-backspace"],
    ),
    ("InputDelete", &["delete", "ctrl-d"], &["delete"]),
    (
        "InputDeleteWordBack",
        &["alt-backspace"],
        &["ctrl-backspace"],
    ),
    ("InputDeleteWordForward", &["alt-delete"], &["ctrl-delete"]),
    ("InputDeleteToLineStart", &["cmd-backspace"], &[]),
    ("InputContextMenu", &[], &["shift-f10", "menu"]),
];

/// Every `(keystroke, action short name)` for the given platform.
pub fn input_keymap(macos: bool) -> Vec<(&'static str, &'static str)> {
    TABLE
        .iter()
        .flat_map(|(name, mac, other)| {
            let keys = if macos { *mac } else { *other };
            keys.iter().map(move |k| (*k, *name))
        })
        .collect()
}

/// Action short names, in table order (for the palette rows).
pub fn input_action_names() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|r| r.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_of(macos: bool, action: &str) -> Vec<&'static str> {
        input_keymap(macos)
            .into_iter()
            .filter(|(_, a)| *a == action)
            .map(|(k, _)| k)
            .collect()
    }

    #[test]
    fn a4_table_on_both_platforms() {
        // (action, macOS, other)
        let rows: &[(&str, &[&str], &[&str])] = &[
            ("InputLeft", &["left"], &["left"]),
            ("InputWordLeft", &["alt-left"], &["ctrl-left"]),
            ("InputWordRight", &["alt-right"], &["ctrl-right"]),
            ("InputLineStart", &["cmd-left", "home", "ctrl-a"], &["home"]),
            ("InputLineEnd", &["cmd-right", "end", "ctrl-e"], &["end"]),
            ("InputDocStart", &["cmd-up"], &["ctrl-home"]),
            ("InputDocEnd", &["cmd-down"], &["ctrl-end"]),
            ("InputSelectAll", &["cmd-a"], &["ctrl-a"]),
            ("InputCopy", &["cmd-c"], &["ctrl-c", "ctrl-insert"]),
            ("InputCut", &["cmd-x"], &["ctrl-x", "shift-delete"]),
            ("InputPaste", &["cmd-v"], &["ctrl-v", "shift-insert"]),
            ("InputUndo", &["cmd-z"], &["ctrl-z"]),
            ("InputRedo", &["cmd-shift-z"], &["ctrl-y", "ctrl-shift-z"]),
            (
                "InputDeleteWordBack",
                &["alt-backspace"],
                &["ctrl-backspace"],
            ),
            ("InputDeleteWordForward", &["alt-delete"], &["ctrl-delete"]),
            ("InputDeleteToLineStart", &["cmd-backspace"], &[]),
            ("InputContextMenu", &[], &["shift-f10", "menu"]),
        ];
        for (action, mac, other) in rows {
            assert_eq!(&keys_of(true, action), mac, "{action} on macOS");
            assert_eq!(&keys_of(false, action), other, "{action} elsewhere");
        }
        assert!(keys_of(true, "InputBackspace").contains(&"ctrl-h"));
        assert!(keys_of(true, "InputDelete").contains(&"ctrl-d"));
        assert!(!keys_of(false, "InputBackspace").contains(&"ctrl-h"));
    }

    #[test]
    fn shift_extends_every_movement() {
        for macos in [true, false] {
            let map = input_keymap(macos);
            for moving in [
                "Left",
                "Right",
                "WordLeft",
                "WordRight",
                "LineStart",
                "LineEnd",
                "Up",
                "Down",
                "DocStart",
                "DocEnd",
            ] {
                let sel = format!("InputSelect{moving}");
                assert!(
                    map.iter().any(|(_, a)| *a == sel),
                    "{sel} missing (macos={macos})"
                );
            }
        }
    }

    #[test]
    fn owner_keys_are_not_bound() {
        for macos in [true, false] {
            for (k, _) in input_keymap(macos) {
                assert!(
                    !matches!(k, "enter" | "escape" | "tab" | "shift-tab"),
                    "{k} belongs to the owner"
                );
            }
        }
    }

    #[test]
    fn no_keystroke_is_bound_twice_and_all_parse() {
        for macos in [true, false] {
            let map = input_keymap(macos);
            let mut seen = std::collections::HashSet::new();
            for (k, a) in &map {
                assert!(seen.insert(*k), "{k} bound twice (macos={macos}), at {a}");
                assert!(gpui::Keystroke::parse(k).is_ok(), "{k} does not parse");
            }
        }
    }
}
