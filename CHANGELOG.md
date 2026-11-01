# Changelog

## Unreleased

### Added
- **History viewer (M9.03).** History is a workspace-level tab (Cmd/Ctrl+Shift+H): a sortable, filterable, virtualized table with resizable columns, folder-run groups, an inline detail pane (request, response, error and phase for failed sends), a stats strip (count, success rate, p50, p95), pin, notes, multi-select, diff, *Compare to previous*, restore, and JSON/CSV export. The left-dock History panel is now a navigator (timeline, saved views, requests with counts) whose items open the tab with a filter. Pinned entries survive retention and Clear. New actions: `history: open`, `history: focus search`, `history: pin`, `history: export`. Store migration 3 adds `pinned` and `note`.
- **Environment management (M9.02).** Environments are workspace-level (`<workspace>/environments/`, one active environment); collection-level environments are deferred. Environment switcher overlay (title-bar pill, Cmd/Ctrl+E) with *New environment…*, *Create environment “q”*, and *Manage environments…*; Environments panel; New environment dialog (Start from, Set as active); validated rename; delete confirm listing files.
- **Environment editor.** Key–value table with live Name/Value inputs, Secret lock, Source, drag reorder, filter, bulk edit, validation (E101–E106), document undo, dirty state, Cmd/Ctrl+S, close confirm, and an external-change banner. Sends use unsaved values.
- **One active environment per workspace,** remembered in the session (including an explicit "No environment").
- **Shared text input.** Mouse selection, per-OS keyboard table, clipboard, undo, IME and dead keys, context menu, masked and read-only variants; the code editors gain selection, clipboard, and undo.
- New actions: `environment: edit active`, `environment: manage`, `environment: new…`, `environment: rename…`, `environment: duplicate`, `environment: delete…`; the `@` quick-search prefix lists the workspace's environments.

### Fixed (post-M9.02 review)
- Tab moves Name → Value → next row in the environment editor; E106 uses `git check-ignore` (so `*.env` counts) and clears on its own; the Environments panel lists environments only (no repeated heading or variables), has a per-row menu (right-click and "…") with Copy as .env / Postman JSON; the dirty dot uses the `modified` colour in the tab and panel; Secret is a checkbox with an eye button that reveals for 10 s.

### Changed
- Environment files are written only on save, byte-preserving (comments, order, quoting kept).
- `OpenEnvironment` is now `EditActiveEnvironment` (old names still work in `keymap.toml`).
