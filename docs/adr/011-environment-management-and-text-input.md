# ADR-011: Environment management and text input

Status: accepted (M9.02). The per-collection environment decisions below are superseded by ADR-012 (workspace-level environments).

## Context

Environments were workspace-wide (one active name), written to disk on every keystroke, and reachable only through a picker that could not create or edit them. Text editing existed three times (`ui::TextEdit`, `editor::EditorState`, raw `String` cells) with no clipboard, IME, or context menu, and mouse selection was re-implemented per field.

## Decisions

### Environments

- **Environments belong to a Collection.** Files stay in `<collection>/environments/<name>.env` and `<name>.local.env`. The UI shows the collection whenever the Workspace has more than one.
- **The active environment is per Collection.** `EnvStore.active_by` maps the absolute collection root to a name; the session stores `active_envs` keyed by the workspace-relative root (`.` for the workspace root). A session written by M9.01 (one name in `active_env`) is read as that name for every collection that has an environment of that name, others fall back to their first environment, and the session is then written in the new shape only.
- **"Current collection"** is the active tab's collection, else the selected tree item's, else the only collection. With two or more and none current, the switcher asks first.
- **An environment is a document** (`env_doc.rs`): rows live in memory, edits mark it dirty, undo steps are whole row sets (a run of typing in one cell is one step), and only an explicit save (Cmd/Ctrl+S, or Save on close) writes both files. No write per keystroke.
- **Sending never requires saving.** A dirty document of the active environment is passed as `ScopeInputs.env_overlay` (and `ScriptOptions.env_overlay` to the runner), so scopes, hovers, and sends use unsaved values.
- **Persistence is byte-preserving** (`fluxwire-format`): `rewrite_dotenv` keeps comments, blank lines, `export` prefixes, quoting, and untouched lines; new keys append; reordering moves a line with the comment and blank lines directly above it; `rename_key_in_place` keeps line numbers. Invalid rows (E101) are skipped and counted; of two same-named rows in one file the later wins (E102). Property tests: load→save is byte-exact, save is idempotent.
- **Tab key** is `env/<collection-rel>/<name>`; an old `env/<name>` maps to the first collection that has it.
- **Name rules** live in one validator (`validate_environment_name`), shared by New, Rename, and the "Create environment “q”" row; matching is case-insensitive on macOS and Windows.

### Text input

- **One input** (`text_input.rs`) on a pure `TextBuffer` (`input.rs`). Printable text arrives only through `EntityInputHandler` (IME, dead keys, emoji picker); key handlers never insert `key_char` (guard test).
- **Offsets are byte offsets on grapheme boundaries,** not char indices as the prompt says: Rust strings are bytes, and the shaped-line and platform-UTF-16 conversions need byte positions anyway. Every move and edit snaps to a grapheme boundary (`unicode-segmentation`), so the observable behaviour matches.
- **Platform keymaps** are one table (`input_keys.rs`) built by `input_keymap(macos)` so both platforms are tested on any host. Actions run in the `TextInput` key context; owners add their own context (`Modal`, `RequestEditor`, `EnvTable`) because a GPUI context predicate matches only the deepest context.
- **Masked fields** (secrets) draw bullets and refuse Copy and Cut; typing and paste work.
- **Multi-line editors** (`editor.rs`) run on the same `TextBuffer` (selection, clipboard, coalescing undo, mouse selection) but still take characters from key events. They have no IME or context menu yet; the guard test allow-lists `editor.rs` and says why.

## Consequences

Per-collection state removes the "picker list changes with the tab" bug. The document model gives dirty state, undo, and close confirms for free. Known gaps are listed in `docs/demos/M9.02.md`.
