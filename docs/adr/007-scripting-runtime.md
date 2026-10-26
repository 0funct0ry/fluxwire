# ADR-007: Scripting runtime, expect checking, and script trust

Status: accepted (M7)

## Decision

**Engine.** `fluxwire-script` embeds QuickJS through `rquickjs` 0.14. Every block runs as its own ES module (`<file>#<role>-<n>`), in file order, inside one runtime per phase. TypeScript is only type-stripped by `oxc` (parser, semantic, transformer, codegen with a source map); nothing is type-checked. `oxc` is pinned to 0.100 because later releases need a newer Rust than the workspace MSRV (1.89). Note that the TypeScript transform removes unused imports, so an import only counts as an import when it is used or side-effect only.

**Runtimes are not pooled.** SPEC §5.3 says "pooled and reset"; a `Runtime` + `Context` costs well under the 2 ms script budget (§20) to create, and a fresh one per phase removes any chance of state leaking between sends. Revisit with a pool if the folder-run benchmark (M9) shows the cost.

**Limits** (SPEC §11.1): 5 s wall time through the interrupt handler (reset per block), 64 MiB heap, 512 KiB stack. `[scripts] timeout_ms` and `memory_mb` in `fluxwire.toml` override them per collection (SPEC §6.1 lists no keys for this, so the table name is ours). A heap that is exhausted leaves no readable exception behind; that is reported as S003.

**Sandbox.** The context has no `std`/`os` modules, `fetch`, `process` or timers. Imports resolve lexically and only to `lib/…` under the collection, and the loader canonicalises the path and rejects anything outside `lib/` (symlinks included). The only network path is `fw.sendRequest`, which calls back into the runner through the `ScriptHost` trait, so `fluxwire-script` stays protocol-free.

**Error mapping.** Each module keeps the oxc source map; an exception's first stack frame inside a known module becomes `file:line` in the original `.flux.md` (or `lib/x.ts`). Diagnostic codes (SPEC defines none for scripts): `S001` uncaught exception, `S002` time limit, `S003` memory limit, `S004` import outside `lib/` or unresolvable, `S005` syntax error, `S006` unsupported (`pm.*`, `require()`).

**Boundary.** Scripts see the exchange as JSON (`fw.request`, `fw.response`) produced by the protocol, and record changes in a `ScriptState` (variables, request edits, tests, console lines). Four new optional `Protocol` methods connect them: `script_request(doc, scope, plan)`, `apply_script_ops(plan, ops)`, `script_response(outcome)` and `capture(outcome, expr)`; `check` now takes a `CheckCtx { doc, scope }` so expect blocks are interpolated and their bodies can be read. The runner contains no HTTP types.

**Pipeline** (SPEC §11.2): collection → folder → request `pre`; plan again with the variables scripts set, then apply the recorded request edits on top of the plan; send; `capture`; request → folder → collection `post`; `expect`; collection → folder → request `test`. A failing `pre` script blocks the send (`RunError::Script`); failures in `post`/`test` keep the response and appear as `script_errors`. `fw.skip(reason)` in a `pre` block ends the run without sending.

**Expect.** `HttpProtocol::check` implements SPEC §7.7: status (exact, class, any), version, headers (exact, `{{*}}`, `~/re/`, names case-insensitive) and body. A JSON expected body is a partial deep match that reports one test per expected leaf (`expect body $.customer.id`), arrays must have equal length, `"{{*}}"` requires the key to exist. A non-JSON body must equal the trimmed response. `serde_json` is built with `preserve_order` so tests come out in the order the block lists them.

**`pm` shim.** A JS prelude (`pm.js`), loaded only when a block mentions `pm.` or `require(`, maps `pm.*` onto `fw` and a Chai BDD subset. Unknown members return a proxy that throws `Unsupported in Fluxwire: pm.visualizer.set` when called. `pm.sendRequest` sends an ad-hoc request through the same host.

**Trust (SPEC §21).** The first send from a collection that has scripts at any level shows a banner (Trust / Not now). Decisions are stored per canonical collection path in `trust.toml` in the user config directory (`fluxwire_format::{is_trusted, set_trusted}`), not in `settings.toml`, so the app never rewrites a hand-edited settings file. Palette actions `scripts: trust this collection` and `scripts: revoke trust for this collection` flip it. `fluxwire run` sets `ScriptOptions::trusted` itself. Without a config directory (tests) trust is session-only.

**`fw.d.ts`.** `fluxwire_script::fw_dts()` is written to `<collection>/.fluxwire/types/fw.d.ts` when a workspace opens and by the palette action `scripts: regenerate fw.d.ts`.

## Consequences

- Variables a `pre` script sets are not known to the UI's unresolved-variable warning, so a request whose variable comes from a script warns once before the first send.
- `fw.env.set` and `fw.vars.set` both write runtime variables; `fw.env.persist(name)` writes the current value to the active environment's `.local.env`.
- Nested `fw.sendRequest` runs the other file's scripts too, up to three levels deep.
