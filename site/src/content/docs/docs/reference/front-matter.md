---
title: Front matter reference
description: Every front matter key of a .flux.md file, generated from the published JSON Schema.
---

Front matter of a `.flux.md` file. Keys starting with `x-` are extensions and are preserved, never interpreted.

The schema is published at [`/schema/front-matter.schema.json`](../../../schema/front-matter.schema.json).
Point your editor or agent at it with a YAML language-server modeline or a `yaml.schemas` setting. This page is
generated from `docs/schema/front-matter.schema.json`.

## Keys

| Key | Type | Description |
|---|---|---|
| `auth` | `AuthSchema` or `null` | `inherit` (default), `none`, or a map with `type` and fields. |
| `capture` | map of `string` | Variables set from the response: `$.json.path`, `header <Name>`, `cookie <name>`, `status`. |
| `deprecated` | `boolean` | Mirrors OpenAPI `deprecated`. |
| `kind` | `string` | `request` (default for `*.flux.md`), `folder`, or `collection`. Other kinds need `requires`. |
| `name` | `string` | Display name (required for requests). |
| `operation` | `string` | OpenAPI operationId or `METHOD /path`. |
| `path` | map of `string` | Values for `:name` path parameters. |
| `requires` | list of `string` | Features this file needs to run. Maintained by Fluxwire; unsupported ones make the file non-runnable. |
| `seq` | `integer` | Ordering within the parent. |
| `settings` | `SettingsSchema` or `null` | Per-request settings. |
| `tags` | list of `string` | Used by filters such as `--filter tag:smoke`. |
| `vars` | map of `string` | Variables set before the send. |


### Settings

| Key | Type | Description |
|---|---|---|
| `follow_redirects` | `boolean` |  |
| `http_version` | `string` |  |
| `max_redirects` | `integer` |  |
| `run_mode` | `string` | Folders only: `sequential` or `concurrent`. |
| `timeout_ms` | `integer` |  |
| `verify_tls` | `boolean` |  |
