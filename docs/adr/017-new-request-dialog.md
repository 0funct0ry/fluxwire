# ADR-017: New request dialog

## Status
Accepted (M9.07).

## Context
Creating a request offered only an inline name row (empty or `{{baseUrl}}/` URL, GET); cURL could only be pasted into an existing request. Dialogs had no type selector, location picker or multi-line field.

## Decision
- `New request…` is a modal (`new_request_dialog.rs`) over a pure model (`new_request.rs`: `NewRequestForm`, `validate`, `suggest_name`, `plan`). The inline flow stays as `Quick request` (`workspace: quick request`, palette-visible, no binding). `workspace: new request` (Secondary-N) opens the dialog. `New WebSocket request` leaves the menus; WebSocket is a type in the dialog.
- Types come from the registered Protocols (`Runner::protocols()`, `Protocol::label`/`url_schemes`) plus From cURL when a Protocol answers `preview_paste`. The dialog's model names its type `NewKind` (not `RequestKind`, which exists in `fluxwire-model`); an unknown Protocol appears as `NewKind::Other(id)` with name, location and URL.
- cURL: one parser. `parse_curl_ex` returns warnings; value-taking flags that do not affect a request (`-o`, `--proxy`, `-m`, …) are ignored with a warning, extra URLs are ignored with a warning, `-d @file` is refused. Create writes the template, then applies the same `edits_from_paste` edits as pasting, so the two cannot diverge. Credential headers are kept and flagged.
- File writing: `fsops::create_request_from`/`render_request` (`RequestSpec`, `InitialBody`). JSON adds `Content-Type` and a `{}` body inside the request block (the form paste produces); Form adds the header and an `http form` block; Text adds the header only. Output is canonical.
- An empty name is valid while the URL (or cURL command) yields a suggestion (`GET users`); otherwise Create is disabled with the reason shown.
- An empty URL uses `default_url_for` (the active environment's `baseUrl`/`host`), as Quick request does.
- Containment: the location must be inside the Workspace root. `containment::check_rules` is for Collections and would flag every folder below one as nested, so it is not used here.
- After Create the request is selected, opened in a permanent, pinned tab with the URL focused, and the status bar names the path. Quick request already opened a permanent (not preview) tab and keeps that.
- Secondary-Enter creates from multi-line fields; it is bound to `request: send`, so the Send handler submits the dialog while it is open.

## Consequences
A later protocol adds a type by implementing `label`/`url_schemes`; the dialog only needs extra fields if the protocol needs them.

## Deferred
Arrow-key stepping in the segmented control (`segmented::step` exists), `{{variable}}` completion popup and highlighting in the dialog URL field, an injectable `Runner` for GPUI tests (kinds are tested on the pure model), shell highlighting for the cURL field, GraphQL/gRPC types.
