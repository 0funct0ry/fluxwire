---
title: Configuration
description: The files that configure a Collection, a Workspace, and the app.
---

| File | Scope | Purpose |
|---|---|---|
| `fluxwire.toml` | Collection root | Declares `format = 1`, lists `[secrets]`, and holds Collection-level options. Commit it. |
| `fluxwire.local.toml` | Collection root | Local overrides such as TLS settings. Keep it out of git. |
| `environments/*.env` | Workspace | One file per Environment. See [Environments and secrets](../../guides/environments-and-secrets/). |
| `settings.toml` | User config directory | App-wide settings. |
| `keymap.toml` | User config directory | Keybinding overrides. Edit with `keymap: edit`. |
| `.fluxwire/types/fw.d.ts` | Collection | Editor typings for the [`fw` API](../fw-api/), written by Fluxwire. |

## `settings.toml`

```toml
theme = "One Dark"            # any of the bundled themes, or a Zed theme JSON you add
check_for_updates = false     # default off; nothing is contacted unless you turn it on
allow_process_env = false     # allow {{process.env.NAME}}
persist_cookies = false

[globals]
region = "eu"

[editor]
# editor options: soft wrap, tab size, and similar

[mcp]
# options for `fluxwire mcp serve`
```

Fluxwire edits `settings.toml` in place: comments and keys it does not own are preserved. A file that does not parse
is left unchanged and the app reports the error.

## Themes

Fluxwire bundles themes including One Dark and One Light. Zed theme JSON and microterm TOML themes load as they are.
Use `theme: select…` from the palette, or type `~` in the palette to preview a theme as you move through the list.

## Crash logs

Crash logs are written locally. After a crash, **Open crash log** appears on the next launch. Nothing is sent
anywhere.
