# Agent testing guide

Fluxwire can be driven and tested by an agent (Claude Code or any MCP client) through the
Model Context Protocol. Design: [ADR-009](adr/009-mcp-agent-server.md).

## Two ways to run it

| Mode | Start | Tools |
| --- | --- | --- |
| Headless server | `fluxwire mcp serve` | `ping`, `server_info`, read, write and execute engine tools |
| Live app (`dev-mcp`) | `cargo run -p fluxwire --features dev-mcp -- --mcp-socket /tmp/fw.sock fixtures/payments-api` | the engine tools plus every `ui.*` tool |

On macOS without the Xcode Metal toolchain add `runtime-shaders` to `--features`.
`dev-mcp` is off by default and absent from release builds: a release binary contains no MCP
server code or flags.

## Setup

`.mcp.json` for Claude Code, headless over stdio:

```json
{
  "mcpServers": {
    "fluxwire": {
      "command": "fluxwire",
      "args": ["mcp", "serve", "--allow", "read,write,execute", "--root", "fixtures/payments-api"]
    }
  }
}
```

The live app over stdio (stdout is reserved for the protocol):

```json
{
  "mcpServers": {
    "fluxwire-ui": {
      "command": "cargo",
      "args": ["run", "-q", "-p", "fluxwire", "--features", "dev-mcp", "--",
               "--mcp-stdio", "fixtures/payments-api"]
    }
  }
}
```

The live app over its unix socket: start it with `--mcp-socket /tmp/fw.sock` and point a
client at that path (`fluxwire mcp scenario` below is one). The socket is mode 0600 and removed
on exit. There is no TCP listener; Windows has stdio only.

## Tool catalog

Class `read`, `write` and `execute` tools are selected with `--allow` (default `read,execute`).
The `ui` class is available only when a live app hosts the server.

| Group | Tools |
| --- | --- |
| Server | `ping`, `server_info` |
| Read | `list_collections`, `list_requests`, `read_request`, `diagnostics`, `list_environments`, `resolve_vars`, `history_query` |
| Write | `write_request`, `fmt` (span editor and canonical formatter; `dry_run` returns the diff) |
| Execute | `send_request`, `run_folder`, `cancel` |
| UI state | `ui.state`, `ui.open_workspace`, `ui.open_request`, `ui.tree`, `ui.find` |
| UI input | `ui.list_actions`, `ui.invoke_action`, `ui.command`, `ui.key`, `ui.type_text`, `ui.click_element`, `ui.wait_for_idle`, `ui.wait_for` |
| Screenshots | `ui.screenshot`, `ui.screenshot_all`, `ui.set_theme`, `ui.resize` |
| Audit | `ui.audit`, `ui.walkthrough` |
| Scenarios | `ui.run_scenario`, `ui.record` |
| Events | `ui.events` and logging notifications |

### Scenarios

A scenario is YAML in `fixtures/scenarios/` (`name` must equal the file stem):

```yaml
name: m03-send
workspace: fixtures/payments-api
size: default
steps:
  - step: command            # any ui.screenshot step: open, invoke, key, type, command,
    command: open_file       # click, set_theme, resize, wait_for, wait_idle
    key: payments/get-payment.flux.md
  - step: invoke
    name: Send
  - step: wait_for
    path: /response
  - step: assert             # JSON pointer into ui.state; equals | contains | matches | not
    path: /response/sending
    equals: false
  - step: assert_find        # ui.find: count, exists, enabled
    label: Send
    enabled: true
  - step: click_element      # first visible match by label/role
    label: Send
  - step: screenshot         # checkpoint, attached to the report
    name: response
  - step: audit              # fails on findings at or above fail_on (default error)
  - step: wait_event         # wait for a new event of a kind, optionally containing text
    kind: run
    contains: finished
```

`matches` supports literals, `.*`, `^` and `$` only. `ui.run_scenario` takes a `name` or an
inline `scenario` and returns `{name, passed, steps[], failure?}`. A failure carries the step,
the expected and actual value, the `ui.state` and a screenshot. `ui.record` with `start` then
`stop` writes the state-changing calls in between as a scenario (`save: true` writes it to
`fixtures/scenarios`).

From a shell or CI, with the app already running on a socket:

```bash
fluxwire mcp scenario fixtures/scenarios/m02-collections.yaml --socket /tmp/fw.sock --out target/shots
```

Exit code 0 when every step passes, 1 on a failed step, 2 when the app cannot be reached. Use
`--no-capture` where there is no display or no Screen Recording permission.

### Events

`ui.events {since, kinds, wait_ms}` returns `{events, latest, gap}`. Events have a `seq`, `ts`,
`kind` (`run`, `frame`, `notice`, `console`, `generation`) and `data`. Pass the last `latest`
as `since` to continue; `wait_ms` blocks for the first match, so an agent can wait for a run to
finish instead of polling `ui.state`. The buffer keeps the last 1024 events; `gap: true` means
older ones were dropped. The server also pushes every event as an MCP logging notification
(logger `fluxwire.events`) to clients that connected after the app started.

## Safety model

- Tool classes: a tool outside the allowed classes is neither listed nor callable.
- Writes go only through the span editor and canonical formatter and only under `--root`.
- Scripts run only in a Collection trusted in `trust.toml`; MCP reads that file and never writes it.
- Results are redacted with the run's secrets; `list_environments` returns names, never values.
- History rows written through MCP carry the origin `agent`.
- `ui.*` tools never touch GPUI entities off the UI thread; calls time out after 5 seconds.
- `dev-mcp` is a development feature; do not ship it.

## macOS Screen Recording

Screenshots use `screencapture` on macOS. Grant Screen Recording to the app that starts
Fluxwire (Terminal, iTerm, your editor) under System Settings, Privacy and Security. Without it
the tool returns an error that names the permission; it never returns a blank image. On Linux
install `grim` or ImageMagick (`import`); headless runs use `xvfb-run`.

## Bug-hunting playbook

1. **Orient.** `server_info`, then `ui.state` to learn the Workspace, tabs and `generation`.
2. **Run the scenarios.** `ui.run_scenario` for each name in `fixtures/scenarios`. Fix or file every failure before exploring.
3. **Audit.** `ui.audit` with `themes: ["one-dark","one-light"]` and `sizes: ["default","min"]`. Treat errors as bugs; triage warnings.
4. **Sweep states.** `ui.screenshot_all` over the named states in `fixtures/ui-states` in both themes; look for clipping, overlap and empty areas.
5. **Probe a flow by keyboard.** `ui.list_actions` for what is enabled, `ui.invoke_action` and `ui.key` to do it, `ui.events` with `wait_ms` to wait. A disabled action must return an error.
6. **Try the edges.** A malformed Request (`fixtures/format-edge`), an unreachable host, an untrusted script, a cancelled folder run, a closed WebSocket.
7. **Reduce.** Turn the failing path into a scenario with `ui.record`, add the assertion that fails, and attach the report and screenshot to the bug.
8. **Compare cost.** `ui.walkthrough` on a common task before and after a change; compare keystrokes, clicks and time.
