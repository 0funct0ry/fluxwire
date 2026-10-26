---
name: Get payment
seq: 2
path:
  paymentId: "{{paymentId}}"
---

# Get payment

Fetches a payment by id.

```http
GET {{baseUrl}}/v1/payments/:paymentId
Accept: application/json
```

```http expect
HTTP 200
```
