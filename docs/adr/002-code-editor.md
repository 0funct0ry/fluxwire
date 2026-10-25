# ADR-002: Code editor component

Status: accepted (M1)

## Context
SPEC §17.3 and §27 ask M1 to evaluate `gpui-component`'s editor (tree-sitter highlighting, virtualization for large bodies) and fall back to a minimal editor of our own.

## Decision
Use a small editor surface of our own (`crates/fluxwire-ui/src/editor.rs`): a line-number gutter plus tree-sitter highlighting.

## Why
- `gpui-component 0.8.0` requires `gpui-pre =0.3.9`, a different package from the `gpui 0.2.2` we pin (ADR-001). Using it would mean moving the whole app to that GPUI fork, with different types and APIs, for one widget.
- M1 only needs read-mostly surfaces (bodies, scripts, responses). Highlighting is a thin layer over `tree-sitter` + `tree-sitter-json`: leaf nodes are classified (property, string, number, boolean, punctuation) and colored from theme tokens.

## Consequences
- The current surface is static: no caret, selection, or virtualization yet. Editable buffers arrive with M2 (span editor), and the response viewer is virtualized in M3 (50 MB bodies). Both build on `editor.rs`.
- Highlighting for Markdown with injected fence languages, YAML, TS/JS, XML and HTTP is added in M2 (ADR-003).
- Fonts: SPEC §17.7 names IBM Plex Mono for buffers. It is not bundled yet, so JetBrains Mono (OFL, from microterm) stands in via `assets::BUFFER_FONT`. Swap the constant and the `FONTS` list once the Plex Mono files are added.
