---
title: Environments and secrets
description: Define variables per Environment, mark secrets, and understand variable precedence.
---

An *Environment* is a named set of variables. Environments are workspace-level: each one is a file in
`<workspace root>/environments/`, for example `environments/staging.env`. One Environment is active at a time, shown
in the title bar; press <kbd>Cmd/Ctrl+E</kbd> to switch.

```text
# environments/local.env
baseUrl = http://localhost:8080
merchantId = m_123
clientSecret = dev-only-secret
```

## Using variables

Write `{{name}}` anywhere in a URL, header, or body. Typing `{{` in the URL bar offers completions, including the
dynamic variables such as `{{$uuid}}`, `{{$timestamp}}`, and `{{$randomInt}}`. Hovering a variable shows its value and
the file and line it came from. An unresolved variable raises warning `P101`: the first Send warns, the second sends
anyway.

## Precedence

From highest to lowest precedence:

1. Variables set by scripts at run time (`fw.vars.set`).
2. Command-line `--var NAME=VALUE` overrides, then `--env-file`.
3. Request `vars` in front matter, then Folder and Collection `vars`.
4. The active Environment.
5. Globals from `settings.toml`.

`{{process.env.X}}` resolves only when `allow_process_env = true` is set in `settings.toml`.

## Secrets

Mark a variable as secret in the Environments panel, or list it under `[secrets]` in `fluxwire.toml`. Secret values
display as dots, are revealed for ten seconds on click, and are redacted from History, exports, and **Copy as cURL**.
Commit the environment file with placeholders and keep real values in an untracked local file or your CI secret store.

## Inheritance

`auth`, `vars`, `settings`, `http defaults` headers, and scripts flow from the Collection to Folders to Requests. Set
`auth: none` on a Request to opt out of inherited auth. Auth types: basic, bearer, API key, digest, and OAuth2 (client
credentials, and authorization code with PKCE). Tokens are cached per Environment, and each Environment has its own
cookie jar.
