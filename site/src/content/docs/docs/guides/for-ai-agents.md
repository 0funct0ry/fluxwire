---
title: For AI agents
description: How coding agents should read, write, and validate .flux.md files.
---

`.flux.md` files are plain Markdown, so an agent needs no special tooling to edit them. This page lists the rules that
keep edits correct.

## Reading

- Only `*.flux.md` files are Requests. Other `.md` files are docs.
- The front matter is a restricted YAML subset: no anchors, aliases, or tags.
- A fenced block's info string is `<language> [role]`. The first `http` block is the request; `http expect` is the
  expected response; `ts pre`, `ts post`, and `ts test` are scripts.
- `<lang> example` blocks are documentation only. Use them for samples in prose.

## Writing (strict profile)

1. Start with front matter containing `name`.
2. Write the request as an HTTP message in an `http` block.
3. Give every executable fence a role. Do not invent roles; unknown roles raise `F201`, and these are reserved:
   `query`, `variables`, `arguments`, `schema`, `step`, `response`, `fixture`, `mock`, `stream`, `metadata`,
   `resource`, `prompt`.
4. Change only what the task needs. Preserve unknown keys, `x-` content, prose, and block order.
5. Do not add `requires` for features you do not use. See [Extending the format](../../reference/extending-the-format/).

## Validating

```sh
fluxwire fmt --check path/       # canonical formatting
fluxwire validate path/          # diagnostics with codes and fixes
fluxwire run path/ --env local   # execute and report
```

Validate front matter in an editor with the published
[JSON Schema](../../../schema/front-matter.schema.json). The schema checks keys and types, while `fluxwire validate`
also checks blocks.

## Fluxwire as an MCP server

`fluxwire mcp serve` exposes Collections, Environments (secrets excluded), and run tools to agents over stdio or a unix
socket. Restrict access with `--root` and choose tool classes with `--allow` (default `read,execute`).
