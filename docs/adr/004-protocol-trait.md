# ADR-004: The `Protocol` trait and the HTTP engine

Status: accepted (M3)

## Decision

`fluxwire-protocol` defines the `Protocol` trait of SPEC §5.5 and every type it exchanges. The runner, history, CLI and UI only ever see these types; nothing outside `fluxwire-http` names an HTTP type.

- `plan(doc, scope) -> Result<Plan, Vec<Diagnostic>>` is synchronous and pure. `Plan` is `{ protocol, label, warnings, payload }` where the payload is `Arc<dyn Any + Send + Sync>` that only the owning protocol can downcast. `VarScope` (in `fluxwire-vars`) carries literal variables and the `base_dir` that `< path` and `@file()` resolve against; M4 replaces the literals with the precedence chain.
- `execute(plan, ctx) -> BoxStream<Event>` must be called inside a Tokio runtime. The stream is `Phase | Redirect | Frame`\*, then exactly one terminal event: `Done(Outcome)` or `Failed(ExecError)`. Cancelling `ExecCtx::cancel` ends the stream with `Failed { phase: "cancelled" }`; dropping the stream aborts the work.
- `Outcome` is protocol-neutral: `final_url`, `timings: Vec<PhaseTiming>`, `redirects`, `connection_reused`, and for exchange protocols an `Exchange` (status, status text, version label, headers as pairs, `Body`, wire and decoded size). Session protocols (M8) will use `Frame` events and leave `exchange` empty.
- `Body` is `Memory(Bytes)` up to 16 MiB and `File { TempPath, len }` above; the temp file is deleted when the last clone drops. The hard cap defaults to 2 GiB (`ExecSettings::max_body`).
- `ExecError { phase, message, fix }` names the failed phase and a fix (SPEC §17.8).
- `check(outcome, expect, cx) -> Vec<TestResult>` implements `http expect` (status, version, headers, JSON and text bodies since M7; see ADR-007).
- Two optional hooks, `to_curl(plan, mask_secrets)` and `edits_from_paste(text, doc)`, default to `None`. They let "Copy as cURL" and "paste cURL" work through the registry without the UI or runner knowing about HTTP. (`edits_from_paste` rather than `from_pasted` because clippy reserves `from_*` for constructors.)
- `fluxwire_protocol::conformance::run` is the shared conformance suite: deterministic planning, event ordering, terminal event, history record shape, `check` pass and fail, and prompt cancellation. `fluxwire-http` runs it against an in-process axum server.

`fluxwire-runner` owns the shared multi-thread Tokio runtime and a registry of `Arc<dyn Protocol>` (HTTP behind the default `http` feature). `Runner::send` takes the tab's in-memory `FluxDoc`, so sending never requires saving. The UI bridges with `cx.spawn` over an unbounded channel.

## HTTP engine choices

- hyper 1.x client connections driven directly (`http1`/`http2::handshake`) over our own connector, rather than hyper-util's pooled client, because phase timings need to see DNS, connect and TLS separately. A small pool keyed by scheme, host, port, proxy and verify flag keeps one multiplexed h2 sender or a stack of idle h1 senders; reused connections report `dns`, `connect` and `tls` as skipped.
- rustls with the ring provider and `rustls-platform-verifier` (extra roots for configured CAs); `verify_tls = false` swaps in an accept-all verifier. Per-host client certificates (PEM) come from `fluxwire.local.toml`.
- DNS uses `tokio::net::lookup_host` (the system resolver) instead of hickory-resolver. It is timed the same way, honours `/etc/hosts` and platform resolver settings, and avoids a second resolver stack. Revisit if per-request resolver control is needed.
- Proxy: `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` or `[proxy]` in `fluxwire.local.toml`; HTTP proxies only (CONNECT for https targets). SOCKS5 is deferred.
- Redirects are followed manually so the chain is recorded; 301/302/303 turn non-GET/HEAD into GET and drop the body; credentials are dropped when the origin changes. The Timeline shows the final hop plus a leading `redirects` span for earlier hops.
- Responses are decoded while streaming (gzip, zlib "deflate", brotli, zstd) into a sink that spills to a temp file.
- Digest auth answers one 401 challenge (MD5 or SHA-256, `qop=auth`). `inherit` is treated as `none` until M4; `oauth2` reports "arrives in M4".

## Response viewer

Bodies are turned into a `BodyView` on a background thread: content sniffing, a streaming JSON pretty-printer that keeps key order and number text (no parse into a tree), a line index (8 bytes per line), fold regions, and a hex dump. The list is a GPUI `uniform_list` of fixed 20 px rows with per-line JSON lexing, so cost is proportional to the visible rows, not the body size. Wrap re-splits long lines at 140 characters instead of using variable-height rows. JSONPath supports `$`, `.name`, `['name']`, `[n]`, `[-n]`, `[*]`, `.*` and `..name`.

## Consequences

- Adding a protocol means implementing the trait in its own crate and registering it in the runner; the UI, history and CLI need no changes.
- The `Plan` payload is opaque, so cross-protocol features (for example showing a planned request before sending) must go through new trait methods.
