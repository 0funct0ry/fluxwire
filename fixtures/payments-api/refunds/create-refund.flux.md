---
name: Create refund
---

# Create refund

Refunds a captured payment.

```http
POST {{baseUrl}}/v1/refunds
Content-Type: application/json

{ "payment": "{{paymentId}}", "amount": 4200 }
```
