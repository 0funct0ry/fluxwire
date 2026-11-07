---
title: Importing
description: Import Postman, Insomnia, OpenAPI, Hurl, and .http sources, with a report of anything that did not convert.
---

Fluxwire imports these sources:

| Source | What converts |
|---|---|
| Postman | v2.0 and v2.1 collections and environments, including scripts through the `pm` layer. |
| Insomnia | v4 JSON and v5 YAML exports. |
| OpenAPI 3.x and Swagger 2.0 | One Request per operation, with examples. |
| Hurl | A directory of `.hurl` files. |
| IntelliJ and VS Code `.http` | Requests, variables, and basic scripts. |

## In the app

Use **Import…** from the workspace header or the palette. The wizard detects the format, shows a plan, and ends with a
report. Existing files are never overwritten unless you ask.

## From the command line

```sh
fluxwire import auto ./payments.postman_collection.json --into ./payments-api --report import-report.md
fluxwire import openapi ./openapi/payments.yaml --into ./payments-api --spec-link
fluxwire import postman ./collection.json --into ./out --dry-run
```

`--dry-run` prints the report and writes nothing. `--spec-link` copies an OpenAPI spec into the Collection and creates
a link for [OpenAPI sync](../openapi-sync/).

## The report

Everything that did not convert cleanly is listed: unsupported auth, scripts that use npm libraries, and secrets that
were found in the source. Secrets are moved to an Environment and flagged so they are not committed by accident.
