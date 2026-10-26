# ADR-003: Highlighting for `.flux.md`

Status: accepted (M2)

## Decision

- **Markdown structure**: `tree-sitter-md` block grammar (headings, fence delimiters, info strings, list/quote markers).
- **Injected languages**: JSON, YAML (also front matter and `yaml config`), JavaScript, TypeScript, XML use their tree-sitter grammars and the highlight queries shipped in the grammar crates. TypeScript runs the JavaScript query plus the TypeScript query.
- **HTTP-family blocks** (`http`, `ws`, `http expect`, `http form`, `http multipart`, `http defaults`) are painted from `fluxwire-format`'s typed spans, not from a tree-sitter grammar.
- Variables (`{{name}}`) are painted as an overlay in heads, bodies and messages.

## Why not the community HTTP grammar

`tree-sitter-http` 0.1.0 compiles against our tree-sitter 0.26 but ships no highlight queries and models a bare HTTP message. It does not know four-space `?`/`&` continuation lines, `# ` disabled entries or the expect block's `HTTP 2xx` status forms. The format parser already produces exact spans for all of these, so painting from it is more accurate and cannot drift from what the parser accepts. No fork under `grammars/` was needed; revisit if a future milestone needs HTTP highlighting outside `.flux.md`.

## Consequences

- The grammar crates are dependencies of `fluxwire-ui` only.
- Highlighting is recomputed on each edit (whole document); fine for request files. Large response bodies (M3) will need background, incremental highlighting.
- Inline Markdown (emphasis, links) is not highlighted yet; the inline grammar can be added without API changes.
