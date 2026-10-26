---
name: List payments
seq: 3
---

# List payments

Lists recent payments, newest first.

```http
GET {{baseUrl}}/v1/payments
    ?limit=20
    # &status=failed
Accept: application/json
```
