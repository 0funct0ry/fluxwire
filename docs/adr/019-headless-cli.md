# ADR-019: Headless CLI

Status: accepted (M10)

## Decisions
- `fluxwire-cli` is a thin layer over `fluxwire_runner::prepare` (`Prep`, `FolderOpts`), `Runner::run_folder` and `Runner::send`; the app, MCP and CLI share one execution path. The `fluxwire` binary dispatches to it for any subcommand and never initializes GPUI; `fluxwire-cli` is the same tree built without GPUI (`[[bin]]` in the crate).
- **Exit codes** follow SPEC §18. When one run produces several, precedence is 3 (a script threw) > 2 (a request failed at the network level) > 1 (a failed test or other request failure). A cancelled request is not a failure. Parse failures (65) and unsupported features (66, diagnostic F401) are detected before any request is sent, so a bad file never produces a partial run. Usage errors, unknown paths and the M11–M13 stubs exit 64. `fmt --check` exits 1 when a file is not canonical (the spec is silent).
- `run` trusts script collections by definition (SPEC §21) and always allows `process.env`.
- **Reporters**: `pretty`, `json`, `junit`, `tap`. `-r` repeats; `-o` receives the last reporter and the others go to stdout. `--quiet` hides `pretty` only. With several PATHs, each reporter's text is concatenated. All text is redacted with the run's secrets. Colour is on only for a terminal without `NO_COLOR` or `--no-color`.
- Precedence of variables: environment < `--env-file` < `--var` (both go in the runtime layer).
- `send` writes the body to stdout (redacted), the status line and timing to stderr, and with `-i` the status line and headers to stdout first.
- The app offers `cli: install shell command` / `cli: uninstall shell command` (a `~/.local/bin/fluxwire` symlink, only ever replacing a symlink). A Settings page and a Windows PATH entry are deferred.
