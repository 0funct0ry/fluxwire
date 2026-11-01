# ADR-013: History tab, navigator, and pinned-exempt retention

Status: accepted (M9.03)

## Context

History drew every entry in the left dock. At dock width the URL, time and status could not all show, filtering was two chip rows plus three Clear buttons whose scope was unclear, and reading an entry meant opening a tab per entry. Entries could not be kept past retention, annotated, summarized, or exported.

## Decision

- **One History tab per Workspace** (key `history`, reopen-or-focus). It is a master/detail viewer: a sorted, filtered, virtualized table and an inline detail pane. Entry and diff tabs keep their `history/{id}` and `historydiff/{a}/{b}` keys and still use `TabKind::History`; the tab key, not a new kind, selects the view, so saved sessions keep restoring.
- **The dock panel is a navigator.** It lists the timeline, built-in saved views and the requests that have history, with counts. Every item sets the tab's `Query` (`history_query.rs`, pure and unit-tested) and opens the tab; the active item is highlighted. The Clear buttons are gone: Clear lives in the tab's Clear menu and the Requests context menu, always behind the confirm dialog.
- **Saved views are queries**: failures (status class 0, 4, 5), slow (over 1 s), sent unsaved, pinned, folder runs. They are built in and not user-editable in v1.0.
- **Pinned entries are exempt** from retention (age and count) and from Clear. The confirm dialog offers "Clear including pinned" as a separate button. Deleting entries explicitly removes pinned ones too.
- **Store additions** (additive migration 3: `pinned`, `note`): extended `Filter` and `Sort`, `stats` (count, success rate, nearest-rank p50/p95), `counts`, `set_pinned`, `set_note` (redacted), `delete`, `previous_of`, and `export`.
- **Export** is JSON or CSV over the whole filtered set. The folder-run report serializers take a `FolderRunResult`, not history rows, so history has its own serializers in `fluxwire-store::export`. Entries are redacted at write time, so exports add no secrets.
- **Paging**: newest-first order pages by `before_id`; any other sort pages by offset.
- Column widths, sort and the detail divider persist per workspace in the session layout.

## Consequences

The store stays protocol-neutral and GPUI-free. The table is a fixed-row-height `uniform_list`, so only visible rows are built. Day headers scroll with the list (they are not sticky yet).
