# ADR-008: Folder runs

Status: accepted (M9)

## Decision

**Units, not modes.** A run selects requests in tree order (`seq`, then name) and splits them into *units*. In concurrent mode every request is its own unit, except that the requests under a folder (or collection) whose `settings.run_mode` is `sequential` form one unit. `--sequential` puts the whole run in one unit. A unit runs its requests in order and shares runtime variables between them (`var_changes` are replayed into the next request's scope); units run concurrently, bounded by one semaphore (`concurrency`, default 8). Each concurrent request therefore starts from the run's starting scope and cannot see another unit's captures.

**One engine.** `fluxwire-runner::folder` calls the same `exec` as a single send, so scripts, expect blocks, captures and auth behave identically. Ancestor docs are loaded once per folder per run and handed to `exec` instead of being re-read per request; the scope is still built per request (small files) so `build_scope` stays the single source of precedence.

**Data rows** are set at `Layer::Cli`, just below runtime variables (SPEC §12 "level 2"). Each iteration starts from a fresh scope, so captures do not leak between rows. `--iterations N` without a data file repeats with empty rows.

**CSV/JSON data without new dependencies.** JSON uses `serde_json`; CSV is a 40-line RFC 4180 reader (quotes, escaped quotes, CRLF, BOM). Revisit with the `csv` crate if real files need more.

**Cancel, bail, timeout.** One `CancellationToken` per run is the `ExecCtx` token of every request, so cancel and the run timeout abort in-flight requests. Bail does not cancel: it sets a flag so unstarted requests are recorded as *skipped* (in-flight ones finish). Unstarted requests always appear in the result so the tree is complete.

**Results.** `FolderRunResult` holds `Arc<RequestRun>` per request per iteration (the existing `RunResult` plus timing offsets). `to_json` / `to_junit` live in the runner so the M10 reporters share them; both redact secrets. JUnit has one suite per folder and one case per request (failed expectations become `<failure>` children).

**History.** The UI inserts one entry per finished request with the run id (a time-sortable id from the runner). The store gains an index on `(workspace, run_id)` and exposes `run_id` on `Summary`; the History panel filters by it.

**Saved files only.** Runs read request files from disk, not open editor buffers (the runner resolves folders and docs itself, like the CLI will). The tab warns when dirty files exist under the folder.

## Consequences

- The ADR-007 note about pooling QuickJS runtimes was not needed: the 1,000-request, concurrency-32 benchmark (a test in `crates/fluxwire-runner/tests/folder.rs`) runs well under the 3 s budget, in debug builds too, against requests without scripts. Revisit if script-heavy folders are slow.
- Run tabs restore without results (options and results are not persisted).
- WebSocket requests are skipped by folder runs; session protocols need a stop condition (M21).
