# ADR-014: Palette visibility

## Status
Accepted (M9.04).

## Context
The palette listed every `ACTIONS` entry whose scope was open, including `input:` editing actions, `modal: dismiss`, and sub-view switchers that are only useful as key bindings.

## Decision
`ActionInfo` gains `palette: bool` (`a!` = visible, `h!` = hidden). Hidden actions stay in `ACTIONS`, so keybindings, `keymap.toml`, the Keybindings tab and `find_action` are unchanged; only `refresh_search_index` skips them. `environment: delete…` is also skipped while no environment is selected. Renamed labels keep resolving through `ALIASES`; the alias `workspace: new collection` now points to the new `workspace: new collection` entry, which opens the same dialog as `collection: new…`.

A user binding for an action now shadows another action's default on the same key and context (no K002 warning); user-vs-user conflicts still warn.

## Consequences
Adding an action means choosing `a!` or `h!`; a test pins the visible set to the curated list.
