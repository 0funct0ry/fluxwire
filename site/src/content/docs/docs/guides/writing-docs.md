---
title: Writing docs
description: Document a Collection with Markdown that renders in the app and on GitHub, and export it as static HTML.
---

The prose in a `.flux.md` file is its documentation. Collection and Folder files carry their own prose, and any other
`.md` file, such as `README.md`, is shown as a doc page and never executed.

In the **Docs** tab, choose **Edit**, **Preview**, or **Split**. Executable blocks appear as chips (method and URL,
tests, scripts) so the document reads cleanly. Links between files are checked, and headings get stable anchors.

## Examples from real responses

Use **Save as example** on a response to store it as a `<lang> example` block. Example blocks are documentation only
and are never executed.

## Export

```sh
fluxwire docs export payments-api --out site-out
```

This writes a single-page HTML reference with no JavaScript. The `fw` API and command-line pages on this site are
generated in the same spirit, from the source, so they cannot drift.
