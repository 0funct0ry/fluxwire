---
name: Order stream
---

# Order stream

```ws
GET wss://{{host}}/v1/orders/stream
Authorization: Bearer {{accessToken}}
```

```json message subscribe
{ "op": "subscribe", "channel": "orders" }
```

```text message ping
ping
```
