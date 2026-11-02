# ADR-018: Semantic UI tree

Status: accepted (M9.5)

## Context
`ui.state` (M9.3) is data about the app, not about its controls. Agents need to find a control by
label or role and activate it. GPUI 0.2.2 has no accessibility tree, and `Window::dispatch_event`
returns a crate-private type, so pointer events cannot be injected from outside the crate.

## Decision
- `dev_id.rs` tags an element with `dev_click(id, role, label, enabled, handler)` (method form
  `DevClick::dev_click`, last call of a chain) or `dev_tag`/`dev_tag_state` for non-clickable ones.
  With `dev-mcp` off these are identity functions (`dev_click` is `on_click`), so render paths
  gain no code or allocation. Call sites pass `&str` ids, never `format!`.
- With `dev-mcp` on, the tag wraps the element in `Tagged`, which records bounds, the ancestor
  content mask (clip), focus and a parent stack during prepaint into a `Registry` global
  (the window's frame). Paint order is the z-order. `WorkspaceView::render` calls `begin_frame`.
- `dev_tree.rs` is the pure model (nesting, `#n` suffixes for repeated keys by occurrence, filter,
  find, click point). Ids are semantic keys (`tab` ids are the tab's Request key), never pointers or
  loop indices.
- `ui.click_element` cannot synthesize a mouse event, so it runs the click handler the element
  registered, with `ClickEvent::default()`, via `update_window` outside the root view's lease
  (listeners update the view themselves). The point is still validated (enabled, visible,
  not under a modal). Elements without a registered handler report so; use `ui.invoke_action`.
- Requests run between draws, so the tree is the latest completed frame; after an action, wait
  with `ui.wait_for_idle` before reading it.

## Consequences
Coverage grows by tagging; M9.5 tags the title bar, status bar, toolbar and request tabs.
The rest of the controls listed in the demos are deferred to follow-up tagging.
