# ADR-022: API documentation

Status: accepted (M13)

## Decisions
- **`fluxwire-docs` is the single owner of docs logic and has no GPUI.** It holds the Markdown block model (moved from `fluxwire-ui/src/markdown_view.rs`, which now re-exports it), `chips` (the compact label and owning sub-tab of an executable block), `generated` (sections built from the parsed request), `links` (relative link resolution), `headings`, `example` (the `http example` content built from a response), `html` (static HTML writer) and `export` (the directory walk behind `fluxwire docs export`). It depends on `fluxwire-format` and `fluxwire-model` only.
- **Preview is a second renderer over one model.** The app draws the block model with GPUI (`docs_preview.rs`); the exporter draws the same model as HTML. Both mark executable blocks as chips and `example` blocks as samples, so the two cannot drift on what is prose.
- **Edit, Preview and Split.** The Docs sub-tab keeps `docs_source` (editor on or off) and adds `docs_split`. Split shows the M9.06 `CodeEditor` and the Preview side by side; the editor's read-only block ranges are unchanged, so Split cannot change a request.
- **Generated sections are never written.** Method and URL, parameters, headers, example body and the expect block in words are computed from `FluxDoc` on every render. Response schemas from a Spec link are deferred.
- **Save response as example is one span insert.** `response: save as example` builds the content with `example::example_content` (status line, headers, body; only an allowlist of headers is kept, in canonical casing, secrets of the run are replaced by `‹secret:name›` through `RunResult::redact`) and inserts it with `fluxwire_format::insert_block(BlockKind::Example, "http example", …)`. Nothing else in the file changes.
- **Links.** `links::resolve` returns `Doc`, `Anchor`, `External` or `Other`. Only `http`, `https` and `mailto` open in the browser; `javascript:` and other schemes are never opened or emitted as `href`. A relative `.flux.md` link opens that file in the app; a link that leaves the Workspace is refused.
- **Static export.** `fluxwire docs export <PATH> --out <DIR>` writes `index.html` and copies local images to `assets/`. The page is one file with inline CSS, a navigation list, no `<script>`, and One Light / One Dark colours through `prefers-color-scheme`. Relative `.flux.md` links become in-page anchors. Text is escaped; unsafe link and image schemes are dropped. JSON bodies get token colouring; other languages are plain.
- **Search.** Docs headings were already indexed by `fluxwire-search` from the workspace rows (M6); M13 adds nothing to the index.
- **Exit codes.** A missing path or no docs files exits 64; a write failure exits 74.

Deferred: response schemas from a Spec link in the generated sections, tree-sitter highlighting of non-JSON code in the exported HTML, plain `README.md` files as Docs tabs inside the app (they are exported), a Save-as-example button in the response header (the palette action exists), and scroll-sync between the editor and the Preview in Split.
