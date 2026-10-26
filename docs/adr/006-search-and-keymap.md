# ADR 006: Search index, keymap overrides, and "open beside"

Status: accepted (M6)

## Search

`fluxwire-search` wraps `nucleo-matcher` in a small in-memory `Index` of `Item { id, kind, label, detail, extra, tags }`. One haystack string per item (`label detail extra`) is scored with a smart-case fuzzy pattern; the top results get a second pass that computes label match indices for highlighting. Recency is a rank map (most recent first) added as a bounded score boost, and an empty query lists recent items first.

The UI never rebuilds the index from scratch. `Project` caches a row per file (URL, front matter `tags`, headings) when it parses the file for diagnostics, and after a file-system change re-parses only the changed files and files not seen before (`rescan_incremental`). `Index::sync` then diffs the wanted items against the stored ones and touches only the delta. Measured with `cargo bench -p fluxwire-search`: 0.5–3 ms per query at 5,000 requests (budget 16 ms).

Quick search prefixes: `>` actions, `@` environments, `#` tags, `/` folders, `!` history. Palette = quick search with a forced `>`.

## Keymap

`keymap.toml` uses `[[bindings]]` tables with an optional `context` and `"keystroke" = "Action"` entries. Action names are matched by their last `::` segment, so the SPEC's `request::Send`, `fluxwire::Send` and `Send` all resolve (the namespace is documentation only). A user binding for an action **replaces** that action's default bindings (no stacking); `"key" = "none"` unbinds a default key. Invalid entries are skipped and reported as Problems: K001 parse error, K002 same key and context on two actions, K003 unknown action, K004 invalid keystroke, K005 invalid context. The file is watched and applied with `cx.clear_key_bindings()` + rebinding, so no restart is needed.

## Open beside

The center area has a single active-tab view; real split panes are out of scope for M6. Cmd/Ctrl+Enter in quick search opens the result as a permanent tab placed immediately to the right of the active tab (and opens a recent workspace in a new window). Real side-by-side panes are deferred.
