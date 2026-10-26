---
requires: [grpc]
name: Future gRPC
x-owner: payments-team
retry:
  count: 3
---

# Future gRPC

```grpc
payments.v1.Payments/CreatePayment {{grpcHost}}
authorization: Bearer {{accessToken}}
```

```json message
{ "amount": 4200 }
```

```json schema
{ "type": "object" }
```

```yaml x-acme-lint
rules: [a, b]
```
