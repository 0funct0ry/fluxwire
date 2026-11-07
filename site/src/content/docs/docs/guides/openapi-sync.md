---
title: OpenAPI sync
description: Link a Folder to an OpenAPI spec and merge changes in both directions.
---

A *Spec link* ties a Folder to an OpenAPI file. Fluxwire compares the spec, the Folder, and the last synced state, so
it can tell which side changed.

- **Pull** brings new operations, parameters, and descriptions from the spec into Requests.
- **Push** writes renamed Requests and new parameters back into the spec. YAML comments, key order, and formatting
  are preserved; constructs that cannot be edited safely, such as anchors and aliases, are shown as a patch for you
  to apply.
- **Conflicts** (both sides changed the same field) wait for your decision and block pull and push until resolved.
- Deleted operations are marked `deprecated` by default; use `--remove` to remove them.

Requests reference an operation with the `operation` front matter key (an `operationId` or `METHOD /path`).

## From the command line

```sh
fluxwire openapi status --root payments-api
fluxwire openapi diff   --root payments-api
fluxwire openapi pull   --root payments-api --yes
fluxwire openapi push   --root payments-api --link payments
```

`status` and `diff` change nothing. Conflicts make pull and push exit with code `4`.
