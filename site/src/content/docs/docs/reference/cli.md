---
title: Command-line reference
description: Every fluxwire command and flag, generated from the clap command tree.
---

The `fluxwire` binary opens the app; `fluxwire-cli` is the static, GUI-free build for CI. Both accept the same
commands. This page is generated from the clap definitions, so it matches the shipped binary.

Exit codes are listed on the [Exit codes](../exit-codes/) page. Shell completions: `fluxwire completions <SHELL>`.

## `fluxwire run`

Run requests, folders or collections headlessly

```text
Usage: fluxwire run [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...
          Requests, folders or collections

Options:
  -e, --env <NAME>
          Environment to use

      --var <NAME=VALUE>
          Variable override; repeatable

      --insecure
          Skip TLS verification (prints a warning)

      --env-file <FILE>
          Extra dotenv file (just below --var)

      --concurrency <N>
          Max parallel requests

      --sequential
          Force sequential mode

      --data <FILE>
          CSV or JSON data file for iterations

      --iterations <N>
          

      --filter <EXPR>
          Filter, e.g. tag:smoke or name:*refund*

      --bail
          Stop at first failure

      --timeout <MS>
          Per-request timeout override in milliseconds

  -r, --reporter <REPORTER>
          Reporter; repeatable
          
          [possible values: pretty, json, junit, tap]

  -o, --output <FILE>
          Output file for the last reporter

      --no-color
          

      --quiet
          

      --verbose
          

  -h, --help
          Print help
```

## `fluxwire send`

Send one request and print the response body (status and timing go to stderr)

```text
Usage: fluxwire send [OPTIONS] <FILE>

Arguments:
  <FILE>
          

Options:
  -e, --env <NAME>
          Environment to use

      --var <NAME=VALUE>
          Variable override; repeatable

      --insecure
          Skip TLS verification (prints a warning)

  -i, --include
          Print the status line and headers before the body

  -h, --help
          Print help
```

## `fluxwire fmt`

Canonical formatting

```text
Usage: fluxwire fmt [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...
          

Options:
      --check
          Report non-canonical files and exit 1 instead of rewriting

  -h, --help
          Print help
```

## `fluxwire validate`

Parse files and report diagnostics

```text
Usage: fluxwire validate <PATHS>...

Arguments:
  <PATHS>...
          

Options:
  -h, --help
          Print help
```

## `fluxwire env list`

List the environments of a Workspace

```text
Usage: fluxwire env list <PATH>

Arguments:
  <PATH>
          

Options:
  -h, --help
          Print help
```

## `fluxwire completions`

Print a shell completion script

```text
Usage: fluxwire completions <SHELL>

Arguments:
  <SHELL>
          [possible values: bash, elvish, fish, powershell, zsh]

Options:
  -h, --help
          Print help
```

## `fluxwire import`

Import a Postman, Insomnia, OpenAPI, Hurl or .http source into a folder (SPEC §14)

```text
Usage: fluxwire import [OPTIONS] --into <DIR> <SOURCE> <SRC>

Arguments:
  <SOURCE>
          postman, insomnia, openapi, hurl or http (use `auto` to detect from the file)

  <SRC>
          The file to import (a directory of `.hurl` files for hurl)

Options:
      --into <DIR>
          Folder to write into. Existing files are never overwritten unless --overwrite is given

      --report <FILE>
          Write the import report (Markdown) to this file

      --overwrite
          Overwrite files that already exist in --into

      --dry-run
          Plan only: print the report and write nothing

      --spec-link
          OpenAPI: copy the spec into the collection and create a Spec link

      --name <NAME>
          Collection name (default: the name in the source)

  -h, --help
          Print help
```

## `fluxwire openapi status`

Show incoming, outgoing and conflicting changes

```text
Usage: fluxwire openapi status

Options:
  -h, --help
          Print help
```

## `fluxwire openapi pull`

Apply incoming spec changes to the collection

```text
Usage: fluxwire openapi pull

Options:
  -h, --help
          Print help
```

## `fluxwire openapi push`

Apply outgoing collection changes to the spec

```text
Usage: fluxwire openapi push [OPTIONS]

Options:
      --remove
          Remove deleted operations instead of marking them deprecated

  -h, --help
          Print help
```

## `fluxwire openapi diff`

Print every change, and the patch a push would make

```text
Usage: fluxwire openapi diff

Options:
  -h, --help
          Print help
```

## `fluxwire docs export`

Write a single-page, no-JavaScript HTML reference of a collection, folder or file

```text
Usage: fluxwire docs export --out <DIR> <PATH>

Arguments:
  <PATH>
          A collection or folder directory, or one `.flux.md` / `README.md` file

Options:
      --out <DIR>
          Directory to write `index.html` (and `assets/`) into

  -h, --help
          Print help
```

## `fluxwire mcp serve`

Serve Fluxwire tools over stdio and/or a unix socket. Logs go to stderr

```text
Usage: fluxwire mcp serve [OPTIONS]

Options:
      --transport <stdio|unix|both>
          stdio, unix or both (default stdio; `unix` and `both` are Unix-only)

      --socket <PATH>
          Socket path for the unix transport (default: per-user runtime dir, mcp-<pid>.sock)

      --allow <CLASSES>
          Tool classes to expose (default read,execute)

      --root <PATH>
          Workspace allow-list; repeatable

  -h, --help
          Print help
```

## `fluxwire mcp scenario`

Run a scenario file against a running `dev-mcp` app over its socket. Exit 0 when every step passes, 1 when one fails, 2 when the app cannot be reached

```text
Usage: fluxwire mcp scenario [OPTIONS] --socket <PATH> <FILE>

Arguments:
  <FILE>
          Scenario file, such as fixtures/scenarios/m03-send.yaml

Options:
      --socket <PATH>
          Socket of the app started with `--mcp-socket`

      --no-capture
          Skip screenshot checkpoints (no display or Screen Recording permission)

      --out <DIR>
          Write checkpoint and failure screenshots here

  -h, --help
          Print help
```

