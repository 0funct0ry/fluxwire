# ADR-010: Workspace lifecycle

Status: accepted (M9.01)

## Context

The welcome view offered collection-level actions before any Workspace existed, "New collection" silently opened the chosen folder as the Workspace, and "workspace: close" left environments, runs and sessions behind and was undone on the next launch.

## Decisions

- **No Workspace manifest.** A Workspace is a directory opened in one window. Opening writes nothing to it.
- **Collections live inside the Workspace root,** at a path `find_collections` discovers (at most 4 levels deep, not hidden, not `node_modules` or `target`). The app never references a Collection outside the root. `containment.rs` holds the rules; every create/add path uses it.
- **Collections do not nest.** `find_collections` stops at the first `fluxwire.toml`. A Workspace whose root is a Collection is valid but cannot hold more Collections.
- **Opening fails only for a missing or unreadable directory.** A Collection that fails to load is an error row; the rest of the Workspace opens.
- **`OpenWorkspace` owns all workspace-scoped state** (tabs, responses, runs, environments, search, layout, watcher). Opening builds a fresh value and closing calls `shutdown` (also run on `Drop`) before replacing it, so no field-by-field reset exists to forget. `WorkspaceView.scope` is always present and holds `workspace: Option<Workspace>`; app-level state (theme, keymap, recent, modals, notices) stays on the view.
- **Closing clears `recent.last`** so the next launch shows the welcome view; the session (tabs, environment, layout) is still saved so reopening restores it. Quitting keeps `recent.last`.
- **Actions have a level** (`App`, `Workspace`, `Collection`). Unavailable actions are hidden from the palette and ignored when bound.
- **Aliases.** Old action names and labels (`OpenFolder`, "workspace: open folder", "workspace: new collection", "workspace: import", "workspace: close (show welcome)", "panel: project") keep working in `keymap.toml` for v1.0. A persisted panel value of "project" loads as "workspace".
- **New workspace** closes the current Workspace (with the usual confirm) when Create is pressed, not when the dialog opens, so cancelling loses nothing.

## Consequences

Scope bugs of this kind (an action offered at a level that does not own it) are prevented structurally by levels and `OpenWorkspace`. SPEC §17.9 states the rule.
