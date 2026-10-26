# ADR 008: Session protocols (WebSocket)

Status: accepted (M8)

## Context

SPEC §5.5 makes `Interaction::Session` the second shape of a `Protocol`. WebSocket is the
reference session protocol. The M3 trait was written against HTTP: `execute` takes a `Plan` and
returns one event stream, and `Outcome` carries a single `Exchange`. A session needs three things
that shape does not give: a way to send frames *into* a live run, a transcript in the result, and
timestamps on frames.

## Decision

- **Commands into a session.** `ExecCtx` gets `session: Option<SessionInbox>`. The runner creates
  a `session_channel()` for every send; the protocol takes the receiver once at the start of
  `execute`. The caller keeps a `SessionHandle` (`send(kind, bytes)`, `close()`), exposed as
  `RunHandle::send_frame` / `close_session`. Exchange protocols ignore the inbox. The trait's
  method set is unchanged, so HTTP needed no code change beyond `session: None`.
- **Frames out.** `Frame` gains `ts_ms` (since run start) and `close_code`; `Event::Frame` is now
  forwarded by the runner as `RunEvent::Frame`.
- **Transcript in the result.** `Outcome.session: Option<SessionSummary>` holds the first
  `TRANSCRIPT_FRAME_CAP` (1,000) frames, the total frame count, the close code and reason, and an
  error if the session ended abnormally after opening. The upgrade response fills `Outcome.exchange`
  (status 101), so `http expect`, history status and the status line work unchanged.
- **Lifecycle.** A session run ends with `Done` when either side closes. A failure before the
  upgrade completes, a rejected upgrade (any non-101 status), and cancellation end with `Failed`
  (phases `dns`, `connect`, `tls`, `upgrade`, `cancelled`). An abnormal end after the upgrade is
  `Done` with `close_code` 1006 and `error`, so the transcript is never lost. Disconnect is a
  graceful close (`SessionCmd::Close`); Esc/Cancel is `cancel`.
- **Conformance suite unchanged.** It already only assumes `fast` ends by itself with `Done`,
  `slow` is cancellable, and the exchange status matches. The ws fixture is a server that sends one
  frame and closes, and a server that accepts TCP but never answers the upgrade.
- **Shared network code is copied, not shared.** Protocol crates may not depend on each other, so
  `fluxwire-ws` carries its own copy of the rustls configuration (platform verifier, extra CAs,
  per-host client certs, `verify_tls`), HTTP CONNECT proxy handling, and the auth subset it supports
  (none, basic, bearer, apikey). Digest and OAuth2 are rejected with a clear diagnostic (P103).
  Moving these helpers into a neutral crate is deferred until a third protocol needs them.
- **Session log in the UI.** `fluxwire-ui/src/session_log.rs` is protocol-neutral state (direction
  and text filters, 50,000-frame cap with a dropped counter, selection); SSE, MCP notifications
  and gRPC streaming reuse it.

## Consequences

- Scripts for sessions (`fw.ws.onMessage`, `fw.ws.send`, SPEC §10) are not part of M8; the
  `SessionHandle` is the seam they will use.
- History stores the handshake and up to 1,000 frames (16 KiB per frame, 1 MiB per transcript) under
  `payload.session`, outside the store's `body` cap.
