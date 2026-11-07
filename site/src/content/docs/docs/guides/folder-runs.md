---
title: Folder runs
description: Run a Folder or Collection sequentially or concurrently, with data files, filters, and exports.
---

A *Run* executes many Requests and reports each result. Right-click a Folder and choose **Run folder**, press
<kbd>Cmd/Ctrl+Shift+R</kbd>, or use the palette entry `run: run folder`.

- **Sequential or concurrent.** A Folder with `settings: run_mode: sequential` runs in `seq` order and shares
  variables, so a login can feed later Requests. Concurrent runs accept a concurrency limit.
- **Data files.** Choose a CSV (header row names the variables) or a JSON array of objects. The run repeats once per
  row, and `{{column}}` resolves in each Request.
- **Filters.** `tag:smoke`, `name:*refund*`, or a name glob; separate several with commas.
- **Bail.** Stop starting new Requests after the first failure.
- **Cancel.** In-flight Requests are cancelled and the rest are skipped.
- **Re-run failed.** Run only the Requests that failed.
- **Export.** JSON and JUnit exports contain the whole tree with secrets redacted. **Copy CLI command** gives the
  equivalent `fluxwire run` invocation.

Every Run is recorded in History with a run id, and **View in History** filters to it.

The command line uses the same engine; see [Running in CI](../running-in-ci/).
