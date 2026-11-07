---
title: WebSockets
description: Connect to a WebSocket, send saved messages, and read the frame log.
---

A WebSocket request is a `.flux.md` file with a `ws` block holding the upgrade request, plus any number of saved
`message` blocks.

````markdown
---
name: Order stream
---

```ws
GET wss://{{host}}/v1/orders/stream
Authorization: Bearer {{accessToken}}
```

```json message subscribe
{ "op": "subscribe", "topic": "orders" }
```
````

1. Click **Connect** (or use the palette entry `websocket: connect or disconnect`). The status line shows
   `● Connected`.
2. Select a saved message and press <kbd>Cmd/Ctrl+Enter</kbd> to send it. `{{variables}}` are interpolated at send time.
3. Write ad-hoc messages in the composer. **Save as message** inserts a new `message` block and changes nothing else
   in the file.
4. The log shows sent (`↑`) and received (`↓`) frames. Click a frame to expand it; JSON is pretty-printed and binary
   is a hex dump. Filter by direction or text.
5. **Disconnect** sends a close frame. A failed connection reports the phase that failed, such as `connect` or
   `upgrade`.

Sessions are recorded in History, including the handshake and the first 1,000 frames. `fw.ws.*` script handlers and
`pre` scripts for WebSocket requests are not available in this version.
