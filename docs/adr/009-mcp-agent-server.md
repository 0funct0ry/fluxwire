# ADR-009: MCP agent server

Status: proposed (M9.1–M9.8)

Numbering note: 005 and 008 are each used by two ADRs. They are left as they are; this one takes the next free number.

## Context

UI work has become the bottleneck. An AI agent can edit GPUI code but cannot run the app, see it, drive it, or read its state, so every defect round-trips through a person. The goal is to let an agent test Fluxwire thoroughly while it is being built: the engine (parse, format, send, run) and the live app (state, input, screenshots, audits).

SPEC planned `fluxwire mcp serve` for M23 (Streamable HTTP on 127.0.0.1, plus stdio). This ADR pulls a local server forward, changes its transports to stdio and unix socket, and adds an in-app host for development. It amends SPEC §5.1 (a `fluxwire-agent` crate), §18 (an `mcp` subcommand) and M23 item 4 (HTTP stays there; stdio and unix socket arrive in M9.1).

## Decision

**One tool layer, two hosts.** A new GPUI-free crate, `fluxwire-agent`, holds the transports, the tool registry and the engine tools, built on `rmcp` with `schemars` for tool schemas. `fluxwire mcp serve` hosts it headless (no GPUI). The GUI hosts the same layer in-process behind the cargo feature `dev-mcp`, which is off in release builds like `runtime-shaders` is opt-in, and adds `ui.*` tools through a `UiBridge` trait that `fluxwire-ui` implements. Only `fluxwire-ui` and `fluxwire` touch GPUI; the crate-boundary rule is unchanged.

**Transports are configurable.** `--transport stdio|unix|both` and `--socket PATH`, with the same keys under `[mcp]` in `fluxwire.toml` or user settings (flags win). The default socket is `$XDG_RUNTIME_DIR/fluxwire/mcp-<pid>.sock` (macOS: the per-user temp directory), mode 0600, removed on exit, with a stale-socket check on start. There is no TCP listener. The unix transport is `cfg(unix)`; Windows gets stdio only (named pipes deferred). In the GUI, `fluxwire --mcp-stdio` takes over stdin and stdout and logs to stderr or a file.

**Main-thread bridge.** The server runs on the shared Tokio runtime. `ui.*` calls cross to the GPUI thread over a channel drained by `cx.spawn`, each carrying a oneshot reply. Tools never touch GPUI entities off-thread.

**One home for engine setup.** Scope, environment, net and settings assembly lives in `fluxwire-ui` today (`response.rs`, `runs.rs`). M9.2 moves it into a GPUI-free helper used by the UI, MCP and the M10 CLI, so `build_scope` stays the single source of precedence.

**Safety.**
- Tools belong to classes `read`, `write`, `execute` and `ui`. `--allow` selects classes; the default is `read,execute`. `ui` exists only in `dev-mcp` builds.
- Every path is checked against a Workspace allow-list. Writes go only through the span editor and canonical formatter (SPEC §7.10).
- Script trust follows the app, not `fluxwire run`: MCP reads `trust.toml` (`is_trusted`), never writes it, never trusts on its own, and runs untrusted Collections with scripts off.
- Output reuses `RunResult::redact` and `report::to_json`, so secrets never leave in a tool result. History rows written by an agent carry origin `agent` (a store migration; the `run_id` namespace is not enough).

**Finding UI and UX defects.** Functional defects: state snapshots, driven scenarios and assertions. Visual defects: screenshots, at light and dark and several window sizes. Structural and UX defects: a semantic UI tree and an `ui.audit` linter (clipping, overlap, small targets, contrast against theme tokens, hard-coded colors, unreachable actions, missing empty/error/loading states, frame and latency budget). Friction: scripted task walkthroughs that report step and keystroke counts. Subjective taste is not automated; screenshots exist for review.

**Screenshots at a given state are a deliverable.** gpui 0.2.2 has no render-to-image and `unsafe_code` is forbidden, so capture is a `Capture` trait with tiers tried in order:
1. *macOS window region (primary).* The app reports its window bounds in screen coordinates and scale factor and runs `screencapture -x -R x,y,w,h`. It needs Screen Recording permission for the parent process; a blank or denied capture is detected and reported with the permission to grant.
2. *Linux.* `grim` or `import` (xvfb in CI). Windows is deferred.
3. *Offscreen render.* If a gpui bump (revisit ADR-001) exposes render-to-image, it becomes tier 0, with no permission and no visible window. M9.3 checks upstream and records the result here.

The app brings its window to the front and waits for idle (no pending run events, animation or layout change, fonts loaded) before capturing. `ui.screenshot` takes optional `state`: a list of steps, or a named state from `fixtures/ui-states/*.yaml` (for example `response-200-json`, `folder-run-failed`, `ws-session-open`, `trust-prompt`). The result is the image plus the `ui.state` JSON and the size, theme and scale used, so image and state always match.

**Rejected.** Headless only (does not unblock UI work); an always-on server in release builds (attack surface); hand-rolled JSON-RPC (compliance and schema upkeep); Streamable HTTP now (stays in M23).

## Consequences

- `rmcp` becomes a dependency of `fluxwire-agent` (M20's MCP client work reuses it). `clap` arrives in `fluxwire-cli` in M9.1 with only `mcp serve`; M10 extends the same command tree rather than replacing it.
- The store gains an origin column (migration). Release builds contain no `ui.*` code.
- Tool names and schemas become a public agent-facing API; changes after M9.8 need a note in the changelog.
- Screenshots on macOS depend on a user-granted permission and a visible window; CI covers Linux under xvfb only until an offscreen path exists.
- Tools are documented in `docs/agent-testing.md` (M9.8); the headless server, not the GUI, is what ships to users before M23.

## M9.1 notes

- `rmcp` 3.8 with a hand-written `ServerHandler` over our own registry, not the `#[tool]` macros: class filtering must apply to both `tools/list` and `tools/call`, and tool schemas still come from `schemars` types.
- A refused class returns a protocol error (`invalid_params`), not a tool error, so an agent cannot mistake it for a failed run.
- With `--transport both`, stdin closing ends only the stdio session; the socket keeps serving until SIGINT/SIGTERM.
- Not yet: any `ui.*` tool, `fluxwire --mcp-stdio` in the GUI, named pipes on Windows.

## M9.2 notes

- Engine setup moved to `fluxwire_runner::prepare` (`Prep`, `limits_from`, `has_scripts`, `collection_root_of`) rather than a new crate: the runner already depends on format, vars and script. `fluxwire-agent` now depends on the runner and the store.
- Tool handlers stay synchronous; the server runs each on the blocking pool (`spawn_blocking`) so a send or run can wait on runner events. `cancel` addresses a live send or run by a caller-chosen `cancel_key` (default: the run id).
- Untrusted Collections: the runner refuses a request that has scripts when `trusted` is false (`RunError::Untrusted`), so the request is not sent and the result says `scripts: not run…`. Requests without scripts run normally. MCP never calls `set_trusted`.
- MCP runs are stateless: no persisted cookies and no unsaved-editor overlay; environment files come from the allowed root that contains the request.
- WebSocket requests are not sent through `send_request` (sessions need the M10 transcript flow).
- History rows written by the app have `origin = NULL`; MCP rows have `origin = 'agent'`.
