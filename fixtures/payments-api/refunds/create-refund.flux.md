---
name: Create refund
---

# Create refund


Refunds a captured payment.

```http
POST {{baseUrl}}/v1/refunds
Content-Type: application/json

{ 
  "payment": "{{paymentId}}", 
  "amount": 4200 
}
```

```http example
HTTP 404
Content-Type: application/json; charset=utf-8

{
  "error": {
    "type": "not_found",
    "code": "route_not_found",
    "message": "No route matches POST /v1/refunds",
    "param": null,
    "request_id": "17ccebd5-a19d-4650-b7a2-6557e27847cc"
  }
}
```
