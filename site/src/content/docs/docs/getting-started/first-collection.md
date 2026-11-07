---
title: Your first collection
description: How a Workspace, Collection, Folder, and Request map to files and directories.
---

A *Workspace* is a folder you open. A *Collection* is a directory that contains `fluxwire.toml`. Everything inside is
ordinary files you can commit.

```text
my-workspace/
├── environments/
│   ├── local.env
│   └── staging.env
└── payments-api/                 ← Collection
    ├── fluxwire.toml             ← format = 1
    ├── collection.flux.md        ← collection name, auth, defaults, docs
    ├── lib/sign.ts               ← shared script helpers
    └── payments/                 ← Folder
        ├── folder.flux.md        ← folder defaults, auth, run mode
        ├── create-payment.flux.md
        └── get-payment.flux.md   ← Request
```

## Create a Collection

Choose **New collection** from the workspace header or the palette. Fluxwire writes `fluxwire.toml` with
`format = 1` and a `collection.flux.md`. A higher `format` number than the app understands refuses to open.

## Write a Request

```` markdown
---
name: Get payment
auth: inherit
---

# Get payment

Fetches a payment by id.

```http
GET {{baseUrl}}/v1/payments/:id
Accept: application/json
```

```http expect
HTTP 200
```
````

- Only files that end in `.flux.md` are Requests. Other Markdown files, such as `README.md`, are shown as docs and
  never executed.
- Add a `folder.flux.md` to share headers, auth, variables, and scripts with every Request below it.
- Folders inherit from their parent Folder, then the Collection. See
  [Environments and secrets](../../guides/environments-and-secrets/) for variable precedence.

Continue with [The `.flux.md` format](../../reference/flux-format/) for the complete rules.
