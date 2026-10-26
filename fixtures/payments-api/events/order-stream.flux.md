---
name: Order stream
---

# Order stream

Live order events.

```ws
GET wss://{{host}}/v1/orders/stream
Authorization: Bearer {{accessToken}}
```

```json message subscribe
{ "op": "subscribe", "channel": "orders" }
```
