---
name: Trade feed
requires: []
auth:
  type: bearer
  token: "{{token}}"
---

# Trade feed

Subscribes to a market and replays saved frames.

```ws
GET wss://{{host}}/v1/feed
    ?market=BTC-USD
    &depth=10
Sec-WebSocket-Protocol: v2.feed
# X-Debug: 1
```

```http expect
HTTP 101
Sec-WebSocket-Accept: {{*}}
```

```json message subscribe
{ "op": "subscribe", "market": "{{market}}" }
```

```xml message legacy
<subscribe market="BTC-USD"/>
```

```binary message blob
@file(fixtures/frame.bin)
```

```ts post
fw.ws.onMessage((m) => console.log(m));
```
